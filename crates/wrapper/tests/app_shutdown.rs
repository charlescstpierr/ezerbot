//! Détruire une `App` attend ses fils de fond (#463) : un tour ralenti ne doit rien écrire dans
//! le dossier de l'application une fois l'`App` détruite.
mod support;

use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, Seen, CLIENT};

/// Liste triée (chemin relatif, taille) de tout ce que contient le dossier.
fn snapshot(root: &Path) -> Vec<(String, u64)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path.strip_prefix(root).unwrap().display().to_string();
                out.push((name, meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// État du tour le plus récent, lu hors de l'App (qui est détruite).
fn run_status(db: &Path) -> Option<String> {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.query_row(
        "SELECT status FROM engine_runs ORDER BY rowid DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .ok()
}

#[test]
fn dropping_the_app_waits_for_a_slow_turn_instead_of_writing_into_its_folder_afterwards() {
    let dir = grokbot_test_support::TestDir::new("app-shutdown-test");
    let db = dir.join("workspace.sqlite");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let provider = Provider::with_endpoints(&db, server.endpoints(), false);
    let mut app = App::open_with(&db, provider.clone()).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        dir.join("workspace.chatgpt-auth.json"),
        json!({
            "host_id": "test-host",
            "client_id": CLIENT,
            "access_token": "access-2",
            "refresh_token": "refresh-2",
            "expires_at": now + 3600,
            "scopes": "chatgpt.tokens.use.direct",
        })
        .to_string(),
    )
    .unwrap();
    provider.refresh_models();
    wait("catalogue", || provider.status()["models_ready"] == true);
    provider.wait_for_background_work().unwrap();

    let mut post = |path: &str, body: Value| -> Value {
        app.api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap()
    };
    let conversation = post("/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Le fil de tour attend la réponse du faux serveur, ralentie : il est encore en vol
    // quand le test détruit l'App.
    seen.lock().unwrap().response_delay = Some(Duration::from_millis(700));
    post(
        "/api/send",
        json!({"conversation_id": conversation, "message_id": "slow", "body": "Bonjour"}),
    );

    drop(app);

    // La destruction a attendu : plus aucun fil de fond, et la réponse est déjà écrite.
    let at_drop = snapshot(&dir);
    provider
        .wait_for_background_work_within(Duration::ZERO)
        .expect("l'App détruite ne laisse aucun fil de fond du fournisseur");
    assert_eq!(
        snapshot(&dir),
        at_drop,
        "rien n'est écrit dans le dossier après la destruction de l'App"
    );
    assert_eq!(
        run_status(&db).as_deref(),
        Some("finished"),
        "le tour ralenti est fini quand l'App est détruite"
    );
}
