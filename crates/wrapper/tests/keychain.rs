//! Real Security.framework tests: every operation explicitly selects a disposable
//! keychain created by `security` inside a throwaway HOME. No login search/list.
#![cfg(all(target_os = "macos", debug_assertions))]
#[path = "support/launch_identity.rs"]
mod launch_identity;
use grokbot_forms::vault::{Vault, MCP_ENV_PURPOSE};
use launch_identity::LaunchIdentity;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const PASSWORD: &str = "ezerbot-throwaway-test-991";
// Admission simulation only; no real signing or trust settings in tests.
const CERTIFICATE_REQUIREMENT: &str =
    "certificate leaf = H\"0123456789ABCDEF0123456789ABCDEF01234567\"";
struct Fixture {
    home: PathBuf,
    keychain: PathBuf,
    db: PathBuf,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-keychain");
        let home = guard.to_path_buf();
        fs::write(home.join(".ezerbot-test-home"), "test").unwrap();
        let f = Self {
            keychain: home.join("ezerbot-test.keychain-db"),
            db: home.join("workspace.sqlite"),
            home,
            _dir: guard,
        };
        f.security(&["create-keychain", "-p", PASSWORD]);
        f.security(&["unlock-keychain", "-p", PASSWORD]);
        f
    }
    fn security(&self, args: &[&str]) {
        let result = Command::new("/usr/bin/security")
            .args(args)
            .arg(&self.keychain)
            .env("HOME", &self.home)
            .env("CFFIXED_USER_HOME", &self.home)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "throwaway security command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fn file(&self) -> PathBuf {
        self.db.with_extension("secrets.json")
    }
    fn vault(&self) -> Vault {
        Vault::open_test_keychain(&self.file(), &self.keychain).unwrap()
    }
    fn launch(&self, override_path: &Path) -> Host {
        self.launch_binary(
            Path::new(env!("CARGO_BIN_EXE_grokbot-wrapper")),
            Some(override_path),
            Some(CERTIFICATE_REQUIREMENT),
        )
    }
    fn launch_with_override(&self, override_path: Option<&Path>) -> Host {
        self.launch_binary(
            Path::new(env!("CARGO_BIN_EXE_grokbot-wrapper")),
            override_path,
            None,
        )
    }
    fn launch_binary(
        &self,
        binary: &Path,
        override_path: Option<&Path>,
        requirement: Option<&str>,
    ) -> Host {
        let log = fs::File::create(self.home.join("host.log")).unwrap();
        let mut command = Command::new(binary);
        command
            .args(["--port", "0", "--db"])
            .arg(&self.db)
            .env("HOME", &self.home)
            .env("CFFIXED_USER_HOME", &self.home)
            .env_remove("GROKBOT_TEST_KEYCHAIN")
            .env_remove("GROKBOT_TEST_SIGNATURE_REQUIREMENT")
            .env_remove("GROKBOT_FAKE_OPENAI")
            .stdout(log)
            .stderr(Stdio::null());
        if let Some(path) = override_path {
            command.env("GROKBOT_TEST_KEYCHAIN", path);
        }
        if let Some(requirement) = requirement {
            command.env("GROKBOT_TEST_SIGNATURE_REQUIREMENT", requirement);
        }
        let identity = LaunchIdentity::new();
        let mut child = identity.spawn(&mut command);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = fs::read_to_string(self.home.join("host.log")).unwrap();
            if let Some(url) = text.lines().find_map(|line| {
                serde_json::from_str::<Value>(line)
                    .ok()
                    .and_then(|v| v["url"].as_str().map(str::to_owned))
            }) {
                return Host {
                    child,
                    identity,
                    url,
                };
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "host exited before serving"
            );
            assert!(Instant::now() < deadline, "host startup timed out");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/security")
            .arg("delete-keychain")
            .arg(&self.keychain)
            .env("HOME", &self.home)
            .env("CFFIXED_USER_HOME", &self.home)
            .output();
    }
}
struct Host {
    child: Child,
    identity: LaunchIdentity,
    url: String,
}
impl Host {
    fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .set("X-Ezerbot-Launch", &self.identity.value())
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn post(&self, path: &str, body: Value) -> Value {
        ureq::post(&format!("{}{path}", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
            .set("X-Ezerbot-Launch", &self.identity.value())
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn wait_state(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.state();
            if predicate(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "state transition timed out");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    fn delete(&self, name: &str) -> ureq::Response {
        ureq::post(&format!("{}/api/secrets/delete", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
            .set("X-Ezerbot-Launch", &self.identity.value())
            .send_json(json!({"name":name}))
            .unwrap()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn adhoc_reconstruction_preserves_the_complete_file_without_any_migration() {
    let f = Fixture::new();
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Avant reconstruction", "valeur-factice-rebuild")
        .unwrap();
    let before = fs::read(f.file()).unwrap();
    let rebuilt = f.home.join("wrapper-reconstruit");
    fs::copy(env!("CARGO_BIN_EXE_grokbot-wrapper"), &rebuilt).unwrap();
    for (binary, requirement) in [
        (
            Path::new(env!("CARGO_BIN_EXE_grokbot-wrapper")),
            "cdhash H\"1111111111111111111111111111111111111111\"",
        ),
        (
            rebuilt.as_path(),
            "cdhash H\"2222222222222222222222222222222222222222\"",
        ),
    ] {
        let host = f.launch_binary(binary, Some(&f.keychain), Some(requirement));
        let state = host.state();
        assert_eq!(state["forms"]["storage"]["backend"], "file");
        assert_eq!(state["forms"]["storage"]["notice_kind"], "info");
        assert!(state["forms"]["storage"]["notice"]
            .as_str()
            .unwrap()
            .contains("signature stable"));
        assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
        assert_eq!(fs::read(f.file()).unwrap(), before);
    }
}

#[test]
fn unsigned_host_identifies_previous_items_without_changing_them() {
    let f = Fixture::new();
    f.vault()
        .set("TOKEN", "Version précédente", "valeur-factice-previous")
        .unwrap();
    let host = f.launch_with_override(Some(&f.keychain));
    let state = host.state();
    assert_eq!(state["forms"]["storage"]["backend"], "file");
    assert!(state["forms"]["storage"]["notice"]
        .as_str()
        .unwrap()
        .contains("version précédente"));
    assert_eq!(
        f.vault().get("TOKEN").unwrap().as_deref(),
        Some("valeur-factice-previous")
    );
}

#[test]
fn real_keychain_migration_persistence_rotation_and_deletion() {
    let f = Fixture::new();
    let file = Vault::open(&f.file()).unwrap();
    file.set("FORM_TOKEN", "Formulaire", "valeur-factice-form-9931")
        .unwrap();
    let env = json!({"variables":{"API_TOKEN":"valeur-factice-mcp-9931"}}).to_string();
    file.set("MCP_TEST_ENV", MCP_ENV_PURPOSE, &env).unwrap();
    let before = file.list().unwrap();
    let vault = f.vault();
    assert_eq!(vault.storage_status().backend, "keychain");
    assert_eq!(vault.list().unwrap(), before);
    assert!(file.list().unwrap().is_empty());
    // Metadata query only (-g/-w are deliberately absent): no password logging.
    use sha2::Digest;
    let service = format!(
        "ca.ezerbot.secrets.v1.{:x}",
        sha2::Sha256::digest(
            fs::canonicalize(f.file())
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
        )
    );
    let metadata = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", &service, "-a", "FORM_TOKEN"])
        .arg(&f.keychain)
        .env("HOME", &f.home)
        .env("CFFIXED_USER_HOME", &f.home)
        .output()
        .unwrap();
    assert!(metadata.status.success());
    let attributes = String::from_utf8(metadata.stdout).unwrap();
    // security renders a non-ASCII label either literally or as hex + octal.
    let label = "Ezerbot — FORM_TOKEN";
    let label_hex: String = label
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert!(attributes.contains(label) || attributes.to_ascii_lowercase().contains(&label_hex));
    assert!(attributes.contains("Secret Ezerbot"));
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
    assert!(!fs::read_to_string(f.file()).unwrap().contains("9931"));
}

#[test]
fn locked_keychain_refuses_plaintext_and_unconfirmed_delete_is_not_claimed() {
    let f = Fixture::new();
    let vault = f.vault();
    vault
        .set("TOKEN", "Test", "valeur-factice-before-lock")
        .unwrap();
    f.security(&["lock-keychain"]);
    let created = vault
        .set("NEW_TOKEN", "Secours", "valeur-factice-fallback")
        .unwrap_err()
        .to_string();
    assert!(created.contains("trousseau"));
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
    f.security(&["unlock-keychain", "-p", PASSWORD]);
    assert_eq!(
        vault.get("TOKEN").unwrap().as_deref(),
        Some("valeur-factice-before-lock")
    );
    assert!(vault.get("NEW_TOKEN").unwrap().is_none());
    assert_eq!(vault.storage_status().backend, "keychain");
    assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
}

#[test]
fn wrapper_first_start_projects_storage_and_restart_uses_keychain_without_plaintext() {
    let f = Fixture::new();
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Test HTTP", "valeur-factice-http-9931")
        .unwrap();
    {
        let host = f.launch(&f.keychain);
        let state = host.state();
        assert_eq!(state["forms"]["storage"]["backend"], "keychain");
        assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
        assert!(!state.to_string().contains("valeur-factice"));
        assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
    }
    {
        let host = f.launch(&f.keychain);
        assert_eq!(host.state()["forms"]["secrets"][0]["name"], "TOKEN");
        host.delete("TOKEN");
    }
    assert!(f.launch(&f.keychain).state()["forms"]["secrets"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn mcp_after_locked_restart_explains_unavailable_keychain_instead_of_missing_variables() {
    let f = Fixture::new();
    let id = {
        let host = f.launch_with_override(None);
        host.post("/api/mcp/add", json!({"name":"Atelier jetable","command":"/usr/bin/false","arguments":[],"environment":{"API_TOKEN":"valeur-factice-mcp-locked"}}))["id"].as_str().unwrap().to_owned()
    };
    {
        let host = f.launch(&f.keychain);
        assert_eq!(host.state()["forms"]["storage"]["backend"], "keychain");
        assert!(Vault::open(&f.file()).unwrap().list().unwrap().is_empty());
    }
    f.security(&["lock-keychain"]);
    let host = f.launch(&f.keychain); // No in-memory copy of the migrated value.
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
    assert!(!public.contains("absentes du coffre"));
    assert!(!public.contains("valeur-factice-mcp-locked"));
}

#[test]
fn denied_keychain_deletion_rolls_back_the_mcp_connector_removal() {
    let f = Fixture::new();
    let id = {
        let host = f.launch_with_override(None);
        host.post("/api/mcp/add", json!({"name":"Suppression jetable","command":"/usr/bin/false","arguments":[],"environment":{"API_TOKEN":"valeur-factice-delete"}}))["id"].as_str().unwrap().to_owned()
    };
    {
        let host = f.launch(&f.keychain);
        assert_eq!(host.state()["forms"]["storage"]["backend"], "keychain");
    }
    f.security(&["lock-keychain"]);
    {
        let host = f.launch(&f.keychain);
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
    f.security(&["unlock-keychain", "-p", PASSWORD]);
    let host = f.launch(&f.keychain);
    assert_eq!(host.state()["mcp"]["servers"].as_array().unwrap().len(), 1);
    assert_eq!(
        host.state()["forms"]["secrets"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn debug_without_override_stays_on_file_and_preserves_values_across_restart() {
    let f = Fixture::new();
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Sans override", "valeur-factice-no-override")
        .unwrap();
    for _ in 0..2 {
        let host = f.launch_with_override(None);
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
}

#[test]
fn invalid_debug_override_never_falls_through_to_login_and_french_notice_is_visible() {
    let f = Fixture::new();
    Vault::open(&f.file())
        .unwrap()
        .set("TOKEN", "Test", "valeur-factice-file-9931")
        .unwrap();
    let host = f.launch(&f.home.join("missing.keychain-db"));
    let state = host.state();
    assert_eq!(state["forms"]["storage"]["backend"], "file_fallback");
    assert_eq!(
        state["forms"]["storage"]["label"],
        "Trousseau macOS indisponible"
    );
    let notice = state["forms"]["storage"]["notice"].as_str().unwrap();
    assert!(notice.contains("ne sont pas enregistrés"));
    assert!(!notice.contains("fichier de secours"));
    assert!(!notice.contains("non chiffré"));
    assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
}
