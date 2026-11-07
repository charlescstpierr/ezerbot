//! Le trousseau devient illisible pendant un tour en vol. Seul le propriétaire du tour décide de
//! son sort : un `tick` qui repasse sur le tour ne le déclare pas échoué pendant que son worker
//! travaille encore.
#![cfg(all(target_os = "linux", debug_assertions))]
mod support;

use grokbot_wrapper::{App, Provider};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Attend les fils de fond du fournisseur avant que le faux serveur et le dossier ne tombent,
/// y compris pendant l'unwinding d'une assertion qui échoue (CODING_STANDARDS.md).
struct Settled(Provider);
impl Drop for Settled {
    fn drop(&mut self) {
        match self.0.wait_for_background_work() {
            Ok(()) => {}
            Err(message) if std::thread::panicking() => eprintln!("{message}"),
            Err(message) => panic!("{message}"),
        }
    }
}

#[test]
fn a_tick_never_fails_a_turn_its_worker_still_owns() {
    let dir = grokbot_test_support::TestDir::new("keyring-turn");
    let db = dir.join("workspace.sqlite");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let server = FakeOpenAi::new(seen.clone());
    let provider = Provider::with_endpoints(&db, server.endpoints(), false);
    let _settled = Settled(provider.clone());
    let mut app = App::open_with(&db, provider.clone()).unwrap();
    let expires = now_ms() / 1000 + 3600;
    std::fs::write(
        db.with_extension("chatgpt-auth.json"),
        json!({"host_id":"test-host","client_id":CLIENT,"access_token":"access-2","refresh_token":"refresh-2","expires_at":expires,"scopes":"chatgpt.tokens.use.direct"}).to_string(),
    )
    .unwrap();
    provider.refresh_models();
    wait("catalogue", || provider.status()["models_ready"] == true);
    provider.wait_for_background_work().unwrap();
    let chat = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let gate = Arc::new(AtomicBool::new(false));
    {
        let mut seen = seen.lock().unwrap();
        let at = seen.bodies.len() + 1;
        seen.response_gate = Some((at, gate.clone()));
        seen.responses
            .push_back(ScriptedResponse::openai_text("Bien reçu", ""));
    }
    app.api(
        "POST",
        "/api/send",
        json!({"conversation_id":chat,"message_id":"held","body":"Bonjour"})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    wait("le tour est en vol derrière la porte", || {
        !seen.lock().unwrap().bodies.is_empty()
    });
    let status = || -> String {
        rusqlite::Connection::open(&db)
            .unwrap()
            .query_row(
                "SELECT status FROM engine_runs ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(status(), "running");
    // Coffre migré, trousseau injoignable : le même état qu'un trousseau verrouillé.
    let marker = db.with_extension("secrets.keyring");
    std::fs::write(&marker, b"").unwrap();
    for _ in 0..5 {
        app.tick(now_ms());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        status(),
        "running",
        "a tick failed the turn while its worker still owned it"
    );
    std::fs::remove_file(&marker).unwrap();
    gate.store(true, Ordering::SeqCst);
    wait("fin du tour", || status() != "running");
    assert_eq!(status(), "finished");
    drop(app);
    provider.wait_for_background_work().unwrap();
}
