use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
struct Db(PathBuf);
impl Db {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "ezerbot-demo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn open(&self) -> App {
        App::open(&self.0.join("workspace.sqlite")).unwrap()
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        recordings()
            .lock()
            .unwrap()
            .remove(&self.0.join("workspace.sqlite"));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}
fn chat(app: &mut App) -> String {
    post(app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .into()
}
fn start(app: &mut App, c: &str) -> String {
    post(
        app,
        "/api/demonstrations/start",
        json!({"bot_id":"grokbot","conversation_id":c,"consent":true}),
    )["session"]["id"]
        .as_str()
        .unwrap()
        .into()
}
// One synthetic Mac lease at a time, just like the native helper. Other feature
// tests cannot cause permission requests: debug capture is fixture-only.
use super::super::screen::TEST_MAC as MAC;
#[test]
fn lifecycle_denial_double_start_stop_and_discard() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open();
    let c = chat(&mut app);
    assert_eq!(
        app.api(
            "POST",
            "/api/demonstrations/start",
            json!({"bot_id":"grokbot","conversation_id":c,"consent":false})
                .to_string()
                .as_bytes()
        )
        .unwrap_err()
        .status,
        403
    );
    let id = start(&mut app, &c);
    assert_eq!(
        app.api(
            "POST",
            "/api/demonstrations/start",
            json!({"bot_id":"grokbot","conversation_id":c,"consent":true})
                .to_string()
                .as_bytes()
        )
        .unwrap_err()
        .status,
        409
    );
    assert!(!recordings()
        .lock()
        .unwrap()
        .get(&app.db_path)
        .unwrap()
        .frames
        .is_empty());
    post(&mut app, "/api/demonstrations/stop", json!({"id":id}));
    post(&mut app, "/api/demonstrations/stop", json!({"id":id}));
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "review");
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
    assert!(recordings().lock().unwrap().get(&app.db_path).is_none());
    assert_eq!(owned(&app.connection, &id).unwrap()["summary"], "");
}
#[test]
fn cap_capture_error_restart_and_account_change() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open();
    let c = chat(&mut app);
    let id = start(&mut app, &c);
    let started = owned(&app.connection, &id).unwrap()["started_at"]
        .as_u64()
        .unwrap();
    app.tick(started + CAP + 1);
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "review");
    assert!(owned(&app.connection, &id).unwrap()["notice"]
        .as_str()
        .unwrap()
        .contains("10 minutes"));
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
    let id = start(&mut app, &c);
    post(&mut app, "/api/demonstrations/fake-error", json!({"id":id}));
    assert!(owned(&app.connection, &id).unwrap()["notice"]
        .as_str()
        .unwrap()
        .contains("Erreur de capture"));
    drop(app);
    let mut app = db.open();
    assert!(pending_id(&app.connection).is_none());
    assert!(recordings().lock().unwrap().get(&app.db_path).is_none());
    let id = start(&mut app, &c);
    account_changed(&app.feature_context()).unwrap();
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "discarded");
}
#[test]
fn rolled_back_cancellation_keeps_media_until_a_committed_decision() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open();
    let c = chat(&mut app);
    let id = start(&mut app, &c);
    post(&mut app, "/api/demonstrations/stop", json!({"id":id}));
    app.connection.execute("UPDATE wrapper_demonstrations SET status='drafting',request_id='rollback-request' WHERE id=?1", [&id]).unwrap();
    {
        let tx = write_transaction(&app.connection).unwrap();
        cancelled(&tx, "rollback-request").unwrap();
        assert_eq!(owned(&tx, &id).unwrap()["status"], "discarded");
        // Drop rolls back a hypothetical failure while queuing the engine wake.
    }
    cleanup(&app.feature_context());
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "drafting");
    assert!(!recordings()
        .lock()
        .unwrap()
        .get(&app.db_path)
        .unwrap()
        .frames
        .is_empty());
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
    assert!(!recordings().lock().unwrap().contains_key(&app.db_path));
}
#[test]
fn printable_keys_and_password_text_never_enter_recording() {
    let mut r = Recording {
        id: "test".into(),
        started: 0,
        last_sample: 0,
        frames: vec![],
        events: vec![],
        bytes: 0,
        stopped: false,
        error: None,
    };
    ingest(
        &mut r,
        json!({"inputs":[{"kind":"typing","text":"mot-de-passe","keycode":12},{"kind":"key","key":"a","text":"secret"},{"kind":"private","text":"secret"},{"kind":"key","key":"Entrée","unicode":"secret"}]}),
    );
    let raw = json!(r.events).to_string();
    assert!(!raw.contains("secret"));
    assert!(!raw.contains("mot-de-passe"));
    assert!(!raw.contains("keycode"));
    assert_eq!(r.events.len(), 2);
}
