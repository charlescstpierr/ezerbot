//! Real Security.framework tests: every operation explicitly selects a disposable
//! keychain created by `security` inside a throwaway HOME. No login search/list.
#![cfg(all(target_os = "macos", debug_assertions))]
use grokbot_forms::vault::{Vault, MCP_ENV_PURPOSE};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const PASSWORD: &str = "ezerbot-throwaway-test-991";
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    home: PathBuf,
    keychain: PathBuf,
    db: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!(
            "ezerbot-keychain-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join(".ezerbot-test-home"), "test").unwrap();
        let f = Self {
            keychain: home.join("ezerbot-test.keychain-db"),
            db: home.join("workspace.sqlite"),
            home,
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
        self.launch_with_override(Some(override_path))
    }
    fn launch_with_override(&self, override_path: Option<&Path>) -> Host {
        let log = fs::File::create(self.home.join("host.log")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"));
        command
            .args(["--port", "0", "--db"])
            .arg(&self.db)
            .env("HOME", &self.home)
            .env("CFFIXED_USER_HOME", &self.home)
            .env_remove("GROKBOT_TEST_KEYCHAIN")
            .env_remove("GROKBOT_FAKE_OPENAI")
            .stdout(log)
            .stderr(Stdio::null());
        if let Some(path) = override_path {
            command.env("GROKBOT_TEST_KEYCHAIN", path);
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = fs::read_to_string(self.home.join("host.log")).unwrap();
            if let Some(url) = text.lines().find_map(|line| {
                serde_json::from_str::<Value>(line)
                    .ok()
                    .and_then(|v| v["url"].as_str().map(str::to_owned))
            }) {
                return Host { child, url };
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
        let _ = fs::remove_dir_all(&self.home);
    }
}
struct Host {
    child: Child,
    url: String,
}
impl Host {
    fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn delete(&self, name: &str) -> ureq::Response {
        ureq::post(&format!("{}/api/secrets/delete", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
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
fn locked_keychain_falls_back_without_dialog_then_recovers_and_denied_delete_is_not_claimed() {
    let f = Fixture::new();
    let vault = f.vault();
    vault
        .set("TOKEN", "Test", "valeur-factice-before-lock")
        .unwrap();
    f.security(&["lock-keychain"]);
    vault
        .set("NEW_TOKEN", "Secours", "valeur-factice-fallback")
        .unwrap();
    vault
        .set("TOKEN", "Rotation en secours", "valeur-factice-latest-file")
        .unwrap();
    assert_eq!(vault.storage_status().backend, "file_fallback");
    let error = vault.remove("TOKEN").unwrap_err().to_string();
    assert!(error.contains("Suppression non confirmée"));
    assert!(!error.contains("valeur-factice"));
    assert_eq!(
        Vault::open(&f.file())
            .unwrap()
            .get("NEW_TOKEN")
            .unwrap()
            .as_deref(),
        Some("valeur-factice-fallback")
    );
    f.security(&["unlock-keychain", "-p", PASSWORD]);
    assert_eq!(f.vault().list().unwrap().len(), 2);
    assert_eq!(
        vault.get("TOKEN").unwrap().as_deref(),
        Some("valeur-factice-latest-file")
    );
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
    assert!(state["forms"]["storage"]["notice"]
        .as_str()
        .unwrap()
        .contains("non chiffré"));
    assert_eq!(state["forms"]["secrets"][0]["name"], "TOKEN");
}
