use super::*;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

const BASES: [u64; 2] = [257, 263];

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivacyFingerprint {
    pub length: usize,
    pub digest: Vec<u8>,
    pub filter: Vec<u8>,
}

pub struct PrivacySnapshot {
    pub epoch: String,
    pub revision: u64,
    pub complete: bool,
    pub fingerprints: Vec<PrivacyFingerprint>,
    pub active: Vec<String>,
}

pub fn privacy_fingerprints(values: &[String]) -> (Vec<PrivacyFingerprint>, Vec<String>) {
    let mut active = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    for value in values {
        let encoded = serde_json::to_string(value).expect("a string always serializes");
        for variant in [value.as_str(), value.trim(), &encoded[1..encoded.len() - 1]] {
            let normalized: String = variant.nfd().collect();
            if normalized.encode_utf16().count() < 6 || normalized.len() <= 5 {
                continue;
            }
            let mut rolling = [0u64; 2];
            for byte in normalized.as_bytes() {
                for index in 0..2 {
                    rolling[index] = rolling[index]
                        .wrapping_mul(BASES[index])
                        .wrapping_add(u64::from(*byte) + 1);
                }
            }
            let mut filter = [0u8; 16];
            filter[..8].copy_from_slice(&rolling[0].to_le_bytes());
            filter[8..].copy_from_slice(&rolling[1].to_le_bytes());
            fingerprints.insert(PrivacyFingerprint {
                length: normalized.len(),
                digest: Sha256::digest(normalized.as_bytes()).to_vec(),
                filter: Sha256::digest(filter).to_vec(),
            });
            active.insert(normalized);
        }
    }
    (
        fingerprints.into_iter().collect(),
        active.into_iter().collect(),
    )
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ledger {
    version: u32,
    pub epoch: String,
    pub revision: u64,
    pub complete: bool,
    pub fingerprints: BTreeSet<PrivacyFingerprint>,
    file_digest: Vec<u8>,
    #[serde(default)]
    location_digest: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Document {
    #[serde(default, rename = "_privacy", skip_serializing_if = "Option::is_none")]
    pub ledger: Option<Ledger>,
    #[serde(flatten)]
    pub secrets: Secrets,
}

fn epoch() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| invalid("secret vault randomness is unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn file_digest(secrets: &Secrets) -> Result<Vec<u8>> {
    Ok(Sha256::digest(serde_json::to_vec(secrets)?).to_vec())
}
fn location_digest(path: &Path) -> Result<Vec<u8>> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("secret vault location is invalid"))?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("secret vault location is invalid"))?;
    let location = fs::canonicalize(parent)?.join(name);
    Ok(Sha256::digest(location.as_os_str().as_encoded_bytes()).to_vec())
}
impl Ledger {
    pub fn new(path: &Path, secrets: &Secrets, complete: bool) -> Result<Self> {
        Ok(Self {
            version: 1,
            epoch: epoch()?,
            revision: 0,
            complete,
            fingerprints: privacy_fingerprints(&values_from_secrets(secrets.clone()))
                .0
                .into_iter()
                .collect(),
            file_digest: file_digest(secrets)?,
            location_digest: Some(location_digest(path)?),
        })
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.epoch.len() != 32
            || !self.epoch.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.file_digest.len() != 32
            || self
                .location_digest
                .as_ref()
                .is_some_and(|digest| digest.len() != 32)
            || self.fingerprints.iter().any(|entry| {
                entry.length <= 5 || entry.digest.len() != 32 || entry.filter.len() != 32
            })
        {
            return Err(invalid("secret vault privacy data is invalid"));
        }
        Ok(())
    }
    pub fn advance(&mut self) -> Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("secret vault privacy revision is exhausted"))?;
        Ok(())
    }
    pub fn update_file(&mut self, secrets: &Secrets) -> Result<()> {
        self.file_digest = file_digest(secrets)?;
        Ok(())
    }
    pub fn reconcile(&mut self, path: &Path, secrets: &Secrets) -> Result<bool> {
        let location = location_digest(path)?;
        if self.file_digest == file_digest(secrets)?
            && self.location_digest.as_ref() == Some(&location)
        {
            return Ok(false);
        }
        self.epoch = epoch()?;
        self.location_digest = Some(location);
        self.complete = false;
        self.advance()?;
        self.fingerprints
            .extend(privacy_fingerprints(&values_from_secrets(secrets.clone())).0);
        self.update_file(secrets)?;
        Ok(true)
    }
}

impl Vault {
    pub fn privacy_snapshot(&self) -> Result<PrivacySnapshot> {
        let _guard = access(&self.path);
        let _file_guard = self.lock_file(true)?;
        #[cfg(any(test, debug_assertions))]
        if self.refuse_keychain {
            return Err(invalid("secret vault privacy data is unavailable"));
        }
        self.reconcile_privacy()?;
        let secrets = self.read_strict()?;
        self.capture_privacy(&secrets, false)?;
        let ledger = self
            .read_document()?
            .ledger
            .ok_or_else(|| invalid("secret vault privacy data is unavailable"))?;
        let (_, active) = privacy_fingerprints(&values_from_secrets(secrets));
        Ok(PrivacySnapshot {
            epoch: ledger.epoch,
            revision: ledger.revision,
            complete: ledger.complete,
            fingerprints: ledger.fingerprints.into_iter().collect(),
            active,
        })
    }
    pub(super) fn reconcile_privacy(&self) -> Result<()> {
        let mut document = self.read_document()?;
        let changed = match document.ledger.as_mut() {
            Some(ledger) => ledger.reconcile(&self.path, &document.secrets)?,
            None => {
                document.ledger = Some(Ledger::new(&self.path, &document.secrets, false)?);
                true
            }
        };
        if changed {
            self.write_document(&document)?;
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    pub(super) fn capture_backend_privacy(&self, secrets: &Secrets) -> Result<()> {
        self.reconcile_privacy()?;
        let mut document = self.read_document()?;
        let ledger = document
            .ledger
            .as_mut()
            .ok_or_else(|| invalid("secret vault privacy data is unavailable"))?;
        let incoming = privacy_fingerprints(&values_from_secrets(secrets.clone())).0;
        if incoming
            .iter()
            .any(|entry| !ledger.fingerprints.contains(entry))
        {
            ledger.epoch = epoch()?;
            ledger.complete = false;
            ledger.advance()?;
            ledger.fingerprints.extend(incoming);
            self.write_document(&document)?;
        }
        Ok(())
    }
    pub(super) fn capture_privacy(&self, secrets: &Secrets, mutation: bool) -> Result<()> {
        self.reconcile_privacy()?;
        let mut document = self.read_document()?;
        let ledger = document
            .ledger
            .as_mut()
            .ok_or_else(|| invalid("secret vault privacy data is unavailable"))?;
        let before = ledger.fingerprints.len();
        ledger
            .fingerprints
            .extend(privacy_fingerprints(&values_from_secrets(secrets.clone())).0);
        if mutation || ledger.fingerprints.len() != before {
            ledger.advance()?;
            self.write_document(&document)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::test_support::TestVault;

    fn contains(snapshot: &PrivacySnapshot, value: &str) -> bool {
        let (patterns, _) = privacy_fingerprints(&[value.to_owned()]);
        patterns
            .iter()
            .all(|entry| snapshot.fingerprints.contains(entry))
    }

    #[test]
    fn mutations_remember_retired_values_before_any_presentation_read() {
        let test = TestVault::new();
        test.vault
            .set("FIRST", "", "phrase synthetique originale")
            .unwrap();
        test.vault
            .set("FIRST", "", "phrase synthetique suivante")
            .unwrap();
        test.vault.remove("FIRST").unwrap();
        let reopened = Vault::open(test.vault.path()).unwrap();
        let snapshot = reopened.privacy_snapshot().unwrap();
        assert!(snapshot.complete);
        assert!(snapshot.active.is_empty());
        assert!(contains(&snapshot, "phrase synthetique originale"));
        assert!(contains(&snapshot, "phrase synthetique suivante"));
        let bytes = fs::read_to_string(test.vault.path()).unwrap();
        assert!(!bytes.contains("phrase synthetique"));
    }

    #[test]
    fn revisions_detect_an_aba_mutation_by_another_handle() {
        let test = TestVault::new();
        let before = test.vault.privacy_snapshot().unwrap();
        let other = Vault::open(test.vault.path()).unwrap();
        assert_eq!(other.privacy_snapshot().unwrap().revision, before.revision);
        other
            .set_for_bot("bot", "FIRST", "", "phrase synthetique temporaire")
            .unwrap();
        other.remove_for_bot("bot", "FIRST").unwrap();
        let after = test.vault.privacy_snapshot().unwrap();
        assert_eq!(before.epoch, after.epoch);
        assert_eq!(before.active, after.active);
        assert!(after.revision > before.revision);
        let revision = after.revision;
        other.remove("ABSENT").unwrap();
        assert!(other.privacy_snapshot().unwrap().revision > revision);
    }

    #[test]
    fn legacy_and_interrupted_files_never_gain_complete_history_on_reopen() {
        let test = TestVault::new();
        fs::write(test.vault.path(), b"{}").unwrap();
        let legacy = Vault::open(test.vault.path()).unwrap();
        let first = legacy.privacy_snapshot().unwrap();
        assert!(!first.complete);
        legacy
            .set("FIRST", "", "phrase synthetique apres migration")
            .unwrap();
        let again = Vault::open(test.vault.path())
            .unwrap()
            .privacy_snapshot()
            .unwrap();
        assert!(!again.complete);
        assert_eq!(first.epoch, again.epoch);
        fs::write(test.vault.path(), b"").unwrap();
        let recovered = Vault::open(test.vault.path())
            .unwrap()
            .privacy_snapshot()
            .unwrap();
        assert!(!recovered.complete);
        assert_ne!(first.epoch, recovered.epoch);
    }

    #[test]
    fn raw_import_changes_epoch_and_preserves_retired_fingerprints() {
        let test = TestVault::new();
        test.vault
            .set("FIRST", "", "phrase synthetique retraitee")
            .unwrap();
        test.vault.remove("FIRST").unwrap();
        let before = test.vault.privacy_snapshot().unwrap();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(test.vault.path()).unwrap()).unwrap();
        document["IMPORTED"] = serde_json::json!({"purpose":"", "updated_at":1, "value":"phrase synthetique importee", "created_at":1});
        fs::write(test.vault.path(), serde_json::to_vec(&document).unwrap()).unwrap();
        let after = test.vault.privacy_snapshot().unwrap();
        assert_ne!(before.epoch, after.epoch);
        assert!(!after.complete);
        assert!(contains(&after, "phrase synthetique retraitee"));
        assert!(contains(&after, "phrase synthetique importee"));
        assert_eq!(
            after.epoch,
            Vault::open(test.vault.path())
                .unwrap()
                .privacy_snapshot()
                .unwrap()
                .epoch
        );
    }

    #[test]
    fn envelope_fields_and_unicode_variants_are_recorded_before_removal() {
        let test = TestVault::new();
        test.vault
            .set(
                "MCP_TEST_ENV",
                MCP_ENV_PURPOSE,
                r#"{"variables":{"TOKEN":"phrase synthétique connecteur"}}"#,
            )
            .unwrap();
        test.vault
            .set(
                "SITE_TEST",
                "Formulaire de site privé",
                r#"{"values":{"field":"phrase synthétique site"}}"#,
            )
            .unwrap();
        test.vault.remove("MCP_TEST_ENV").unwrap();
        test.vault.remove("SITE_TEST").unwrap();
        let snapshot = test.vault.privacy_snapshot().unwrap();
        assert!(contains(&snapshot, "phrase synthe\u{301}tique connecteur"));
        assert!(contains(&snapshot, "phrase synthétique site"));
    }

    #[test]
    fn corrupt_ledger_blocks_mutations_and_disclosure_without_replacing_bytes() {
        let test = TestVault::new();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(test.vault.path()).unwrap()).unwrap();
        document["_privacy"]["version"] = serde_json::json!(99);
        let bytes = serde_json::to_vec(&document).unwrap();
        fs::write(test.vault.path(), &bytes).unwrap();
        assert!(test
            .vault
            .set("FIRST", "", "phrase synthetique refusee")
            .is_err());
        assert!(test.vault.remove("FIRST").is_err());
        assert!(test.vault.privacy_snapshot().is_err());
        assert!(fs::read(test.vault.path()).unwrap() == bytes);
    }

    #[test]
    fn unavailable_backend_refuses_a_privacy_snapshot() {
        let test = TestVault::new();
        let refusing = Vault::open_refusing_keychain(test.vault.path()).unwrap();
        assert!(refusing.privacy_snapshot().is_err());
    }
    #[test]
    #[cfg(unix)]
    fn ledger_persistence_denial_prevents_replacement_and_deletion() {
        use std::os::unix::fs::PermissionsExt;
        let test = TestVault::new();
        test.vault
            .set("FIRST", "", "phrase synthetique conservee")
            .unwrap();
        let before = fs::read(test.vault.path()).unwrap();
        let parent = test.vault.path().parent().unwrap();
        let permissions = fs::metadata(parent).unwrap().permissions();
        fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
        let replacement = test.vault.set("FIRST", "", "phrase synthetique refusee");
        let deletion = test.vault.remove("FIRST");
        fs::set_permissions(parent, permissions).unwrap();
        assert!(replacement.is_err());
        assert!(deletion.is_err());
        assert!(fs::read(test.vault.path()).unwrap() == before);
        let snapshot = test.vault.privacy_snapshot().unwrap();
        assert!(contains(&snapshot, "phrase synthetique conservee"));
        assert!(!contains(&snapshot, "phrase synthetique refusee"));
    }
    #[test]
    fn copying_a_valid_vault_to_another_location_cannot_copy_its_history_claim() {
        let source = TestVault::new();
        source
            .vault
            .set("FIRST", "", "phrase synthetique deplacee")
            .unwrap();
        source.vault.remove("FIRST").unwrap();
        let before = source.vault.privacy_snapshot().unwrap();
        assert!(before.complete);
        let destination = TestVault::new();
        fs::copy(source.vault.path(), destination.vault.path()).unwrap();
        let imported = Vault::open(destination.vault.path()).unwrap();
        let after = imported.privacy_snapshot().unwrap();
        assert_ne!(before.epoch, after.epoch);
        assert!(!after.complete);
        assert!(contains(&after, "phrase synthetique deplacee"));
        let reopened = Vault::open(destination.vault.path())
            .unwrap()
            .privacy_snapshot()
            .unwrap();
        assert_eq!(after.epoch, reopened.epoch);
        assert_eq!(after.revision, reopened.revision);
        assert!(!reopened.complete);
    }
}
