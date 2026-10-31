//! Real GNOME Keyring tests: every operation goes through a throwaway bus and a throwaway
//! HOME, never the session bus or a real keyring.
#![cfg(all(target_os = "linux", debug_assertions))]
#[path = "support/launch_identity.rs"]
mod launch_identity;
#[path = "support/linux_host.rs"]
mod linux_host;
use grokbot_forms::vault::{Vault, MCP_ENV_PURPOSE};
use grokbot_linux_desktop::{secrets::Collection, testing::Keyring};
use linux_host::Host;
use serde_json::json;
use sha2::Digest;
use std::{fs, path::PathBuf};

struct Fixture {
    keyring: Keyring,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Option<Self> {
        let keyring = Keyring::new()?;
        let db = keyring.home().path().join("workspace.sqlite");
        Some(Self { keyring, db })
    }
    fn file(&self) -> PathBuf {
        self.db.with_extension("secrets.json")
    }
    fn vault(&self) -> Vault {
        Vault::open_test_keychain(&self.file(), self.keyring.address()).unwrap()
    }
    fn scope(&self) -> String {
        format!(
            "ca.ezerbot.secrets.v1.{:x}",
            sha2::Sha256::digest(
                fs::canonicalize(self.file())
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
            )
        )
    }
    fn stored(&self) -> Vec<String> {
        let collection = Collection::open(&self.keyring.bus(), self.scope()).unwrap();
        let mut names: Vec<String> = collection
            .entries()
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        names.sort();
        names
    }
    fn launch(&self, override_address: Option<&str>) -> Host {
        self.launch_with(override_address, None)
    }
    fn launch_with(&self, override_address: Option<&str>, ambient: Option<&str>) -> Host {
        linux_host::launch(self.keyring.home(), &self.db, override_address, ambient)
    }
    fn bus(&self) -> String {
        self.keyring.address().to_owned()
    }
}

#[test]
fn real_secret_service_migration_persistence_rotation_and_deletion() {
    let Some(f) = Fixture::new() else { return };
    let file = Vault::open(&f.file()).unwrap();
    file.set("FORM_TOKEN", "Formulaire", "valeur-factice-form-9931")
        .unwrap();
    let private = "valeur-factice-mcp-9931";
    let env = json!({"variables":{"API_TOKEN":private}}).to_string();
    file.set("MCP_TEST_ENV", MCP_ENV_PURPOSE, &env).unwrap();
    let before = file.list().unwrap();
    let vault = f.vault();
    assert_eq!(vault.storage_status().backend, "keychain");
    assert_eq!(vault.list().unwrap(), before);
    assert!(file.list().unwrap().is_empty());
    assert_eq!(f.stored(), ["FORM_TOKEN", "MCP_TEST_ENV"]);
    assert_eq!(
        vault.get("MCP_TEST_ENV").unwrap().as_deref(),
        Some(env.as_str())
    );
    vault
        .set("FORM_TOKEN", "Modification", "valeur-factice-rotated-9931")
        .unwrap();
    assert_eq!(
        f.vault().get("FORM_TOKEN").unwrap().as_deref(),
        Some("valeur-factice-rotated-9931")
    );
    vault.remove("FORM_TOKEN").unwrap();
    assert!(f.vault().get("FORM_TOKEN").unwrap().is_none());
    assert_eq!(f.stored(), ["MCP_TEST_ENV"]);
    assert!(!fs::read_to_string(f.file()).unwrap().contains("9931"));
}

#[test]
fn locked_secret_service_refuses_plaintext_and_unconfirmed_delete_is_not_claimed() {
    let Some(mut f) = Fixture::new() else { return };
    let vault = f.vault();
    vault
        .set("TOKEN", "Test", "valeur-factice-before-lock")
        .unwrap();
    f.keyring.lock();
    let created = vault
        .set("NEW_TOKEN", "Secours", "valeur-factice-fallback")
        .unwrap_err()
        .to_string();
    assert!(created.contains("service de secrets"));
    assert!(!created.contains("valeur-factice"));
    let rotated = vault
        .set("TOKEN", "Rotation en secours", "valeur-factice-latest-file")
        .unwrap_err()
        .to_string();
    assert!(!rotated.contains("valeur-factice"));
    assert_eq!(vault.storage_status().backend, "file_fallback");
    let plaintext = fs::read_to_string(f.file()).unwrap();
    assert!(!plaintext.contains("valeur-factice-fallback"));
    assert!(!plaintext.contains("valeur-factice-latest-file"));
    assert!(Vault::open(&f.file())
        .unwrap()
        .get("NEW_TOKEN")
        .unwrap()
        .is_none());
    let error = vault.remove("TOKEN").unwrap_err().to_string();
    assert!(error.contains("Suppression non confirmée"));
    assert!(!error.contains("valeur-factice"));
    f.keyring.unlock();
    assert_eq!(
        vault.get("TOKEN").unwrap().as_deref(),
        Some("valeur-factice-before-lock")
    );
    assert!(vault.get("NEW_TOKEN").unwrap().is_none());
    assert_eq!(vault.storage_status().backend, "keychain");
    assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
}

#[test]
fn a_service_that_disappears_refuses_new_secrets_and_keeps_the_old_ones() {
    let Some(mut f) = Fixture::new() else { return };
    let vault = f.vault();
    vault
        .set("TOKEN", "Test", "valeur-factice-before-stop")
        .unwrap();
    f.keyring.stop();
    let error = vault
        .set("NEW_TOKEN", "Après arrêt", "valeur-factice-after-stop")
        .unwrap_err()
        .to_string();
    assert!(error.contains("indisponible"));
    assert_eq!(vault.storage_status().backend, "file_fallback");
    assert_eq!(
        vault.storage_status().label,
        "Service de secrets indisponible"
    );
    assert!(!fs::read_to_string(f.file())
        .unwrap()
        .contains("valeur-factice"));
    f.keyring.unlock();
    assert_eq!(
        vault.get("TOKEN").unwrap().as_deref(),
        Some("valeur-factice-before-stop")
    );
    assert!(vault.get("NEW_TOKEN").unwrap().is_none());
}

#[test]
fn wrapper_first_start_projects_storage_and_restart_uses_secret_service_without_plaintext() {
    let Some(f) = Fixture::new() else { return };
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Test HTTP", "valeur-factice-http-9931")
        .unwrap();
    {
        let host = f.launch(Some(&f.bus()));
        let state = host.state();
        assert_eq!(state["forms"]["storage"]["backend"], "keychain");
        assert_eq!(
            state["forms"]["storage"]["label"],
            "Service de secrets du bureau"
        );
        assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
        assert!(!state.to_string().contains("valeur-factice"));
        assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
        assert!(!host.output().contains("valeur-factice"));
    }
    {
        let host = f.launch(Some(&f.bus()));
        assert_eq!(host.state()["forms"]["secrets"][0]["name"], "TOKEN");
        host.delete("TOKEN");
        assert!(!host.output().contains("valeur-factice"));
    }
    assert!(f.launch(Some(&f.bus())).state()["forms"]["secrets"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(f.stored().is_empty());
}

#[test]
fn mcp_after_locked_restart_explains_unavailable_store_instead_of_missing_variables() {
    let Some(f) = Fixture::new() else { return };
    let id = {
        let host = f.launch(None);
        host.post("/api/mcp/add", json!({"name":"Atelier jetable","command":"/usr/bin/false","arguments":[],"environment":{"API_TOKEN":"valeur-factice-mcp-locked"}}))["id"].as_str().unwrap().to_owned()
    };
    {
        let host = f.launch(Some(&f.bus()));
        assert_eq!(host.state()["forms"]["storage"]["backend"], "keychain");
        assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
    }
    f.keyring.lock();
    let host = f.launch(Some(&f.bus())); // No in-memory copy of the migrated value.
    assert_eq!(host.state()["forms"]["storage"]["backend"], "file_fallback");
    host.post("/api/mcp/restart", json!({"id":id}));
    let tool = format!("mcp_start_{id}");
    let state = host.wait_state(|state| {
        state["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"]["tool"] == tool && r["status"]["status"] == "pending")
    });
    let request = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"]["tool"] == tool)
        .unwrap();
    host.post("/api/approvals/resolve", json!({"entry_id":request["entry_id"],"conversation_id":request["route"]["target_conversation_id"],"submission_id":"test-unlock-message","decision":"allow_once"}));
    let state = host.wait_state(|state| {
        state
            .to_string()
            .contains("impossible de récupérer les variables privées de ce connecteur")
    });
    let public = state.to_string();
    assert!(public.contains("Vérifier le stockage"));
    assert!(public.contains("service de secrets du bureau"));
    assert!(!public.contains("macOS"));
    assert!(!public.contains("absentes du coffre"));
    assert!(!public.contains("valeur-factice-mcp-locked"));
    assert!(!host.output().contains("valeur-factice"));
}

#[test]
fn denied_deletion_rolls_back_the_mcp_connector_removal() {
    let Some(mut f) = Fixture::new() else { return };
    let id = {
        let host = f.launch(None);
        host.post("/api/mcp/add", json!({"name":"Suppression jetable","command":"/usr/bin/false","arguments":[],"environment":{"API_TOKEN":"valeur-factice-delete"}}))["id"].as_str().unwrap().to_owned()
    };
    {
        let host = f.launch(Some(&f.bus()));
        assert_eq!(host.state()["forms"]["storage"]["backend"], "keychain");
    }
    f.keyring.lock();
    {
        let host = f.launch(Some(&f.bus()));
        host.post("/api/mcp/remove", json!({"id":id}));
        let tool = format!("mcp_remove_{id}");
        let state = host.wait_state(|s| {
            s["forms"]["requests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"]["tool"] == tool && r["status"]["status"] == "pending")
        });
        let request = state["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"]["tool"] == tool)
            .unwrap();
        host.post("/api/approvals/resolve", json!({"entry_id":request["entry_id"],"conversation_id":request["route"]["target_conversation_id"],"submission_id":"test-denied-delete","decision":"allow_once"}));
        let state = host.wait_state(|s| s.to_string().contains("Suppression non confirmée"));
        assert_eq!(state["mcp"]["servers"].as_array().unwrap().len(), 1);
    }
    f.keyring.unlock();
    let host = f.launch(Some(&f.bus()));
    assert_eq!(host.state()["mcp"]["servers"].as_array().unwrap().len(), 1);
    assert_eq!(
        host.state()["forms"]["secrets"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn debug_without_override_stays_on_file_even_when_a_keyring_is_on_the_ambient_bus() {
    let Some(f) = Fixture::new() else { return };
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Sans override", "valeur-factice-no-override")
        .unwrap();
    for _ in 0..2 {
        let host = f.launch_with(None, Some(&f.bus()));
        let state = host.state();
        assert_eq!(state["forms"]["storage"]["backend"], "file");
        assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
        assert_eq!(
            Vault::open(&f.file())
                .unwrap()
                .get("TOKEN")
                .unwrap()
                .as_deref(),
            Some("valeur-factice-no-override")
        );
    }
    assert!(f.stored().is_empty());
}

#[test]
fn invalid_debug_override_never_falls_through_and_french_notice_is_visible() {
    let Some(f) = Fixture::new() else { return };
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Test", "valeur-factice-file-9931")
        .unwrap();
    let socket = f.bus();
    for malformed in [
        format!("{socket},guid=0123456789abcdef0123456789abcdef"),
        socket.replace("unix:path=", "unix:abstract="),
        "unix:path=/tmp/ezerbot-not-under-home/bus".to_owned(),
        format!("{socket};unix:path=/tmp/other"),
    ] {
        let host = f.launch_with(Some(&malformed), Some(&f.bus()));
        let state = host.state();
        assert_eq!(state["forms"]["storage"]["backend"], "file_fallback");
        assert_eq!(
            state["forms"]["storage"]["label"],
            "Service de secrets indisponible"
        );
        let notice = state["forms"]["storage"]["notice"].as_str().unwrap();
        assert!(notice.contains("ne sont pas enregistrés"));
        assert!(!notice.contains("fichier de secours"));
        assert!(!notice.contains("non chiffré"));
        assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
    }
    let Some(outside) = Fixture::new() else {
        return;
    };
    let link = f.keyring.home().path().join("bus-lien");
    let target = outside.bus();
    std::os::unix::fs::symlink(target.strip_prefix("unix:path=").unwrap(), &link).unwrap();
    let host = f.launch(Some(&format!("unix:path={}", link.display())));
    assert_eq!(
        host.state()["forms"]["storage"]["backend"],
        "file_fallback",
        "a link under HOME must not reach a bus outside it"
    );
    assert!(f.stored().is_empty());
    assert!(fs::read_to_string(f.file())
        .unwrap()
        .contains("valeur-factice-file-9931"));
}
