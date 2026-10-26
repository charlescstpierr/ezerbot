//! Owner-only legacy file and macOS Keychain, never a SQLite secrets table.
//! The executable enables the OS backend; library fixtures remain explicitly file-backed.
//! PS08 in `reports/goal-permissions-secrets.md` (A36/A45) establishes name/value
//! validation and metadata-only DTOs. Its bot form trims inputs and uppercases
//! names. The legacy file is a reconstruction choice; PS09 motivates OS-protected
//! storage. The wrapper supplies the companion path used for migration and fallback.
use grokbot_engine::contracts::{Error, Result, SecretRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VAULT_FILE_MODE: u32 = 0o600;
pub const MAX_SECRET_NAME_CHARACTERS: usize = 128;
pub const MAX_SECRET_PURPOSE_CHARACTERS: usize = 500;
pub const MIN_SECRET_VALUE_CHARACTERS: usize = 8;
/// Site form fields shorter than this never join GLOBAL masking: a value like
/// "a" or "42" would otherwise mask JSON keys, IDs and prose everywhere.
/// Kept strictly longer than redact::MASK.
pub const MIN_SITE_MASK_CHARACTERS: usize = 6;
pub const MAX_SECRET_VALUE_BYTES: usize = 4096;
/// A connector's envelope holds the access and the refresh token of one sign-in together:
/// Microsoft's pair alone may pass 4 KB. Only the host writes these envelopes.
pub const MAX_MCP_ENV_BYTES: usize = 16_384;
/// Opaque envelope used by the local connector host; never a public projection.
pub const MCP_ENV_PURPOSE: &str = "Variables privées d’un connecteur MCP";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub name: String,
    pub purpose: String,
    pub updated_at: i64,
}

/// Public bot-panel row. The value is never part of this type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotSecretMetadata {
    pub name: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
}

// Only this private disk representation serializes values. Neither Vault nor
// any public DTO implements Serialize/Debug for secret-bearing state.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSecret {
    purpose: String,
    updated_at: i64,
    value: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_name: Option<String>,
}
type Secrets = BTreeMap<String, StoredSecret>;
mod privacy;
pub use privacy::{privacy_fingerprints, PrivacyFingerprint, PrivacySnapshot};

// Serialize threads on one vault before taking its stable companion-file lock,
// which also protects against other wrapper processes and against another
// spelling of the same file. One lock per vault file: a write that waits for the
// disk must not hold up the readers of another vault (a state read of another
// workspace, another test fixture). A path is in VAULT_ACCESS only while a
// thread holds it, so the table never outgrows the vaults in use. Releasing runs
// in Drop, also while a panic unwinds; the on-disk snapshot is revalidated next.
static VAULT_ACCESS: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
static VAULT_RELEASED: Condvar = Condvar::new();
struct Access(PathBuf);
impl Drop for Access {
    fn drop(&mut self) {
        held_vaults().remove(&self.0);
        VAULT_RELEASED.notify_all();
    }
}
fn held_vaults() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    VAULT_ACCESS.lock().unwrap_or_else(PoisonError::into_inner)
}
fn access(path: &Path) -> Access {
    let mut held = held_vaults();
    while held.contains(path) {
        held = VAULT_RELEASED
            .wait(held)
            .unwrap_or_else(PoisonError::into_inner);
    }
    held.insert(path.to_path_buf());
    Access(path.to_path_buf())
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.to_owned())
}

/// Deliberately has no Serialize or Debug implementation.
pub struct Vault {
    path: PathBuf,
    #[cfg(target_os = "macos")]
    keychain: Option<keychain::Target>,
    signature_notice: Option<&'static str>,
    /// Test double of an active keychain that refuses mutations. Production
    /// hosts leave this false; macOS uses `keychain` instead.
    #[cfg(any(test, debug_assertions))]
    refuse_keychain: bool,
}
#[cfg(target_os = "macos")]
mod keychain;
std::thread_local! {
    static READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
/// Reads of a vault's contents on this thread, file or keychain alike: what a test counts to
/// show that one rendering reads the vault once (#328).
pub fn reads_on_this_thread() -> u64 {
    READS.get()
}
// 0: explicit file-only library fixture; 1: host lacks a stable signer; 2: admitted.
static OS_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Enable the native backend in the app host, not in file-only library fixtures.
pub fn enable_os_vault(stable_signature: bool) {
    OS_MODE.store(
        if stable_signature { 2 } else { 1 },
        std::sync::atomic::Ordering::Relaxed,
    );
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageStatus {
    pub backend: &'static str,
    pub label: &'static str,
    pub notice: Option<&'static str>,
    pub notice_kind: Option<&'static str>,
}
impl Vault {
    /// Open an explicit vault-file path (not a database path). Initialization is
    /// atomic. Recover a zero-length file left by an interrupted older initializer;
    /// refuse all other corrupt content. Missing data after open fails closed.
    pub fn open(path: &Path) -> Result<Self> {
        let vault = Self::create(path)?;
        vault.known_values()?;
        Ok(vault)
    }
    /// `open` without its read of the secrets, which the caller does next and fails closed on.
    fn create(path: &Path) -> Result<Self> {
        let path = std::env::current_dir()?.join(path);
        if path.file_name().is_none() {
            return Err(invalid("secret vault requires a file path"));
        }
        let guard = access(&path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let vault = Self {
            path,
            #[cfg(target_os = "macos")]
            keychain: None,
            signature_notice: None,
            #[cfg(any(test, debug_assertions))]
            refuse_keychain: false,
        };
        let _file_guard = vault.lock_file(true)?;
        // The companion lock keeps every other opener of this vault out, so the
        // thread mutex is released before the scan of the parent directory, whose
        // cost grows with the directory.
        drop(guard);
        vault.cleanup_temporaries()?;
        let (initialize, complete) = match checked_file(&vault.path, false) {
            Ok(file) => (file.metadata()?.len() == 0, false),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => (true, true),
            Err(error) => return Err(error),
        };
        if initialize {
            let secrets = Secrets::new();
            vault.write_document(&privacy::Document {
                ledger: Some(privacy::Ledger::new(&vault.path, &secrets, complete)?),
                secrets,
            })?;
        } else {
            vault.reconcile_privacy()?;
        }
        Ok(vault)
    }
    /// Application entry point. macOS release hosts prefer Keychain; development
    /// uses only an explicit throwaway override, otherwise the file vault.
    /// The empty legacy file remains a recovery container. ChatGPT credentials
    /// are never opened by this code.
    pub fn open_app(path: &Path) -> Result<Self> {
        Ok(Self::open_app_known(path)?.0)
    }
    /// `open_app` and what its one read found, keychain migration included: redaction masks with
    /// it instead of reading the vault again (`redact::app_patterns`, #328).
    pub(crate) fn open_app_known(path: &Path) -> Result<(Self, Vec<String>)> {
        let vault = Self::create(path)?;
        #[cfg(target_os = "macos")]
        let mut vault = vault;
        #[cfg(target_os = "macos")]
        match OS_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            1 => vault.signature_notice = Some(keychain::signature_notice(&vault.path)),
            2 => vault.keychain = keychain::Target::for_app(),
            _ => (),
        }
        let values = vault.known_values()?;
        Ok((vault, values))
    }
    /// Simulates keychain refusal without falling back to plaintext storage.
    #[cfg(any(test, debug_assertions))]
    #[doc(hidden)]
    pub fn open_refusing_keychain(path: &Path) -> Result<Self> {
        let mut vault = Self::open(path)?;
        vault.refuse_keychain = true;
        Ok(vault)
    }
    pub fn storage_status(&self) -> StorageStatus {
        #[cfg(target_os = "macos")]
        if self.keychain.is_some() {
            return keychain::status(&self.path);
        }
        StorageStatus {
            backend: "file",
            label: "Fichier local protégé (lecture réservée à votre compte)",
            notice: self.signature_notice,
            notice_kind: self.signature_notice.map(|_| "info"),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Another handle on the same store, to read it again later (`redact::hold`).
    pub(crate) fn handle(&self) -> Self {
        Self {
            path: self.path.clone(),
            #[cfg(target_os = "macos")]
            keychain: self.keychain.clone(),
            signature_notice: self.signature_notice,
            #[cfg(any(test, debug_assertions))]
            refuse_keychain: self.refuse_keychain,
        }
    }
    /// Whether both handles read the same secrets: same file, same backend.
    pub(crate) fn same_store(&self, other: &Self) -> bool {
        #[cfg(target_os = "macos")]
        if self.keychain.is_some() != other.keychain.is_some() {
            return false;
        }
        self.path == other.path
    }
    pub fn set(&self, name: &str, purpose: &str, value: &str) -> Result<SecretRef> {
        let name = canonical_secret_name(name)?;
        let purpose = purpose.trim();
        validate_purpose(purpose)?;
        let value = value.trim();
        validate_value(value, purpose)?;
        self.write_secret(
            name.clone(),
            StoredSecret {
                purpose: purpose.to_owned(),
                updated_at: timestamp()?,
                value: value.to_owned(),
                created_at: 0,
                bot_id: None,
                public_name: None,
            },
        )?;
        Ok(SecretRef { name })
    }
    /// Store a bot-owned secret. The same write path reaches the macOS keychain
    /// when that backend is active. Metadata readers never receive `value`.
    pub fn set_for_bot(
        &self,
        bot_id: &str,
        name: &str,
        purpose: &str,
        value: &str,
    ) -> Result<SecretRef> {
        let name = canonical_secret_name(name)?;
        let key = storage_key(bot_id, &name)?;
        let purpose = purpose.trim();
        validate_purpose(purpose)?;
        let value = value.trim();
        validate_value(value, purpose)?;
        self.write_secret(
            key,
            StoredSecret {
                purpose: purpose.to_owned(),
                updated_at: timestamp()?,
                value: value.to_owned(),
                created_at: 0,
                bot_id: Some(bot_id.to_owned()),
                public_name: Some(name.clone()),
            },
        )?;
        Ok(SecretRef { name })
    }
    /// Unscoped storage key only. A bot secret is never returned here, even when
    /// its public name matches. Call `get_for_bot` for that value.
    pub fn get(&self, name: &str) -> Result<Option<String>> {
        let name = canonical_secret_name(name)?;
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        Ok(self.read()?.remove(&name).map(|secret| secret.value))
    }
    /// Bot-owned value at `bot:{bot_id}:{NAME}`. This is the only read of that key.
    pub fn get_for_bot(&self, bot_id: &str, name: &str) -> Result<Option<String>> {
        let key = storage_key(bot_id, name)?;
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        Ok(self.read()?.remove(&key).map(|secret| secret.value))
    }
    pub fn list(&self) -> Result<Vec<SecretMetadata>> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        Ok(self
            .read()?
            .into_iter()
            .map(|(name, secret)| SecretMetadata {
                name,
                purpose: secret.purpose,
                updated_at: secret.updated_at,
            })
            .collect())
    }
    pub fn remove(&self, name: &str) -> Result<()> {
        let name = canonical_secret_name(name)?;
        self.remove_exact(&name)
    }
    pub fn remove_for_bot(&self, bot_id: &str, name: &str) -> Result<()> {
        let key = storage_key(bot_id, name)?;
        self.remove_exact(&key)
    }
    /// Metadata grouped by bot. Values stay in the vault.
    pub fn bot_secrets(&self) -> Result<BTreeMap<String, Vec<BotSecretMetadata>>> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        let mut grouped: BTreeMap<String, Vec<BotSecretMetadata>> = BTreeMap::new();
        for secret in self.read()?.into_values() {
            let Some(bot_id) = secret.bot_id else {
                continue;
            };
            let Some(name) = secret.public_name else {
                continue;
            };
            grouped.entry(bot_id).or_default().push(BotSecretMetadata {
                name,
                description: secret.purpose,
                created_at: if secret.created_at > 0 {
                    secret.created_at
                } else {
                    secret.updated_at
                },
                updated_at: secret.updated_at,
            });
        }
        for secrets in grouped.values_mut() {
            secrets.sort_by(|left, right| left.name.cmp(&right.name));
        }
        Ok(grouped)
    }
    /// Every stored value, including bot-owned ones, for redaction only.
    pub fn values_for_redaction(&self) -> Result<Vec<String>> {
        self.known_values()
    }
    /// Restricted to redaction, not model context construction.
    pub(crate) fn known_values(&self) -> Result<Vec<String>> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        Ok(values_from_secrets(self.read()?))
    }

    // Call only while holding this vault's VAULT_ACCESS entry and its companion-file lock. Parse
    // errors must not echo input: serde diagnostics can contain secret strings.
    fn read(&self) -> Result<Secrets> {
        self.reconcile_privacy()?;
        READS.set(READS.get() + 1);
        #[cfg(target_os = "macos")]
        if let Some(target) = &self.keychain {
            return keychain::snapshot(self, target);
        }
        self.read_file()
    }
    fn read_strict(&self) -> Result<Secrets> {
        READS.set(READS.get() + 1);
        #[cfg(target_os = "macos")]
        if let Some(target) = &self.keychain {
            return keychain::strict_snapshot(self, target);
        }
        self.read_file()
    }
    fn read_file(&self) -> Result<Secrets> {
        Ok(self.read_document()?.secrets)
    }
    fn read_document(&self) -> Result<privacy::Document> {
        let file = checked_file(&self.path, false)?;
        let mut bytes = Vec::new();
        (&file).read_to_end(&mut bytes)?;
        let document: privacy::Document =
            serde_json::from_slice(&bytes).map_err(|_| invalid("secret vault data is invalid"))?;
        for (name, secret) in &document.secrets {
            validate_stored(name, secret)?;
        }
        if let Some(ledger) = &document.ledger {
            ledger.validate()?;
        }
        Ok(document)
    }
    fn write_secret(&self, key: String, mut secret: StoredSecret) -> Result<()> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        #[cfg(any(test, debug_assertions))]
        if self.refuse_keychain {
            return Err(invalid(
                "Le trousseau macOS est indisponible ou son accès a été refusé.",
            ));
        }
        self.reconcile_privacy()?;
        let mut secrets = self.read_strict()?;
        self.capture_privacy(&secrets, false)?;
        let mut incoming = Secrets::new();
        incoming.insert(key.clone(), secret.clone());
        self.capture_privacy(&incoming, true)?;
        if secret.created_at == 0 {
            secret.created_at = secrets
                .get(&key)
                .map(|existing| existing.created_at)
                .filter(|created| *created > 0)
                .unwrap_or(secret.updated_at);
        }
        secrets.insert(key.clone(), secret);
        self.persist(&secrets, &key)
    }
    fn persist(&self, secrets: &Secrets, key: &str) -> Result<()> {
        #[cfg(any(test, debug_assertions))]
        if self.refuse_keychain {
            let mut file = self.read_file()?;
            commit_keychain_secret(&mut file, key, secrets.get(key).unwrap(), false)?;
            self.write(&file)?;
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        if let Some(target) = &self.keychain {
            return keychain::set(self, target, key, secrets.get(key).unwrap());
        }
        #[cfg(not(any(test, debug_assertions, target_os = "macos")))]
        let _ = key;
        self.write(secrets)
    }
    /// Always ask the keychain when one is active, even if the snapshot has no
    /// row. A refusal is an error. Absence from a file vault is idempotent.
    /// The key is exact: a public name never selects another row.
    fn remove_exact(&self, key: &str) -> Result<()> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        #[cfg(any(test, debug_assertions))]
        if self.refuse_keychain {
            return Err(invalid(
                "Suppression non confirmée : rétablissez l’accès au trousseau macOS puis réessayez.",
            ));
        }
        self.reconcile_privacy()?;
        let mut secrets = self.read_strict()?;
        self.capture_privacy(&secrets, true)?;
        #[cfg(target_os = "macos")]
        if let Some(target) = &self.keychain {
            return keychain::remove(self, target, key);
        }
        if secrets.remove(key).is_some() {
            self.write(&secrets)?;
        }
        Ok(())
    }
    // Keep this inode stable: locking the vault itself would stop protecting
    // readers/writers as soon as atomic replacement renamed a new inode over it.
    fn lock_file(&self, exclusive: bool) -> Result<File> {
        let mut name = self.path.file_name().unwrap().to_os_string();
        name.push(".lock");
        let path = self.path.with_file_name(name);
        let file = match secure_options()?
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => {
                owner_only(&file)?;
                file
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                checked_file(&path, true)?
            }
            Err(error) => return Err(error.into()),
        };
        if exclusive {
            file.lock()?;
        } else {
            file.lock_shared()?;
        }
        Ok(file) // Drop releases the OS lock, including after a panic.
    }

    fn cleanup_temporaries(&self) -> Result<()> {
        #[cfg(test)]
        test_support::pause_cleanup(&self.path);
        let parent = self.path.parent().unwrap();
        let mut prefix = self.path.file_name().unwrap().to_os_string();
        prefix.push(".");
        let prefix = prefix.as_encoded_bytes();
        let mut removed = false;
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let suffix = name
                .as_encoded_bytes()
                .strip_prefix(prefix)
                .and_then(|suffix| suffix.strip_suffix(b".tmp"));
            if suffix.is_some_and(|suffix| {
                suffix.len() == 32 && suffix.iter().all(u8::is_ascii_hexdigit)
            }) && entry.file_type()?.is_file()
            {
                fs::remove_file(entry.path())?;
                removed = true;
            }
        }
        if removed {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    fn write(&self, secrets: &Secrets) -> Result<()> {
        let mut document = self.read_document()?;
        let ledger = document
            .ledger
            .as_mut()
            .ok_or_else(|| invalid("secret vault privacy data is unavailable"))?;
        ledger.update_file(secrets)?;
        document.secrets = secrets.clone();
        self.write_document(&document)
    }
    fn write_document(&self, document: &privacy::Document) -> Result<()> {
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|_| invalid("secret vault randomness is unavailable"))?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut name = self.path.file_name().unwrap().to_os_string();
        name.push(format!(".{suffix}.tmp"));
        let temporary = self.path.with_file_name(name);
        let mut file = secure_options()?
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            owner_only(&file)?;
            serde_json::to_writer(&mut file, document)?;
            file.sync_all()?;
            fs::rename(&temporary, &self.path)?;
            File::open(self.path.parent().unwrap())?.sync_all()?;
            Ok(())
        })();
        // Never leave a plaintext temporary behind on an ordinary write error.
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn values_from_secrets(secrets: Secrets) -> Vec<String> {
    let mut values = Vec::new();
    for (name, secret) in secrets {
        // Connector envelopes preserve short/whitespace-sensitive values.
        // Long values also belong to the GLOBAL redaction boundary: another
        // connector or the provider must not echo a different server's key.
        // Keep the > MASK-length invariant of redact.rs. Short values are
        // masked by the connector host before persistence, never added to
        // global substring redaction (which would corrupt IDs and prose).
        if name.starts_with("MCP_") && name.ends_with("_ENV") && secret.purpose == MCP_ENV_PURPOSE {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&secret.value) {
                if let Some(environment) = value["variables"].as_object() {
                    values.extend(
                        environment
                            .values()
                            .filter_map(|v| v.as_str())
                            .filter(|v| v.encode_utf16().count() >= MIN_SECRET_VALUE_CHARACTERS)
                            .map(str::to_owned),
                    );
                }
            }
        }
        // Site envelopes stay local; each field must also be masked alone.
        if name.starts_with("SITE_") && secret.purpose == "Formulaire de site privé" {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&secret.value) {
                if let Some(fields) = value["values"].as_object() {
                    values.extend(
                        fields
                            .values()
                            .map(|v| match v {
                                serde_json::Value::String(text) => text.clone(),
                                other => other.to_string(),
                            })
                            .filter(|v| v.encode_utf16().count() >= MIN_SITE_MASK_CHARACTERS),
                    );
                }
            }
        }
        values.push(secret.value);
    }
    values
}

/// Record a verified keychain write by dropping the plaintext copy. A refusal
/// leaves the file untouched and returns an error: no plaintext fallback.
#[cfg(any(test, debug_assertions, target_os = "macos"))]
fn commit_keychain_secret(
    file: &mut Secrets,
    name: &str,
    _secret: &StoredSecret,
    accepted: bool,
) -> Result<()> {
    if !accepted {
        return Err(invalid(
            "Le trousseau macOS est indisponible ou son accès a été refusé.",
        ));
    }
    file.remove(name);
    Ok(())
}

fn checked_file(path: &Path, writable: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(invalid(
            "secret vault must be a regular file, not a symlink",
        ));
    }
    let file = secure_options()?.read(true).write(writable).open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let opened = file.metadata()?;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() || opened.nlink() != 1 {
            return Err(invalid("secret vault file changed or has hard links"));
        }
        // Avoid rewriting ctime on every state poll or redaction snapshot.
        if opened.permissions().mode() & 0o7777 != VAULT_FILE_MODE {
            owner_only(&file)?;
        }
    }
    Ok(file)
}

fn owner_only(file: &File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(VAULT_FILE_MODE))?;
    }
    Ok(())
}

fn secure_options() -> std::io::Result<OpenOptions> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options.mode(VAULT_FILE_MODE);
        Ok(options)
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secret vault requires owner-only Unix file permissions",
        ))
    }
}

/// Single source of truth for secret names. Request creators must canonicalize
/// before persisting `InputKind::Secret`, so the name matches the returned SecretRef.
/// Available to downstream callers as `grokbot_forms::vault::canonical_secret_name`.
pub fn canonical_secret_name(name: &str) -> Result<String> {
    let name = name.trim().to_uppercase();
    if name.is_empty()
        || name.len() > MAX_SECRET_NAME_CHARACTERS
        || !name.as_bytes()[0].is_ascii_uppercase()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(invalid(
            "secret name must match [A-Z][A-Z0-9_]* and be at most 128 characters",
        ));
    }
    // PS08/A45: chunk-grok-bot-secrets-vYdoKF7p.js:25-59.
    const RESERVED: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "SHELL",
        "TERM",
        "PWD",
        "DISPLAY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
    ];
    const PREFIXES: &[&str] = &["LD_", "CURSOR_", "CLOUD_AGENT_", "SAND_"];
    if RESERVED.contains(&name.as_str())
        || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || name.contains("CURSOR_SANDBOX")
    {
        return Err(invalid("secret name is reserved"));
    }
    Ok(name)
}
fn timestamp() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or_else(|| invalid("secret vault clock is invalid"))
}
fn valid_bot_id(bot_id: &str) -> bool {
    !bot_id.is_empty()
        && bot_id.len() <= 128
        && bot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
fn storage_key(bot_id: &str, name: &str) -> Result<String> {
    if !valid_bot_id(bot_id) {
        return Err(invalid("secret bot is invalid"));
    }
    let name = canonical_secret_name(name)?;
    Ok(format!("bot:{bot_id}:{name}"))
}
fn validate_stored(key: &str, secret: &StoredSecret) -> Result<()> {
    validate_purpose(&secret.purpose)?;
    validate_value(&secret.value, &secret.purpose)?;
    if secret.created_at < 0 || secret.updated_at <= 0 {
        return Err(invalid("secret vault data is invalid"));
    }
    match (&secret.bot_id, &secret.public_name) {
        (None, None) => {
            if canonical_secret_name(key)? != key {
                return Err(invalid("secret vault name is not canonical"));
            }
        }
        (Some(bot_id), Some(public_name)) => {
            if storage_key(bot_id, public_name)? != key {
                return Err(invalid("secret vault name is not canonical"));
            }
        }
        _ => return Err(invalid("secret vault data is invalid")),
    }
    Ok(())
}
fn validate_purpose(purpose: &str) -> Result<()> {
    // PS08/A36 maxLength=500, measured like the HTML input in UTF-16 units.
    if purpose.encode_utf16().count() > MAX_SECRET_PURPOSE_CHARACTERS {
        return Err(invalid("secret purpose must be at most 500 characters"));
    }
    Ok(())
}
fn validate_value(value: &str, purpose: &str) -> Result<()> {
    // Match the shipped JS validator's .length (UTF-16 code units), while its
    // upper bound uses TextEncoder, i.e. UTF-8 bytes, not character count.
    let envelope = purpose == MCP_ENV_PURPOSE;
    if value.encode_utf16().count() < MIN_SECRET_VALUE_CHARACTERS
        || value.len()
            > if envelope {
                MAX_MCP_ENV_BYTES
            } else {
                MAX_SECRET_VALUE_BYTES
            }
    {
        return Err(invalid(if envelope {
            "connector envelope must have at least 8 characters and at most 16384 UTF-8 bytes"
        } else {
            "secret value must have at least 8 characters and at most 4096 UTF-8 bytes"
        }));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    type Pause = (
        PathBuf,
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    static PAUSE: Mutex<Option<Pause>> = Mutex::new(None);
    /// Makes the cleanup scan of one vault wait until released, so a test can
    /// observe what other vaults do meanwhile. Returns the signal that the scan
    /// was entered and the way to release it.
    pub(super) fn hold_cleanup(
        path: &Path,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *PAUSE.lock().unwrap() = Some((path.to_owned(), entered_tx, release_rx));
        (entered, release)
    }
    pub(super) fn pause_cleanup(path: &Path) {
        let pause = {
            let mut slot = PAUSE.lock().unwrap();
            if slot.as_ref().is_some_and(|(held, ..)| held == path) {
                slot.take()
            } else {
                None
            }
        };
        if let Some((_, entered, release)) = pause {
            let _ = entered.send(());
            let _ = release.recv_timeout(std::time::Duration::from_secs(20));
        }
    }
    pub(crate) struct TestVault {
        pub(crate) vault: Vault,
        // Après `vault` : le coffre se ferme avant la suppression du dossier.
        _directory: grokbot_test_support::TestDir,
    }
    impl TestVault {
        pub(crate) fn new() -> Self {
            let directory = grokbot_test_support::TestDir::new("grokbot-vault");
            let vault = Vault::open(&directory.join("workspace.secrets.json")).unwrap();
            Self {
                vault,
                _directory: directory,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TestVault;
    use super::*;

    #[test]
    #[cfg(unix)]
    fn vault_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let test = TestVault::new();
        let mode = || {
            fs::metadata(test.vault.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(), VAULT_FILE_MODE);
        fs::set_permissions(test.vault.path(), fs::Permissions::from_mode(0o644)).unwrap();
        let vault = Vault::open(test.vault.path()).unwrap();
        assert_eq!(mode(), VAULT_FILE_MODE);
        vault.set("TOKEN", "test", "private-value").unwrap();
        assert_eq!(mode(), VAULT_FILE_MODE);
        vault.set("TOKEN", "updated", "another-value").unwrap();
        assert_eq!(mode(), VAULT_FILE_MODE);
        assert_eq!(
            fs::read_dir(test.vault.path().parent().unwrap())
                .unwrap()
                .count(),
            2 // vault plus its stable companion lock; no plaintext temporaries
        );
        let lock_path = test.vault.path().with_extension("json.lock");
        assert_eq!(
            fs::metadata(lock_path).unwrap().permissions().mode() & 0o7777,
            VAULT_FILE_MODE
        );
    }

    #[test]
    fn active_keychain_refusal_does_not_store_plaintext() {
        let test = TestVault::new();
        let path = test.vault.path().to_owned();
        let before = fs::read(&path).unwrap();
        let vault = Vault::open_refusing_keychain(&path).unwrap();
        let error = vault
            .set(
                "OPENAI_API_KEY",
                "Compte OpenAI pour générer des images",
                "image-key-secret-9931",
            )
            .unwrap_err();
        assert!(!error.to_string().contains("image-key-secret-9931"));
        assert!(!format!("{error:?}").contains("image-key-secret-9931"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(Vault::open(&path)
            .unwrap()
            .get("OPENAI_API_KEY")
            .unwrap()
            .is_none());
        test.vault
            .set(
                "OPENAI_API_KEY",
                "Compte OpenAI pour générer des images",
                "image-key-secret-9931",
            )
            .unwrap();
        let seeded = fs::read(&path).unwrap();
        let deletion = Vault::open_refusing_keychain(&path)
            .unwrap()
            .remove("OPENAI_API_KEY")
            .unwrap_err();
        assert!(deletion.to_string().contains("Suppression non confirmée"));
        assert!(!deletion.to_string().contains("image-key-secret-9931"));
        assert_eq!(fs::read(&path).unwrap(), seeded);
    }

    #[test]
    fn list_returns_metadata_only() {
        let test = TestVault::new();
        let reference = test
            .vault
            .set(" api_token ", " service login ", " private-value ")
            .unwrap();
        assert_eq!(reference.name, "API_TOKEN");
        assert_eq!(
            serde_json::to_value(reference).unwrap(),
            serde_json::json!({"name": "API_TOKEN"})
        );
        let metadata = test.vault.list().unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].purpose, "service login");
        assert!(metadata[0].updated_at > 0);
        let serialized = serde_json::to_string(&metadata).unwrap();
        assert!(!serialized.contains("private-value"));
        assert!(!serialized.contains("\"value\""));
        assert!(!format!("{metadata:?}").contains("private-value"));
    }

    #[test]
    fn name_validation_matches_ps08() {
        let test = TestVault::new();
        let vault = &test.vault;
        for name in [
            "",
            "_TOKEN",
            "1TOKEN",
            "A-B",
            "A B",
            "A.B",
            "ÉTOKEN",
            "A\nB",
            "PATH",
            "HOME",
            "USER",
            "SHELL",
            "TERM",
            "PWD",
            "DISPLAY",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "LD_PRELOAD",
            "CURSOR_KEY",
            "CLOUD_AGENT_TOKEN",
            "SAND_TOKEN",
            "MY_CURSOR_SANDBOX_TOKEN",
        ] {
            assert!(vault.set(name, "", "12345678").is_err(), "{name}");
        }
        assert!(vault.set(&"A".repeat(129), "", "12345678").is_err());
        for name in ["TOKEN", "A_1", "HOME_TOKEN", "LD", &"A".repeat(128)] {
            vault.set(name, "", "12345678").unwrap();
        }
        for value in ["", "1234567", "        ", " 1234567 "] {
            assert!(vault.set("TOKEN", "", value).is_err());
        }
        vault.set("TOKEN", "", &"a".repeat(4096)).unwrap();
        assert!(vault.set("TOKEN", "", &"a".repeat(4097)).is_err());
        // Only a connector's envelope may hold more: a sign-in's two tokens.
        vault
            .set("TOKEN", MCP_ENV_PURPOSE, &"a".repeat(MAX_MCP_ENV_BYTES))
            .unwrap();
        assert!(vault
            .set("TOKEN", MCP_ENV_PURPOSE, &"a".repeat(MAX_MCP_ENV_BYTES + 1))
            .is_err());
        assert!(vault.set("TOKEN", "autre", &"a".repeat(4097)).is_err());
        vault.set("TOKEN", "", &"é".repeat(2048)).unwrap();
        assert!(vault.set("TOKEN", "", &"é".repeat(2049)).is_err());
        vault.set("TOKEN", "", "🔒🔒🔒🔒").unwrap();
        assert!(vault.set("TOKEN", "", "🔒🔒🔒").is_err());
        vault.set("TOKEN", &"a".repeat(500), "12345678").unwrap();
        assert!(vault.set("TOKEN", &"a".repeat(501), "12345678").is_err());
        vault.set("TOKEN", &"🔒".repeat(250), "12345678").unwrap();
        assert!(vault.set("TOKEN", &"🔒".repeat(251), "12345678").is_err());
        vault
            .set("TOKEN", &format!(" {} ", "a".repeat(500)), "12345678")
            .unwrap();
        assert_eq!(canonical_secret_name(" api_token ").unwrap(), "API_TOKEN");
        assert!(canonical_secret_name("PATH").is_err());
    }

    #[test]
    fn unscoped_get_and_remove_ignore_bot_secrets_and_a_refused_keychain_writes_nothing() {
        let test = TestVault::new();
        test.vault
            .set("TOKEN", "unscoped", "phrase du coffre global")
            .unwrap();
        test.vault
            .set_for_bot("grokbot", "TOKEN", "bot", "phrase du coffre du bot")
            .unwrap();
        assert_eq!(
            test.vault.get("TOKEN").unwrap().as_deref(),
            Some("phrase du coffre global")
        );
        assert!(test.vault.get("MCP_SERVER_ENV").unwrap().is_none());
        assert_eq!(
            test.vault
                .get_for_bot("grokbot", "token")
                .unwrap()
                .as_deref(),
            Some("phrase du coffre du bot")
        );
        test.vault.remove("TOKEN").unwrap();
        assert!(test.vault.get("TOKEN").unwrap().is_none());
        assert_eq!(
            test.vault
                .get_for_bot("grokbot", "TOKEN")
                .unwrap()
                .as_deref(),
            Some("phrase du coffre du bot")
        );
        test.vault.remove("MCP_SERVER_ENV").unwrap();
        assert_eq!(
            test.vault
                .get_for_bot("grokbot", "TOKEN")
                .unwrap()
                .as_deref(),
            Some("phrase du coffre du bot")
        );
        test.vault.remove_for_bot("grokbot", "TOKEN").unwrap();
        assert!(test
            .vault
            .get_for_bot("grokbot", "TOKEN")
            .unwrap()
            .is_none());

        let refusing = Vault::open_refusing_keychain(test.vault.path()).unwrap();
        let created = refusing
            .set("NEW_TOKEN", "refus", "phrase refusee du coffre")
            .unwrap_err()
            .to_string();
        assert!(created.contains("trousseau"));
        assert!(!created.contains("phrase refusee"));
        let bot = refusing
            .set_for_bot("grokbot", "NEW_TOKEN", "refus", "phrase refusee du bot")
            .unwrap_err()
            .to_string();
        assert!(bot.contains("trousseau"));
        let file = fs::read_to_string(test.vault.path()).unwrap();
        assert!(!file.contains("phrase refusee"));
        assert!(refusing.get("NEW_TOKEN").unwrap().is_none());
        assert!(refusing
            .get_for_bot("grokbot", "NEW_TOKEN")
            .unwrap()
            .is_none());
        let removed = refusing.remove("ABSENT_KEY").unwrap_err().to_string();
        assert!(removed.contains("Suppression non confirmée"));
        assert!(refusing
            .remove_for_bot("grokbot", "ABSENT_KEY")
            .unwrap_err()
            .to_string()
            .contains("Suppression non confirmée"));
    }

    #[test]
    fn vault_updates_remove_and_reopen_without_stale_cache() {
        let test = TestVault::new();
        let other = Vault::open(test.vault.path()).unwrap();
        test.vault.set("FIRST", "first", "value-one").unwrap();
        other.set("SECOND", "second", "value-two").unwrap();
        assert_eq!(
            test.vault.get("second").unwrap().as_deref(),
            Some("value-two")
        );
        other.set("FIRST", "replacement", "value-new").unwrap();
        test.vault.remove("SECOND").unwrap();
        test.vault.remove("SECOND").unwrap();
        let reopened = Vault::open(test.vault.path()).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 1);
        assert_eq!(reopened.get("FIRST").unwrap().as_deref(), Some("value-new"));
        assert!(reopened.get("SECOND").unwrap().is_none());
    }

    #[test]
    fn vault_concurrent_handles_preserve_each_update() {
        let test = TestVault::new();
        std::thread::scope(|scope| {
            for index in 0..8 {
                let path = test.vault.path();
                scope.spawn(move || {
                    Vault::open(path)
                        .unwrap()
                        .set(&format!("TOKEN_{index}"), "", "test-value")
                        .unwrap();
                });
            }
        });
        assert_eq!(test.vault.list().unwrap().len(), 8);
    }

    #[test]
    fn vault_corruption_is_not_overwritten_or_echoed_in_errors() {
        let test = TestVault::new();
        let bad = b"{\"TOKEN\":\"private-value\"}";
        fs::write(test.vault.path(), bad).unwrap();
        let error = test.vault.set("TOKEN", "", "another-value").unwrap_err();
        assert!(!error.to_string().contains("private-value"));
        assert!(!format!("{error:?}").contains("private-value"));
        assert!(test.vault.list().is_err());
        assert!(Vault::open(test.vault.path()).is_err());
        assert_eq!(fs::read(test.vault.path()).unwrap(), bad);
    }

    #[test]
    fn vault_zero_length_file_from_interrupted_init_is_recovered() {
        let test = TestVault::new();
        fs::write(test.vault.path(), b"").unwrap();
        let reopened = Vault::open(test.vault.path()).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        reopened.set("TOKEN", "", "test-value").unwrap();
        assert_eq!(
            reopened.get("TOKEN").unwrap().as_deref(),
            Some("test-value")
        );
        assert_eq!(
            crate::redact::redact(&reopened, "test-value").unwrap(),
            "[...]"
        );
    }

    #[test]
    fn a_vault_scanning_its_directory_does_not_hold_up_another_vault() {
        let directory = grokbot_test_support::TestDir::new("grokbot-vault-scan");
        let slow = directory.join("slow.secrets.json");
        let (entered, release) = test_support::hold_cleanup(&slow);
        let scanning = std::thread::spawn({
            let slow = slow.clone();
            move || Vault::open(&slow).map(|_| ())
        });
        entered
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the first vault never reached its directory scan");
        let (done_tx, done) = std::sync::mpsc::channel();
        let other = std::thread::spawn(move || {
            let other = TestVault::new();
            done_tx
                .send(other.vault.list().map(|list| list.len()))
                .unwrap();
        });
        let finished = done.recv_timeout(std::time::Duration::from_secs(5));
        release.send(()).unwrap();
        scanning.join().unwrap().unwrap();
        other.join().unwrap();
        assert_eq!(
            finished
                .expect("another vault waited for a directory scan it does not share")
                .unwrap(),
            0
        );
    }

    #[test]
    fn vault_orphaned_temporaries_are_removed_on_open() {
        let test = TestVault::new();
        let orphan = test
            .vault
            .path()
            .with_extension("json.0123456789abcdef0123456789ABCDEF.tmp");
        let unrelated = test
            .vault
            .path()
            .with_extension("json.not-a-vault-temporary.tmp");
        let directory = test
            .vault
            .path()
            .with_extension("json.abcdef0123456789abcdef0123456789.tmp");
        fs::write(&orphan, b"old-secret-value").unwrap();
        fs::write(&unrelated, b"unrelated").unwrap();
        fs::create_dir(&directory).unwrap();
        let reopened = Vault::open(test.vault.path()).unwrap();
        assert!(reopened.list().unwrap().is_empty());
        assert!(!orphan.exists());
        assert!(unrelated.exists());
        assert!(directory.is_dir());
    }

    #[test]
    fn the_access_table_forgets_a_vault_after_use_and_after_a_failed_open() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "test-value").unwrap();
        assert_eq!(test.vault.list().unwrap().len(), 1);
        assert!(!held_vaults().contains(test.vault.path()));
        let corrupt = test.vault.path().with_file_name("corrompu.secrets.json");
        fs::write(&corrupt, b"{pas du json").unwrap();
        assert!(Vault::open(&corrupt).is_err());
        assert!(!held_vaults().contains(&corrupt));
    }

    #[test]
    fn vault_mutex_poison_does_not_disable_this_or_other_vaults() {
        let test = TestVault::new();
        let path = test.vault.path().to_path_buf();
        assert!(std::panic::catch_unwind(move || {
            let _guard = access(&path);
            panic!("simulated interruption under the vault mutex");
        })
        .is_err());
        test.vault.set("TOKEN", "", "test-value").unwrap();
        assert_eq!(test.vault.list().unwrap().len(), 1);
        let other = TestVault::new();
        other.vault.set("TOKEN", "", "test-value").unwrap();
        assert_eq!(other.vault.list().unwrap().len(), 1);
    }

    /// A write that waits for the disk holds its vault's mutex. Another vault's
    /// reader (a state read of another workspace) must not wait for it.
    #[test]
    fn a_held_vault_does_not_hold_up_another_vault() {
        let busy = TestVault::new();
        let other = TestVault::new();
        other.vault.set("TOKEN", "", "test-value").unwrap();
        let held = access(busy.vault.path());
        let (done, finished) = std::sync::mpsc::channel();
        let path = other.vault.path().to_path_buf();
        let reader = std::thread::spawn(move || {
            done.send(
                Vault::open(&path)
                    .and_then(|vault| vault.list())
                    .map(|l| l.len()),
            )
            .unwrap();
        });
        let listed = finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the other vault waited for the held one");
        drop(held);
        reader.join().unwrap();
        assert_eq!(listed.unwrap(), 1);
    }

    #[test]
    fn vault_read_revalidates_purpose_limit() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "test-value").unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(test.vault.path()).unwrap()).unwrap();
        value["TOKEN"]["purpose"] = serde_json::Value::String("a".repeat(501));
        fs::write(test.vault.path(), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(test.vault.list().is_err());
        assert!(test.vault.known_values().is_err());
    }

    #[test]
    fn vault_cross_process_writer_child() {
        let Some(path) = std::env::var_os("GROKBOT_VAULT_TEST_PATH") else {
            return;
        };
        let prefix = std::env::var("GROKBOT_VAULT_TEST_PREFIX").unwrap();
        let vault = Vault::open(Path::new(&path)).unwrap();
        for index in 0..40 {
            vault
                .set(&format!("TOKEN_{prefix}_{index}"), "", "child-test-value")
                .unwrap();
        }
    }

    #[test]
    fn vault_cross_process_writes_preserve_all_eighty_secrets() {
        use std::process::{Command, Stdio};
        let test = TestVault::new();
        let children: Vec<_> = ["LEFT", "RIGHT"]
            .iter()
            .map(|prefix| {
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "vault::tests::vault_cross_process_writer_child"])
                    .env("GROKBOT_VAULT_TEST_PATH", test.vault.path())
                    .env("GROKBOT_VAULT_TEST_PREFIX", prefix)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert_eq!(test.vault.list().unwrap().len(), 80);
    }

    #[test]
    #[cfg(unix)]
    fn vault_reads_do_not_rewrite_permissions() {
        use std::os::unix::fs::MetadataExt;
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "test-value").unwrap();
        let paths = [
            test.vault.path().to_owned(),
            test.vault.path().with_extension("json.lock"),
        ];
        let times = || {
            paths
                .iter()
                .map(|path| {
                    let metadata = fs::metadata(path).unwrap();
                    (metadata.ctime(), metadata.ctime_nsec())
                })
                .collect::<Vec<_>>()
        };
        let before = times();
        test.vault.get("TOKEN").unwrap();
        test.vault.list().unwrap();
        test.vault.known_values().unwrap();
        assert_eq!(before, times());
    }

    #[test]
    #[cfg(unix)]
    fn vault_refuses_symlinks_and_hard_links() {
        use std::os::unix::fs::symlink;
        let test = TestVault::new();
        let alias = test.vault.path().with_extension("alias");
        symlink(test.vault.path(), &alias).unwrap();
        assert!(Vault::open(&alias).is_err());
        fs::remove_file(&alias).unwrap();
        fs::hard_link(test.vault.path(), &alias).unwrap();
        assert!(Vault::open(&alias).is_err());
        assert!(test.vault.get("TOKEN").is_err());
    }
}
