//! La session ChatGPT survit au redémarrage, à la mise à jour et à un jeton expiré.
//! La reconnexion n’est demandée que si le serveur refuse le refresh_token.
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{Endpoints, Provider};
use serde_json::{json, Value};
use support::{form, wait, FakeOpenAi, Seen, CLIENT};

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

fn codex_provider(db: &Path, server: &FakeOpenAi) -> Provider {
    Provider::with_backends(
        db,
        server.endpoints(),
        Endpoints {
            api: format!("{}/backend-api/codex", server.base),
            ..server.endpoints()
        },
        false,
    )
}

/// Flock tenu par ce processus, distinct du descripteur de la bibliothèque.
fn hold_lock(auth: &Path) -> nix::fcntl::Flock<fs::File> {
    let path = auth.with_extension("lock");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path).unwrap();
    nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive).unwrap()
}

fn touch(path: &Path, when: SystemTime) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
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
fn legacy_sibling_folder_is_renamed_once_and_not_duplicated() {
    let root = scratch("legacy").join("Library/Application Support");
    let legacy = root.join("Grokbot Local/workspace.chatgpt-auth.json");
    write_auth(
        &legacy,
        &auth("legacy-refresh", now().saturating_add(86_400)),
    );
    let db = root.join("Grokbot/workspace.sqlite");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    assert!(session.connected());
    let imported = root.join("Grokbot/workspace.chatgpt-auth.json");
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");
    assert!(legacy.exists());
    assert!(saved(&legacy)["refresh_token"].is_null());
    assert_eq!(seen.lock().unwrap().forms.len(), 0);

    let again = provider(&db, &server);
    assert!(again.connected());
    assert!(legacy.exists());
    assert!(saved(&legacy)["refresh_token"].is_null());
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");

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

    let backwards = scratch("backwards").join("Library/Application Support");
    write_auth(
        &backwards.join("Grokbot/workspace.chatgpt-auth.json"),
        &auth("real-refresh", now().saturating_add(86_400)),
    );
    let local_db = backwards.join("Grokbot Local/workspace.sqlite");
    let local = provider(&local_db, &server);
    assert!(!local.connected());
    assert!(!backwards
        .join("Grokbot Local/workspace.chatgpt-auth.json")
        .exists());
    assert_eq!(
        saved(&backwards.join("Grokbot/workspace.chatgpt-auth.json"))["refresh_token"],
        "real-refresh"
    );
}

#[test]
fn charles_codex_file_in_application_support_is_found_without_reconnect() {
    let root = scratch("charles").join("Library/Application Support/Grokbot");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.codex-auth.json");
    write_auth(
        &auth_path,
        &json!({
            "host_id": "host-charles",
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "access_token": "access-charles",
            "refresh_token": "refresh-charles",
            "account_id": "account-charles",
            "expires_at": now().saturating_add(86_400),
            "scopes": "",
            "saved_at": 1
        }),
    );
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = Provider::with_backends(
        &db,
        server.endpoints(),
        grokbot_wrapper::Endpoints {
            api: format!("{}/backend-api/codex", server.base),
            ..server.endpoints()
        },
        false,
    );
    assert!(session.connected());
    assert_eq!(session.status()["signin"], "idle");
    assert_eq!(session.status()["backend"], "codex");
    assert!(seen.lock().unwrap().forms.is_empty());
    assert_eq!(saved(&auth_path)["refresh_token"], "refresh-charles");
    assert!(session.status()["error"].is_null());
}

#[test]
fn a_single_invalid_grant_retries_the_stored_token_before_clearing() {
    let root = scratch("once");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_refusals: 1,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let outcome = ask(&session);
    assert!(outcome.ok, "{}", outcome.message);
    assert_ne!(outcome.kind, "signin_expired");
    assert_ne!(outcome.kind, "signin");
    assert!(session.connected());
    assert_eq!(session.status()["signin"], "idle");
    assert!(saved(&auth_path)["refresh_token"].is_string());
    assert_ne!(saved(&auth_path)["refresh_token"], "stale-refresh");
    let refreshes = seen
        .lock()
        .unwrap()
        .forms
        .iter()
        .filter(|form| form.get("grant_type").map(String::as_str) == Some("refresh_token"))
        .count();
    assert_eq!(refreshes, 2);
}

#[test]
fn two_explicit_refusals_of_the_stored_token_ask_to_sign_in() {
    let root = scratch("twice");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let error = ask(&session);
    assert_eq!(error.kind, "signin_expired");
    assert!(!session.connected());
    assert!(saved(&auth_path)["refresh_token"].is_null());
    let sent: Vec<_> = seen
        .lock()
        .unwrap()
        .forms
        .iter()
        .filter(|form| form.get("grant_type").map(String::as_str) == Some("refresh_token"))
        .map(|form| form.get("refresh_token").cloned().unwrap_or_default())
        .collect();
    assert_eq!(sent, ["stale-refresh", "stale-refresh"]);
}

#[test]
fn a_busy_lock_does_not_refresh_or_clear() {
    let root = scratch("locked");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("kept-refresh", 1));
    let _held = hold_lock(&auth_path);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let error = ask(&session);
    assert!(!error.ok);
    assert_eq!(error.kind, "other");
    assert_ne!(error.kind, "signin");
    assert_ne!(error.kind, "signin_expired");
    assert!(session.connected());
    assert_eq!(saved(&auth_path)["refresh_token"], "kept-refresh");
    assert!(seen.lock().unwrap().forms.is_empty());
}

#[test]
fn a_failed_write_after_rotation_does_not_clear_the_saved_token() {
    let root = scratch("write-fail");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("kept-refresh", 1));
    let gate = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Seen {
        token_gate: Some(gate.clone()),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let worker = session.clone();
    let handle = std::thread::spawn(move || ask(&worker));
    wait("jeton en cours", || seen.lock().unwrap().forms.len() == 1);
    let original = fs::read(&auth_path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    }
    gate.store(true, Ordering::SeqCst);
    let outcome = handle.join().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(!outcome.ok);
    assert_eq!(outcome.kind, "other");
    assert_ne!(outcome.kind, "signin_expired");
    assert_eq!(fs::read(&auth_path).unwrap(), original);
    assert!(session.connected());
}

#[test]
fn refresh_success_does_not_overwrite_a_newer_pair() {
    let root = scratch("newer");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let gate = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Seen {
        token_gate: Some(gate.clone()),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let worker = session.clone();
    let handle = std::thread::spawn(move || ask(&worker));
    wait("renouvellement en cours", || {
        seen.lock().unwrap().forms.len() == 1
    });
    let winner = auth("winner-refresh", now().saturating_add(86_400));
    write_auth(&auth_path, &winner);
    let bytes = fs::read(&auth_path).unwrap();
    gate.store(true, Ordering::SeqCst);
    let outcome = handle.join().unwrap();
    assert!(outcome.ok, "{}", outcome.message);
    assert_eq!(fs::read(&auth_path).unwrap(), bytes);
    assert_eq!(saved(&auth_path)["refresh_token"], "winner-refresh");
    assert!(session.connected());
    assert_eq!(session.status()["signin"], "idle");
}

#[test]
fn sign_out_waits_for_the_lock_and_does_not_clear_a_newer_session() {
    let root = scratch("signout");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(
        &auth_path,
        &auth("old-refresh", now().saturating_add(86_400)),
    );
    let held = hold_lock(&auth_path);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen);
    let session = provider(&db, &server);
    let blocked = session.clone();
    let auth_for_worker = auth_path.clone();
    let handle = std::thread::spawn(move || blocked.signout());
    std::thread::sleep(std::time::Duration::from_millis(80));
    write_auth(
        &auth_for_worker,
        &auth("new-refresh", now().saturating_add(86_400)),
    );
    drop(held);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(saved(&auth_path)["refresh_token"], "new-refresh");
    assert!(session.connected());

    let _held = hold_lock(&auth_path);
    let refused = session.signout();
    assert!(refused.is_err());
    assert_eq!(refused.unwrap_err().kind, "other");
    assert_eq!(saved(&auth_path)["refresh_token"], "new-refresh");
    assert!(session.connected());
}

fn callback(redirect: &str, state: &str) -> String {
    ureq::get(&format!(
        "{redirect}?code=good-code&state={state}&client_id={CLIENT}"
    ))
    .call()
    .unwrap()
    .into_string()
    .unwrap()
}

#[test]
fn sign_in_write_requires_the_lock_and_keeps_a_newer_pair() {
    let root = scratch("signin");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    let url = session.start_signin().unwrap();
    let query = form(url.split_once('?').unwrap().1);
    seen.lock().unwrap().nonce = query["nonce"].clone();
    let held = hold_lock(&auth_path);
    let page = callback(&query["redirect_uri"], &query["state"]);
    assert!(page.contains("Connexion non terminée"));
    assert!(saved(&auth_path)["refresh_token"].is_null());
    assert!(!session.connected());
    wait("verrou", || session.status()["signin"] == "failed");
    drop(held);

    let gate = Arc::new(AtomicBool::new(false));
    seen.lock().unwrap().token_gate = Some(gate.clone());
    let url = session.start_signin().unwrap();
    let query = form(url.split_once('?').unwrap().1);
    seen.lock().unwrap().nonce = query["nonce"].clone();
    let redirect = query["redirect_uri"].clone();
    let state = query["state"].clone();
    let handle = std::thread::spawn(move || callback(&redirect, &state));
    wait("échange", || seen.lock().unwrap().forms.len() == 2);
    let winner = auth("signin-winner", now().saturating_add(86_400));
    write_auth(&auth_path, &winner);
    let bytes = fs::read(&auth_path).unwrap();
    gate.store(true, Ordering::SeqCst);
    let page = handle.join().unwrap();
    assert!(page.contains("Connexion réussie"), "{page}");
    assert_eq!(fs::read(&auth_path).unwrap(), bytes);
    assert_eq!(saved(&auth_path)["refresh_token"], "signin-winner");
    assert!(session.connected());
    assert_ne!(session.status()["signin"], "failed");
}

#[test]
fn legacy_refresh_is_merged_when_the_destination_has_no_token() {
    let root = scratch("merge").join("Library/Application Support");
    let legacy = root.join("Grokbot Local/workspace.chatgpt-auth.json");
    write_auth(
        &legacy,
        &auth("legacy-refresh", now().saturating_add(86_400)),
    );
    let imported = root.join("Grokbot/workspace.chatgpt-auth.json");
    write_auth(
        &imported,
        &json!({
            "host_id": "host-dest",
            "client_id": CLIENT,
            "expires_at": 0,
            "scopes": "",
            "saved_at": 1
        }),
    );
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    let session = provider(&root.join("Grokbot/workspace.sqlite"), &server);
    assert!(session.connected());
    assert_eq!(saved(&imported)["host_id"], "host-dest");
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");
    assert!(saved(&legacy)["refresh_token"].is_null());
    assert!(!saved(&legacy).to_string().contains("legacy-refresh"));

    let again = provider(&root.join("Grokbot/workspace.sqlite"), &server);
    assert!(again.connected());
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");
    assert!(saved(&legacy)["refresh_token"].is_null());
}

#[test]
fn destination_refresh_wins_and_the_source_is_tombstoned() {
    let root = scratch("dest-wins").join("Library/Application Support");
    let legacy = root.join("Grokbot Local/workspace.chatgpt-auth.json");
    write_auth(
        &legacy,
        &auth("legacy-refresh", now().saturating_add(86_400)),
    );
    let imported = root.join("Grokbot/workspace.chatgpt-auth.json");
    write_auth(
        &imported,
        &auth("dest-refresh", now().saturating_add(86_400)),
    );
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    let session = provider(&root.join("Grokbot/workspace.sqlite"), &server);
    assert!(session.connected());
    assert_eq!(saved(&imported)["refresh_token"], "dest-refresh");
    assert!(saved(&legacy)["refresh_token"].is_null());
    assert_eq!(server.seen.lock().unwrap().forms.len(), 0);
}

#[test]
fn migration_locks_the_source_before_moving_the_token() {
    let root = scratch("migrate-lock").join("Library/Application Support");
    let legacy = root.join("Grokbot Local/workspace.chatgpt-auth.json");
    write_auth(
        &legacy,
        &auth("legacy-refresh", now().saturating_add(86_400)),
    );
    let imported = root.join("Grokbot/workspace.chatgpt-auth.json");
    let _held = hold_lock(&legacy);
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    let session = provider(&root.join("Grokbot/workspace.sqlite"), &server);
    assert!(!session.connected());
    assert!(!imported.exists());
    assert_eq!(saved(&legacy)["refresh_token"], "legacy-refresh");
    drop(_held);

    assert!(session.connected());
    assert_eq!(saved(&imported)["refresh_token"], "legacy-refresh");
    assert!(saved(&legacy)["refresh_token"].is_null());
}

#[test]
fn a_newer_temporary_is_adopted_on_the_next_startup() {
    let root = scratch("tmp");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("old-refresh", 1));
    let temporary = auth_path.with_extension("json.tmp");
    let mut rotated = auth("rotated-refresh", now().saturating_add(86_400));
    rotated["access_token"] = json!("access-rotated");
    write_auth(&temporary, &rotated);
    let past = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(30))
        .unwrap();
    touch(&auth_path, past);
    touch(&temporary, SystemTime::now());
    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let session = provider(&db, &server);
    assert!(session.connected());
    assert_eq!(saved(&auth_path)["refresh_token"], "rotated-refresh");
    assert_eq!(saved(&auth_path)["access_token"], "access-rotated");
    assert!(!temporary.exists());
    let outcome = ask(&session);
    assert!(outcome.ok, "{}", outcome.message);
    assert_ne!(outcome.kind, "signin_expired");
    assert_ne!(outcome.kind, "signin");
    assert!(seen.lock().unwrap().forms.is_empty());
    assert_eq!(session.status()["signin"], "idle");

    let stale_tmp = auth_path.with_extension("json.tmp");
    write_auth(
        &stale_tmp,
        &auth("ignored-refresh", now().saturating_add(86_400)),
    );
    touch(&auth_path, SystemTime::now());
    touch(
        &stale_tmp,
        SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(30))
            .unwrap(),
    );
    let restarted = provider(&db, &server);
    assert_eq!(saved(&auth_path)["refresh_token"], "rotated-refresh");
    assert!(restarted.connected());
    assert_ne!(ask(&restarted).kind, "signin_expired");
}

#[test]
fn codex_rotation_is_kept_when_the_profile_update_fails() {
    let root = scratch("profile");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.codex-auth.json");
    write_auth(
        &auth_path,
        &json!({
            "host_id": "host-test",
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "account_id": "account-original",
            "access_token": "access-stale",
            "refresh_token": "old-refresh",
            "expires_at": 1,
            "scopes": "",
            "saved_at": 1
        }),
    );
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let session = codex_provider(&db, &server);
    let outcome = ask(&session);
    assert!(!outcome.ok);
    assert_ne!(outcome.kind, "signin_expired");
    assert_ne!(outcome.kind, "signin");
    let refresh = saved(&auth_path)["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(refresh, "old-refresh");
    assert!(session.connected());
    drop(session);
    drop(server);

    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let restarted = codex_provider(&db, &server);
    assert!(restarted.connected());
    let outcome = ask(&restarted);
    assert!(outcome.ok, "{}", outcome.message);
    assert_ne!(outcome.kind, "signin_expired");
    assert!(seen.lock().unwrap().forms.is_empty());
    assert_eq!(saved(&auth_path)["refresh_token"], refresh);
    assert_eq!(restarted.status()["signin"], "idle");
}

fn flock_child() {
    let db = PathBuf::from(std::env::var("EZERBOT_FLOCK_DB").unwrap());
    let base = std::env::var("EZERBOT_FLOCK_BASE").unwrap();
    let result = PathBuf::from(std::env::var("EZERBOT_FLOCK_RESULT").unwrap());
    let endpoints = Endpoints {
        authorize: format!("{base}/authorize"),
        token: format!("{base}/token"),
        revoke: format!("{base}/revoke"),
        jwks: format!("{base}/jwks"),
        issuer: base.clone(),
        api: format!("{base}/v1"),
    };
    let session = Provider::with_endpoints(&db, endpoints, false);
    let outcome = ask(&session);
    let label = if outcome.ok {
        "ok".to_owned()
    } else {
        outcome.kind.to_owned()
    };
    fs::write(result, label).unwrap();
}

#[test]
fn two_processes_do_not_double_rotate() {
    if std::env::var_os("EZERBOT_FLOCK_CHILD").is_some() {
        flock_child();
        return;
    }
    let root = scratch("flock");
    let db = root.join("workspace.sqlite");
    let auth_path = root.join("workspace.chatgpt-auth.json");
    write_auth(&auth_path, &auth("stale-refresh", 1));
    let gate = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Seen {
        token_gate: Some(gate.clone()),
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let result_a = root.join("a.result");
    let result_b = root.join("b.result");
    let spawn = |result: &Path| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "two_processes_do_not_double_rotate"])
            .env("EZERBOT_FLOCK_CHILD", "1")
            .env("EZERBOT_FLOCK_DB", &db)
            .env("EZERBOT_FLOCK_BASE", &server.base)
            .env("EZERBOT_FLOCK_RESULT", result)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut first = spawn(&result_a);
    let mut second = spawn(&result_b);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !result_a.exists() && !result_b.exists() {
        let forms = seen.lock().unwrap().forms.len();
        assert!(forms <= 1, "deux processus ont renouvelé le jeton");
        assert!(
            std::time::Instant::now() < deadline,
            "aucun des deux processus n’a rendu la main"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(seen.lock().unwrap().forms.len(), 1);
    let early = if result_a.exists() {
        fs::read_to_string(&result_a).unwrap()
    } else {
        fs::read_to_string(&result_b).unwrap()
    };
    assert_eq!(early, "other");
    gate.store(true, Ordering::SeqCst);
    let wait_child = |child: &mut std::process::Child| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "processus bloqué");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    wait_child(&mut first);
    wait_child(&mut second);
    let results = [
        fs::read_to_string(&result_a).unwrap(),
        fs::read_to_string(&result_b).unwrap(),
    ];
    assert!(results.contains(&"ok".to_owned()), "{results:?}");
    assert!(results.contains(&"other".to_owned()), "{results:?}");
    assert_eq!(seen.lock().unwrap().forms.len(), 1);
    let refresh = saved(&auth_path)["refresh_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(refresh, "stale-refresh");
    drop(server);

    let seen = Arc::new(Mutex::new(Seen {
        refresh_error: true,
        ..Seen::default()
    }));
    let server = FakeOpenAi::new(seen.clone());
    let restarted = provider(&db, &server);
    assert!(restarted.connected());
    let outcome = ask(&restarted);
    assert!(outcome.ok, "{}", outcome.message);
    assert_ne!(outcome.kind, "signin_expired");
    assert!(seen.lock().unwrap().forms.is_empty());
    assert_eq!(saved(&auth_path)["refresh_token"], refresh);
}
