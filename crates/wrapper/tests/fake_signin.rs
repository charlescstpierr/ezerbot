//! `EZERBOT_FAKE_SIGNIN=1` n’est honoré que par le binaire de debug.
#![cfg(debug_assertions)]
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch(directory: &std::path::Path, fake: bool) -> (Running, String) {
    fs::create_dir_all(directory).unwrap();
    let db = directory.join("workspace.sqlite");
    let mut command = Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"));
    command
        .args(["--port", "0", "--db"])
        .arg(&db)
        .env("HOME", directory)
        .env("CFFIXED_USER_HOME", directory)
        .env_remove("GROKBOT_FAKE_OPENAI")
        .env_remove("GROKBOT_TEST_KEYCHAIN")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if fake {
        command.env("EZERBOT_FAKE_SIGNIN", "1");
    } else {
        command.env_remove("EZERBOT_FAKE_SIGNIN");
    }
    let child = command.spawn().unwrap();
    let mut running = Running(child);
    let mut first = String::new();
    BufReader::new(running.0.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let value: Value = serde_json::from_str(first.trim()).expect(&first);
    let address = value["url"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
        .to_owned();
    (running, address)
}

fn state(address: &str) -> Value {
    let mut socket = std::net::TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        socket,
        "GET /api/state HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut result = String::new();
    socket.read_to_string(&mut result).unwrap();
    let body = result.split("\r\n\r\n").nth(1).unwrap_or("");
    serde_json::from_str(body).unwrap_or_else(|_| panic!("{result}"))
}

#[test]
fn la_connexion_factice_ouvre_chatgpt_sans_oauth() {
    let directory = std::env::temp_dir().join(format!(
        "ezerbot-fake-signin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (_running, address) = launch(&directory, true);
    let ready = |provider: &Value| {
        provider["connected"] == true
            && provider["email"] == "vous@exemple.com"
            && provider["models"]
                .as_array()
                .is_some_and(|models| models.iter().any(|model| model["slug"] == "gpt-test"))
    };
    let mut provider = state(&address)["provider"].clone();
    for _ in 0..40 {
        if ready(&provider) {
            break;
        }
        thread::sleep(Duration::from_millis(250));
        provider = state(&address)["provider"].clone();
    }
    assert_eq!(provider["connected"], true, "{provider}");
    assert_eq!(provider["signed_in"], true, "{provider}");
    assert_eq!(provider["email"], "vous@exemple.com");
    assert_eq!(provider["plan"], "plus");
    assert_eq!(provider["welcomed"], true);
    assert_eq!(provider["signin"], "idle");
    assert_eq!(provider["backend"], "codex");
    assert!(
        provider["models"]
            .as_array()
            .is_some_and(|models| models.iter().any(|model| model["slug"] == "gpt-test")),
        "{provider}"
    );
    let session = directory.join("workspace.codex-auth.json");
    let mode = fs::metadata(&session).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let text = fs::read_to_string(&session).unwrap();
    assert!(text.contains("vous@exemple.com"));
    assert!(text.contains("account-fake"));
    assert!(!text.contains("sk-"));
    assert!(!directory.join("workspace.chatgpt-auth.json").exists());
    let _ = fs::remove_dir_all(&directory);
}

#[test]
fn sans_le_drapeau_la_session_reste_locale() {
    let directory = std::env::temp_dir().join(format!(
        "ezerbot-fake-signin-off-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (_running, address) = launch(&directory, false);
    let provider = state(&address)["provider"].clone();
    assert_eq!(provider["connected"], false, "{provider}");
    assert_eq!(provider["signed_in"], false, "{provider}");
    assert!(!directory.join("workspace.codex-auth.json").exists());
    let _ = fs::remove_dir_all(&directory);
}
