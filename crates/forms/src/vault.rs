//! Separate 0600 file vault signatures (Q1 default). No secret value is serializable.
//! Validation follows goal-permissions-secrets.md PS08; storage is gb-forms-vault.
use grokbot_engine::contracts::{pending, Result, SecretRef};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Deliberately has no Serialize or Debug implementation.
pub struct Vault {
    path: PathBuf,
}
impl Vault {
    pub fn open(_path: &Path) -> Result<Self> {
        pending("forms.vault")
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn set(&self, _name: &str, _purpose: &str, _value: &str) -> Result<SecretRef> {
        pending("forms.vault")
    }
    /// For trusted tool consumers only; never return this through state/export/model APIs.
    pub fn get(&self, _name: &str) -> Result<Option<String>> {
        pending("forms.vault")
    }
    pub fn list(&self) -> Result<Vec<SecretMetadata>> {
        pending("forms.vault")
    }
    pub fn remove(&self, _name: &str) -> Result<()> {
        pending("forms.vault")
    }
    /// Restricted to redaction, not model context construction.
    pub(crate) fn known_values(&self) -> Result<Vec<String>> {
        pending("forms.vault")
    }
}
