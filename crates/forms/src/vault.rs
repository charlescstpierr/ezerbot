//! Local Q1 default: a separate, owner-only file, never a SQLite table.
//! PS08 in `reports/goal-permissions-secrets.md` (A36/A45) establishes name/value
//! validation and metadata-only DTOs. Its bot form trims inputs and uppercases
//! names. This file-backed storage is a reconstruction choice, not the PS09
//! encrypted Grok account vault. The wrapper supplies a path beside its database.
use grokbot_engine::contracts::{Error, Result, SecretRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VAULT_FILE_MODE: u32 = 0o600;
pub const MAX_SECRET_NAME_CHARACTERS: usize = 128;
pub const MIN_SECRET_VALUE_CHARACTERS: usize = 8;
pub const MAX_SECRET_VALUE_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub name: String,
    pub purpose: String,
    pub updated_at: i64,
}

// Only this private disk representation serializes values. Neither Vault nor
// any public DTO implements Serialize/Debug for secret-bearing state.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSecret {
    purpose: String,
    updated_at: i64,
    value: String,
}
type Secrets = BTreeMap<String, StoredSecret>;

// The application is one local process. Serialize read/modify/replace across
// *all* Vault handles; no stale in-memory cache may clobber a concurrent update.
static VAULT_ACCESS: Mutex<()> = Mutex::new(());
fn access() -> Result<MutexGuard<'static, ()>> {
    VAULT_ACCESS
        .lock()
        .map_err(|_| invalid("secret vault lock is unavailable"))
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.to_owned())
}

/// Deliberately has no Serialize or Debug implementation.
pub struct Vault {
    path: PathBuf,
}
impl Vault {
    /// Open an explicit vault-file path (not a database path). Existing corrupt
    /// or missing data is never silently replaced after initialization.
    pub fn open(path: &Path) -> Result<Self> {
        let _guard = access()?;
        let path = std::env::current_dir()?.join(path);
        if path.file_name().is_none() {
            return Err(invalid("secret vault requires a file path"));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        match secure_options()
            .and_then(|mut options| options.write(true).create_new(true).open(&path))
        {
            Ok(mut file) => {
                owner_only(&file)?;
                file.write_all(b"{}")?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let vault = Self { path };
        vault.read()?;
        Ok(vault)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn set(&self, name: &str, purpose: &str, value: &str) -> Result<SecretRef> {
        let name = normalized_name(name)?;
        let value = value.trim();
        validate_value(value)?;
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .ok_or_else(|| invalid("secret vault clock is invalid"))?;
        let _guard = access()?;
        let mut secrets = self.read()?;
        secrets.insert(
            name.clone(),
            StoredSecret {
                purpose: purpose.trim().to_owned(),
                updated_at,
                value: value.to_owned(),
            },
        );
        self.write(&secrets)?;
        Ok(SecretRef { name })
    }
    /// For trusted tool consumers only; never return this through state/export/model APIs.
    pub fn get(&self, name: &str) -> Result<Option<String>> {
        let name = normalized_name(name)?;
        let _guard = access()?;
        Ok(self.read()?.remove(&name).map(|secret| secret.value))
    }
    pub fn list(&self) -> Result<Vec<SecretMetadata>> {
        let _guard = access()?;
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
        let name = normalized_name(name)?;
        let _guard = access()?;
        let mut secrets = self.read()?;
        if secrets.remove(&name).is_some() {
            self.write(&secrets)?;
        }
        Ok(())
    }
    /// Restricted to redaction, not model context construction.
    pub(crate) fn known_values(&self) -> Result<Vec<String>> {
        let _guard = access()?;
        Ok(self
            .read()?
            .into_values()
            .map(|secret| secret.value)
            .collect())
    }

    // Call only while holding VAULT_ACCESS. Parse errors must not echo input:
    // serde diagnostics can otherwise include a secret string from a bad file.
    fn read(&self) -> Result<Secrets> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.file_type().is_file() {
            return Err(invalid(
                "secret vault must be a regular file, not a symlink",
            ));
        }
        let file = secure_options()?.read(true).open(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let opened = file.metadata()?;
            if metadata.dev() != opened.dev()
                || metadata.ino() != opened.ino()
                || opened.nlink() != 1
            {
                return Err(invalid("secret vault file changed or has hard links"));
            }
            file.set_permissions(fs::Permissions::from_mode(VAULT_FILE_MODE))?;
        }
        let mut bytes = Vec::new();
        (&file).read_to_end(&mut bytes)?;
        let secrets: Secrets =
            serde_json::from_slice(&bytes).map_err(|_| invalid("secret vault data is invalid"))?;
        for (name, secret) in &secrets {
            if normalized_name(name)? != *name {
                return Err(invalid("secret vault name is not canonical"));
            }
            validate_value(&secret.value)?;
        }
        Ok(secrets)
    }
    fn write(&self, secrets: &Secrets) -> Result<()> {
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
            serde_json::to_writer(&mut file, secrets)?;
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

fn normalized_name(name: &str) -> Result<String> {
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
fn validate_value(value: &str) -> Result<()> {
    // Match the shipped JS validator's .length (UTF-16 code units), while its
    // upper bound uses TextEncoder, i.e. UTF-8 bytes, not character count.
    if value.encode_utf16().count() < MIN_SECRET_VALUE_CHARACTERS
        || value.len() > MAX_SECRET_VALUE_BYTES
    {
        return Err(invalid(
            "secret value must have at least 8 characters and at most 4096 UTF-8 bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    pub(crate) struct TestVault {
        directory: PathBuf,
        pub(crate) vault: Vault,
    }
    impl TestVault {
        pub(crate) fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "grokbot-vault-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let vault = Vault::open(&directory.join("workspace.secrets.json")).unwrap();
            Self { directory, vault }
        }
    }
    impl Drop for TestVault {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
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
            1
        );
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
        vault.set("TOKEN", "", &"é".repeat(2048)).unwrap();
        assert!(vault.set("TOKEN", "", &"é".repeat(2049)).is_err());
        vault.set("TOKEN", "", "🔒🔒🔒🔒").unwrap();
        assert!(vault.set("TOKEN", "", "🔒🔒🔒").is_err());
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
