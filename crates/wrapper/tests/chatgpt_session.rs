//! La session ChatGPT survit au redémarrage, à la mise à jour et à un jeton expiré.
//! La reconnexion n’est demandée que si le serveur refuse le refresh_token.
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{apply_in_app_update, Provider};
use serde_json::{json, Value};
use support::{wait, FakeOpenAi, Seen, CLIENT};

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "ezerbot-session-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn auth(refresh: &str, expires_at: u64) -> Value {
    json!({
        "host_id": "host-test",
        "client_id": CLIENT,
        "email": "charles@example.test",
        "access_token": "access-stale",
        "refresh_token": refresh,
        "expires_at": expires_at,
        "scopes": "chatgpt.tokens.use.direct email offline_access openid profile",
        "saved_at": 1
    })
}

fn write_auth(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, value.to_string()).unwrap();
}

fn saved(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn provider(db: &Path, server: &FakeOpenAi) -> Provider {
    Provider::with_endpoints(db, server.endpoints(), false)
}

struct Outcome {
    ok: bool,
    kind: &'static str,
    message: String,
}

fn ask(provider: &Provider) -> Outcome {
    match provider.responses(json!({"model": "gpt-test", "stream": true, "input": "ping"})) {
        Ok(_) => Outcome {
            ok: true,
            kind: "",
            message: String::new(),
        },
        Err(error) => Outcome {
            ok: false,
            kind: error.kind,
            message: error.message,
        },
    }
}

#[test]
fn expired_token_refreshes_silently_and_survives_restart() {
    let root = scratch("refresh");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);

    assert!(ask(&session).ok);
    let file = saved(&auth_path);
    assert!(file["expires_at"].as_u64().unwrap() > now());
    assert_ne!(file["access_token"], "access-stale");
    assert_ne!(file["refresh_token"], "stale-refresh");
    assert!(file["refresh_token"].is_string());
    assert!(session.connected());
    assert_eq!(session.status()["signin"], "idle");
    assert!(seen.lock().unwrap().forms.iter().any(|form| {
        form.get("grant_type").map(String::as_str) == Some("refresh_token")
            && form.get("refresh_token").map(String::as_str) == Some("stale-refresh")
    }));

    let refresh = file["refresh_token"].clone();
    let forms = seen.lock().unwrap().forms.len();
    drop(session);
    let restarted = provider(&db, &server);
    assert!(restarted.connected());
    assert_eq!(saved(&auth_path)["refresh_token"], refresh);
    assert_eq!(seen.lock().unwrap().forms.len(), forms);
    assert_eq!(restarted.status()["signin"], "idle");
}

#[test]
fn sign_in_is_requested_only_when_the_server_refuses_the_refresh() {
    let root = scratch("refused");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen);
    let session = provider(&db, &server);
    let error = ask(&session);
    assert!(!error.ok);
    assert_eq!(error.kind, "signin_expired");
    assert!(error.message.contains("Reconnectez-vous"));
    assert!(!session.connected());
    assert!(saved(&auth_path)["refresh_token"].is_null());
}

#[test]
fn a_transient_refresh_failure_keeps_the_saved_session() {
    let root = scratch("transient");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("kept-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_transient: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen);
    let session = provider(&db, &server);
    let error = ask(&session);
    assert!(!error.ok);
    assert_eq!(error.kind, "other");
    assert!(session.connected());
    assert_eq!(saved(&auth_path)["refresh_token"], "kept-refresh");
}

#[test]
fn a_refresh_that_drops_the_plan_scope_keeps_the_previous_grant() {
    let root = scratch("scope");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_scope: Some("openid profile".into()),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen);
    let session = provider(&db, &server);
    assert!(ask(&session).ok);
    let scopes = saved(&auth_path)["scopes"].as_str().unwrap().to_owned();
    assert!(scopes.contains("chatgpt.tokens.use.direct"));
    assert!(session.connected());
}

#[test]
fn a_refused_refresh_does_not_delete_a_token_saved_by_the_other_process() {
    let root = scratch("race");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let gate = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        token_gate: Some(gate.clone()),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let worker = session.clone();
    let handle = std::thread::spawn(move || ask(&worker));
    wait("jeton en cours de refus", || {
        seen.lock().unwrap().forms.len() == 1
    });
    write_auth(
        &auth_path,
        &auth("winner-refresh", now().saturating_add(86_400)),
    );
    let winner = fs::read(&auth_path).unwrap();
    gate.store(true, Ordering::SeqCst);
    let outcome = handle.join().unwrap();
    assert!(outcome.ok);
    assert_eq!(fs::read(&auth_path).unwrap(), winner);
    assert_eq!(saved(&auth_path)["refresh_token"], "winner-refresh");
    assert!(session.connected());
    assert_ne!(session.status()["signin"], "pending");
}

#[test]
fn legacy_sibling_folder_is_copied_once_and_left_in_place() {
    let root = scratch("legacy").join("Library/Application Support");
    let legacy = root.join("Grokbot Local/workspace.chatgpt-auth.json");
    write_auth(
        &legacy,
        &auth("legacy-refresh", now().saturating_add(86_400)),
    );
    let db = root.join("Grokbot/workspace.sqlite");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen);
    let session = provider(&db, &server);
    assert!(session.connected());
    let imported = root.join("Grokbot/workspace.chatgpt-auth.json");
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");
    assert_eq!(saved(&legacy)["refresh_token"], "legacy-refresh");

    let unrelated = scratch("unrelated");
    write_auth(
        &unrelated.join("Other/workspace.chatgpt-auth.json"),
        &auth("stray-refresh", now().saturating_add(86_400)),
    );
    let stray = provider(&unrelated.join("NotGrokbot/workspace.sqlite"), &server);
    assert!(!stray.connected());
    assert!(!unrelated
        .join("NotGrokbot/workspace.chatgpt-auth.json")
        .exists());
}

#[test]
fn in_app_update_replaces_the_binary_and_keeps_the_auth_file() {
    let install = scratch("install");
    let data = scratch("data");
    let staged = scratch("staged");
    let auth_path = data.join("workspace.chatgpt-auth.json");
    let original = auth("kept-refresh", now().saturating_add(86_400));
    write_auth(&auth_path, &original);
    fs::write(data.join("workspace.sqlite"), b"sqlite-bytes").unwrap();
    fs::write(data.join("provider.json"), b"{\"codex\":false}").unwrap();
    fs::write(install.join("Ezerbot"), b"old-binary").unwrap();
    fs::create_dir_all(staged.join("MacOS")).unwrap();
    fs::write(staged.join("MacOS/Ezerbot"), b"new-binary").unwrap();
    fs::write(staged.join("notes.txt"), b"release-notes").unwrap();
    fs::write(
        staged.join("workspace.chatgpt-auth.json"),
        b"{\"refresh_token\":null}",
    )
    .unwrap();
    fs::write(staged.join("workspace.sqlite"), b"wiped").unwrap();
    fs::write(staged.join("provider.json"), b"wiped").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("Ezerbot", staged.join("MacOS/linked")).unwrap();

    apply_in_app_update(&install, &staged, &data).unwrap();
    assert_eq!(
        fs::read(install.join("MacOS/Ezerbot")).unwrap(),
        b"new-binary"
    );
    assert_eq!(
        fs::read(install.join("notes.txt")).unwrap(),
        b"release-notes"
    );
    assert!(!install.join("MacOS/linked").exists());
    assert!(!install.join("workspace.chatgpt-auth.json").exists());
    assert!(!install.join("workspace.sqlite").exists());
    assert_eq!(
        fs::read(&auth_path).unwrap(),
        original.to_string().into_bytes()
    );
    assert_eq!(
        fs::read(data.join("workspace.sqlite")).unwrap(),
        b"sqlite-bytes"
    );
    assert_eq!(
        fs::read(data.join("provider.json")).unwrap(),
        b"{\"codex\":false}"
    );

    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen);
    let session = provider(&data.join("workspace.sqlite"), &server);
    assert!(session.connected());
    assert_eq!(saved(&auth_path)["refresh_token"], "kept-refresh");

    let same = scratch("same");
    let same_auth = same.join("workspace.chatgpt-auth.json");
    write_auth(&same_auth, &original);
    fs::write(same.join("Ezerbot"), b"old-binary").unwrap();
    let staged_same = scratch("staged-same");
    fs::write(staged_same.join("Ezerbot"), b"replaced-binary").unwrap();
    fs::write(
        staged_same.join("workspace.chatgpt-auth.json"),
        b"{\"refresh_token\":null}",
    )
    .unwrap();
    apply_in_app_update(&same, &staged_same, &same).unwrap();
    assert_eq!(fs::read(same.join("Ezerbot")).unwrap(), b"replaced-binary");
    assert_eq!(
        fs::read(&same_auth).unwrap(),
        original.to_string().into_bytes()
    );
    assert!(provider(&same.join("workspace.sqlite"), &server).connected());
}
