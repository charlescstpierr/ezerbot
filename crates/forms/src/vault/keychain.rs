//! PS09/A47–A49 establishes OS-encrypted local persistence, not a Keychain
//! schema. Reconstruction: generic passwords, one item per name, service scoped
//! to the canonical workspace path. Native calls never contain shell arguments,
//! logs or OS error descriptions. All UI interaction is disabled (fail to file).
use super::*;
use security_framework::{
    item::{update_item, ItemClass, ItemSearchOptions, ItemUpdateOptions, Limit},
    os::macos::keychain::{KeychainUserInteractionLock, SecKeychain},
};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

#[derive(Clone)]
pub(super) enum Target {
    // The default/login route does not even compile into debug or unit tests.
    #[cfg(not(any(debug_assertions, test)))]
    Default,
    #[cfg(any(debug_assertions, test))]
    File(PathBuf),
    #[cfg(any(debug_assertions, test))]
    Unavailable,
}
impl Target {
    pub(super) fn for_app() -> Option<Self> {
        // Development must never fall through to the user's login keychain.
        // No override means an ordinary file vault; a malformed one adds a notice.
        #[cfg(any(debug_assertions, test))]
        {
            let path = std::env::var_os("GROKBOT_TEST_KEYCHAIN")?;
            let checked = (|| -> Option<PathBuf> {
                let home = fs::canonicalize(std::env::var_os("HOME")?).ok()?;
                let fixed = fs::canonicalize(std::env::var_os("CFFIXED_USER_HOME")?).ok()?;
                let path = fs::canonicalize(PathBuf::from(path)).ok()?;
                if home != fixed
                    || !home.join(".ezerbot-test-home").is_file()
                    || !path.starts_with(&home)
                    || path.file_name()?.to_str()? != "ezerbot-test.keychain-db"
                {
                    return None;
                }
                Some(path)
            })();
            Some(checked.map(Self::File).unwrap_or(Self::Unavailable))
        }
        #[cfg(not(any(debug_assertions, test)))]
        Some(Self::Default)
    }
    fn connect(&self, path: &Path) -> Result<Native> {
        let interaction = SecKeychain::disable_user_interaction().map_err(|_| unavailable())?;
        let keychain = match self {
            #[cfg(not(any(debug_assertions, test)))]
            Self::Default => SecKeychain::default(),
            #[cfg(any(debug_assertions, test))]
            Self::File(path) => SecKeychain::open(path),
            #[cfg(any(debug_assertions, test))]
            Self::Unavailable => return Err(unavailable()),
        }
        .map_err(|_| unavailable())?;
        let canonical = fs::canonicalize(path)?;
        let service = format!(
            "ca.ezerbot.secrets.v1.{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        );
        Ok(Native {
            keychain,
            service,
            _interaction: interaction,
        })
    }
}
fn unavailable() -> Error {
    invalid("Le trousseau macOS est indisponible ou son accès a été refusé.")
}
#[derive(Default)]
struct State {
    cached: Secrets,
    failed: bool,
}
static STATES: OnceLock<Mutex<BTreeMap<PathBuf, State>>> = OnceLock::new();
fn states() -> MutexGuard<'static, BTreeMap<PathBuf, State>> {
    STATES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn status(path: &Path) -> StorageStatus {
    if states().get(path).is_some_and(|s| s.failed) {
        StorageStatus { backend: "file_fallback", label: "Fichier local de secours, non chiffré", notice_kind: Some("warning"), notice: Some("Le trousseau macOS est indisponible ou son accès a été refusé. Ezerbot continue avec un fichier local lisible seulement par votre compte macOS, mais non chiffré. Les secrets déjà transférés restent dans le trousseau et peuvent être indisponibles. Déverrouillez-le ou vérifiez les autorisations d’Ezerbot dans Trousseaux d’accès, puis vérifiez le stockage pour reprendre le transfert.") }
    } else {
        StorageStatus {
            backend: "keychain",
            label: "Trousseau macOS",
            notice: None,
            notice_kind: None,
        }
    }
}
pub(super) fn signature_notice(path: &Path) -> &'static str {
    // Attributes only, restricted to our exact service and selected keychain.
    // In debug, Target::for_app never permits the user's default keychain.
    let previous = Target::for_app()
        .and_then(|target| target.connect(path).ok())
        .is_some_and(|backend| {
            backend
                .query()
                .load_attributes(true)
                .limit(1)
                .search()
                .is_ok_and(|items| !items.is_empty())
        });
    if previous {
        "Le trousseau sera activé quand l’application aura une signature stable. Aucune migration n’est effectuée : les secrets du fichier local protégé y sont conservés. Des secrets d’une version précédente sont aussi présents dans le trousseau ; ils ne sont pas modifiés. Utilisez l’ancienne version autorisée pour les récupérer."
    } else {
        "Le trousseau sera activé quand l’application aura une signature stable. Pour préserver les secrets après chaque reconstruction, aucune migration n’est effectuée : ils restent dans le fichier local protégé, lisible uniquement par votre compte et non chiffré."
    }
}
impl Vault {
    pub(super) fn with_keychain(mut self, target: Target) -> Result<Self> {
        self.keychain = Some(target);
        let _guard = access();
        let _file = self.lock_file(true)?;
        self.read()?; // First-start migration, even before the first HTTP request.
        Ok(self)
    }
    /// Test-only explicit file selection: no default keychain is ever searched.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn open_test_keychain(path: &Path, keychain: &Path) -> Result<Self> {
        Self::open(path)?.with_keychain(Target::File(keychain.to_path_buf()))
    }
}

trait Backend {
    fn read_all(&self) -> Result<Secrets>;
    fn put_verified(&self, name: &str, secret: &StoredSecret) -> Result<()>;
}
struct Native {
    keychain: SecKeychain,
    service: String,
    _interaction: KeychainUserInteractionLock,
}
impl Native {
    fn query(&self) -> ItemSearchOptions {
        let mut query = ItemSearchOptions::new();
        query
            .keychains(std::slice::from_ref(&self.keychain))
            .class(ItemClass::generic_password())
            .service(&self.service);
        query
    }
    fn delete(&self, name: &str) -> Result<()> {
        // macOS can permit metadata-only deletion even while locked. Require
        // access to the targeted item first: denial must never destroy it.
        match self.keychain.find_generic_password(&self.service, name) {
            Ok(_) => (),
            Err(e) if e.code() == -25300 => return Ok(()),
            Err(_) => return Err(unavailable()),
        }
        let mut query = self.query();
        match query.account(name).delete() {
            Ok(()) => (),
            Err(e) if e.code() == -25300 => (), // errSecItemNotFound
            Err(_) => return Err(unavailable()),
        }
        match self.keychain.find_generic_password(&self.service, name) {
            Err(e) if e.code() == -25300 => Ok(()),
            _ => Err(unavailable()),
        }
    }
}
impl Backend for Native {
    fn read_all(&self) -> Result<Secrets> {
        let mut query = self.query();
        let items = match query.load_attributes(true).limit(Limit::All).search() {
            Ok(items) => items,
            Err(e) if e.code() == -25300 => return Ok(Secrets::new()),
            Err(_) => return Err(unavailable()),
        };
        let mut secrets = Secrets::new();
        for item in items {
            let attrs = item.simplify_dict().ok_or_else(unavailable)?;
            let name = attrs.get("acct").ok_or_else(unavailable)?;
            let (bytes, _) = self
                .keychain
                .find_generic_password(&self.service, name)
                .map_err(|_| unavailable())?;
            let secret: StoredSecret =
                serde_json::from_slice(bytes.as_ref()).map_err(|_| unavailable())?;
            validate_stored(name, &secret).map_err(|_| unavailable())?;
            secrets.insert(name.clone(), secret);
        }
        Ok(secrets)
    }
    fn put_verified(&self, name: &str, secret: &StoredSecret) -> Result<()> {
        let bytes = serde_json::to_vec(secret)?;
        match self.keychain.find_generic_password(&self.service, name) {
            Ok((_, mut item)) => item.set_password(&bytes).map_err(|_| unavailable())?,
            Err(e) if e.code() == -25300 => self
                .keychain
                .add_generic_password(&self.service, name, &bytes)
                .map_err(|_| unavailable())?,
            Err(_) => return Err(unavailable()),
        }
        let mut metadata = ItemUpdateOptions::new();
        metadata
            .set_label(format!("Ezerbot — {name}"))
            .set_description("Secret Ezerbot");
        update_item(self.query().account(name), &metadata).map_err(|_| unavailable())?;
        let (readback, _) = self
            .keychain
            .find_generic_password(&self.service, name)
            .map_err(|_| unavailable())?;
        if readback.as_ref() != bytes {
            return Err(unavailable());
        }
        Ok(())
    }
}

// Caller holds both the process mutex and stable companion-file OS lock. If a
// process dies after verification but before the rename, that entry is replayed
// idempotently on restart. File entries always override older Keychain entries.
fn migrate(vault: &Vault, backend: &impl Backend, state: &mut State) -> Result<Secrets> {
    let mut file = vault.read_file()?;
    let mut merged = backend.read_all()?;
    merged.extend(file.clone());
    state.cached = merged.clone();
    for (name, secret) in &merged {
        if !file.contains_key(name) {
            continue;
        }
        backend.put_verified(name, secret)?;
        file.remove(name);
        vault.write(&file)?; // atomic rename + file and directory fsync
    }
    Ok(merged)
}
pub(super) fn snapshot(vault: &Vault, target: &Target) -> Result<Secrets> {
    let mut states = states();
    let state = states.entry(vault.path.clone()).or_default();
    match target
        .connect(&vault.path)
        .and_then(|backend| migrate(vault, &backend, state))
    {
        Ok(secrets) => {
            state.failed = false;
            Ok(secrets)
        }
        Err(_) => {
            state.failed = true;
            let mut secrets = state.cached.clone();
            secrets.extend(vault.read_file()?);
            Ok(secrets)
        }
    }
}
pub(super) fn set(vault: &Vault, target: &Target, name: &str, secret: &StoredSecret) -> Result<()> {
    let mut states = states();
    let state = states.entry(vault.path.clone()).or_default();
    let mut file = vault.read_file()?;
    match target
        .connect(&vault.path)
        .and_then(|backend| backend.put_verified(name, secret))
    {
        Ok(()) => {
            file.remove(name);
            vault.write(&file)?;
        }
        Err(_) => {
            state.failed = true;
            file.insert(name.into(), secret.clone());
            vault.write(&file)?;
        }
    }
    state.cached.insert(name.into(), secret.clone());
    Ok(())
}
pub(super) fn remove(vault: &Vault, target: &Target, name: &str) -> Result<()> {
    let mut states = states();
    let state = states.entry(vault.path.clone()).or_default();
    // Never report a deletion that might reappear when the trousseau unlocks.
    if target
        .connect(&vault.path)
        .and_then(|backend| backend.delete(name))
        .is_err()
    {
        state.failed = true;
        return Err(invalid(
            "Suppression non confirmée : rétablissez l’accès au trousseau macOS puis réessayez.",
        ));
    }
    let mut file = vault.read_file()?;
    file.remove(name);
    vault.write(&file)?;
    state.cached.remove(name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    struct Fake {
        secrets: RefCell<Secrets>,
        writes: Cell<usize>,
        fail_at: usize,
    }
    impl Backend for Fake {
        fn read_all(&self) -> Result<Secrets> {
            Ok(self.secrets.borrow().clone())
        }
        fn put_verified(&self, name: &str, secret: &StoredSecret) -> Result<()> {
            self.writes.set(self.writes.get() + 1);
            self.secrets
                .borrow_mut()
                .insert(name.into(), secret.clone());
            // Simulates a successful write but denied/failed readback.
            if self.writes.get() == self.fail_at {
                return Err(unavailable());
            }
            Ok(())
        }
    }
    #[test]
    fn migration_removes_only_verified_entries_and_replays_after_crash() {
        let test = test_support::TestVault::new();
        test.vault.set("FIRST", "premier", "private-first").unwrap();
        test.vault
            .set("SECOND", "second", "private-second")
            .unwrap();
        let fake = Fake {
            secrets: RefCell::default(),
            writes: Cell::new(0),
            fail_at: 2,
        };
        let mut state = State::default();
        let _guard = access();
        let _lock = test.vault.lock_file(true).unwrap();
        assert!(migrate(&test.vault, &fake, &mut state).is_err());
        assert!(!test.vault.read_file().unwrap().contains_key("FIRST"));
        assert!(test.vault.read_file().unwrap().contains_key("SECOND"));
        // Restart has no in-memory cache; both verified and unverified entries survive.
        let all = migrate(&test.vault, &fake, &mut State::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(test.vault.read_file().unwrap().is_empty());
    }
}
