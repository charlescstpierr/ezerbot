//! Linux counterpart of `keychain.rs`: generic secrets in the desktop Secret Service
//! (GNOME Keyring, KWallet), one item per name, scoped to the canonical workspace path.
//! There is no per-binary ACL here, so the macOS signature gate has no equivalent. The
//! service is never unlocked. A refused or unanswered write fails closed and is not copied
//! into the plaintext companion file. A read it refuses leaves out what only it holds, so
//! redaction refuses too (`Vault::is_partial`): nothing derived from history is served.
use super::*;
use grokbot_linux_desktop::{secrets::Collection, Bus};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

#[derive(Clone)]
pub(super) struct Target(Bus);
impl Target {
    /// Where this vault's secrets live is a fact on disk, not a choice of this process. Once the
    /// keyring holds some, the vault reads through it or not at all: without a bus it is
    /// unavailable, never a complete file vault. `app` asks for the keyring otherwise.
    pub(super) fn for_vault(vault_path: &Path, app: bool) -> Option<Self> {
        if holds_secrets(vault_path).exists() {
            return Some(Self(Bus::for_app().unwrap_or(Bus::Refused)));
        }
        app.then(Bus::for_app).flatten().map(Self)
    }
    fn connect(&self, path: &Path) -> Result<Native> {
        let canonical = fs::canonicalize(path)?;
        let scope = format!(
            "ca.ezerbot.secrets.v1.{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        );
        let collection = Collection::open(&self.0, scope).map_err(|_| unavailable())?;
        Ok(Native { collection })
    }
}
fn unavailable() -> Error {
    invalid("Le service de secrets du bureau est indisponible ou verrouillé.")
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
        StorageStatus { backend: "file_fallback", label: "Service de secrets indisponible", notice_kind: Some("warning"), notice: Some("Le service de secrets du bureau est indisponible, verrouillé ou a refusé l’accès. Les nouveaux secrets ne sont pas enregistrés. Les secrets déjà présents dans le service restent en place. Déverrouillez-le, puis cliquez sur « Vérifier le stockage ».") }
    } else {
        StorageStatus {
            backend: "keychain",
            label: "Service de secrets du bureau",
            notice: None,
            notice_kind: None,
        }
    }
}
/// Written before the first secret reaches the service, removed once a full read finds none.
/// Without it the keyring holds nothing of this vault, so a keyring locked or absent hides
/// nothing and the file is the whole vault.
fn holds_secrets(vault_path: &Path) -> PathBuf {
    vault_path.with_extension("keyring")
}
fn note_secrets_in_service(vault: &Vault) -> Result<()> {
    let marker = holds_secrets(&vault.path);
    if !marker.exists() {
        // Durable before any native write: a power cut must not keep a secret in the
        // keyring and lose the marker, or a locked restart would mask with a partial list.
        let temporary = marker.with_extension("keyring.tmp");
        let file = fs::File::create(&temporary)?;
        file.sync_all()?;
        fs::rename(&temporary, &marker)?;
        fs::File::open(marker.parent().unwrap())?.sync_all()?;
        #[cfg(test)]
        tests::STEPS.with(|steps| steps.borrow_mut().push("marqueur durable".into()));
    }
    Ok(())
}
pub(super) fn is_partial(path: &Path) -> bool {
    states().get(path).is_some_and(|s| s.failed) && holds_secrets(path).exists()
}
impl Vault {
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn open_test_keychain(path: &Path, bus_address: &str) -> Result<Self> {
        let mut vault = Self::create(path)?;
        vault.keychain = Some(Target(Bus::private(bus_address)));
        vault.readable_values()?; // First-start migration, as `open_app` does.
        Ok(vault)
    }
}

trait Backend {
    fn read_all(&self) -> Result<Secrets>;
    fn put_verified(&self, name: &str, secret: &StoredSecret) -> Result<()>;
}
struct Native {
    collection: Collection,
}
impl Backend for Native {
    fn read_all(&self) -> Result<Secrets> {
        let mut secrets = Secrets::new();
        for (name, bytes) in self.collection.entries().map_err(|_| unavailable())? {
            let secret: StoredSecret = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
            validate_stored(&name, &secret).map_err(|_| unavailable())?;
            secrets.insert(name, secret);
        }
        Ok(secrets)
    }
    fn put_verified(&self, name: &str, secret: &StoredSecret) -> Result<()> {
        let bytes = serde_json::to_vec(secret)?;
        self.collection
            .put_verified(name, &format!("Ezerbot — {name}"), &bytes)
            .map_err(|_| unavailable())
    }
}

// Caller holds both the process mutex and stable companion-file OS lock. If a
// process dies after verification but before the rename, that entry is replayed
// idempotently on restart. File entries always override older store entries.
fn migrate(vault: &Vault, backend: &impl Backend, state: &mut State) -> Result<Secrets> {
    let mut file = vault.read_file()?;
    let mut merged = backend.read_all()?;
    merged.extend(file.clone());
    if merged.is_empty() {
        fs::remove_file(holds_secrets(&vault.path)).or_else(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(error),
        })?;
    } else {
        note_secrets_in_service(vault)?;
    }
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
    if target
        .connect(&vault.path)
        .and_then(|backend| {
            note_secrets_in_service(vault)?;
            backend.put_verified(name, secret)
        })
        .is_err()
    {
        state.failed = true;
        return Err(unavailable());
    }
    file.remove(name);
    vault.write(&file)?;
    state.failed = false;
    state.cached.insert(name.into(), secret.clone());
    Ok(())
}
pub(super) fn remove(vault: &Vault, target: &Target, name: &str) -> Result<()> {
    let mut states = states();
    let state = states.entry(vault.path.clone()).or_default();
    // Never report a deletion that might reappear when the service unlocks.
    if target
        .connect(&vault.path)
        .and_then(|backend| {
            backend
                .collection
                .delete_verified(name)
                .map_err(|_| unavailable())
        })
        .is_err()
    {
        state.failed = true;
        return Err(invalid(
            "Suppression non confirmée : déverrouillez le service de secrets puis réessayez.",
        ));
    }
    let mut file = vault.read_file()?;
    file.remove(name);
    vault.write(&file)?;
    state.cached.remove(name);
    // The last secret gone: a keyring locked from now on hides nothing.
    let _ = target
        .connect(&vault.path)
        .and_then(|backend| migrate(vault, &backend, state));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    std::thread_local! {
        pub(super) static STEPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
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
            STEPS.with(|steps| steps.borrow_mut().push(format!("trousseau:{name}")));
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
        let _guard = access(test.vault.path());
        let _lock = test.vault.lock_file(true).unwrap();
        assert!(migrate(&test.vault, &fake, &mut state).is_err());
        assert!(!test.vault.read_file().unwrap().contains_key("FIRST"));
        assert!(test.vault.read_file().unwrap().contains_key("SECOND"));
        // Restart has no in-memory cache; both verified and unverified entries survive.
        let all = migrate(&test.vault, &fake, &mut State::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert!(test.vault.read_file().unwrap().is_empty());
    }
    #[test]
    fn a_link_planted_beside_the_vault_never_gets_a_file_truncated() {
        let test = test_support::TestVault::new();
        test.vault.set("FIRST", "premier", "private-first").unwrap();
        let marker = holds_secrets(test.vault.path());
        let victim = marker.with_file_name("victime");
        fs::write(&victim, "contenu d'un autre compte").unwrap();
        for planted in [marker.with_extension("keyring.tmp"), marker.clone()] {
            std::os::unix::fs::symlink(&victim, &planted).unwrap();
            let fake = Fake {
                secrets: RefCell::default(),
                writes: Cell::new(0),
                fail_at: 0,
            };
            let _guard = access(test.vault.path());
            let _lock = test.vault.lock_file(true).unwrap();
            let _ = migrate(&test.vault, &fake, &mut State::default());
            assert_eq!(
                fs::read_to_string(&victim).unwrap(),
                "contenu d'un autre compte",
                "{} was followed",
                planted.display()
            );
            fs::remove_file(&planted).unwrap();
        }
    }
    #[test]
    fn the_marker_is_durable_on_disk_before_the_first_keyring_write() {
        let test = test_support::TestVault::new();
        test.vault.set("FIRST", "premier", "private-first").unwrap();
        let marker = holds_secrets(test.vault.path());
        assert!(!marker.exists());
        let fake = Fake {
            secrets: RefCell::default(),
            writes: Cell::new(0),
            fail_at: 0,
        };
        STEPS.with(|steps| steps.borrow_mut().clear());
        let _guard = access(test.vault.path());
        let _lock = test.vault.lock_file(true).unwrap();
        migrate(&test.vault, &fake, &mut State::default()).unwrap();
        let steps = STEPS.with(|steps| steps.borrow().clone());
        assert_eq!(steps, ["marqueur durable", "trousseau:FIRST"]);
        assert!(marker.exists());
        assert!(!marker.with_extension("keyring.tmp").exists());
    }
}
