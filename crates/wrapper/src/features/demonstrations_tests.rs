use super::*;
struct Db(PathBuf, #[allow(dead_code)] grokbot_test_support::TestDir);
impl Db {
    fn new() -> Self {
        let directory = grokbot_test_support::TestDir::new("ezerbot-demo");
        Self(directory.to_path_buf(), directory)
    }
    fn credentials(&self) -> PathBuf {
        self.0.join("workspace.chatgpt-auth.json")
    }
    fn connect(&self) {
        std::fs::write(self.credentials(), json!({"host_id":"fixture","client_id":"fixture-client","access_token":"fake-access","refresh_token":"fake-refresh","subject":"fixture-subject","account_id":"fixture-account","expires_at":crate::now()/1000+86400,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    }
    fn open_disconnected(&self) -> App {
        let path = self.0.join("workspace.sqlite");
        // Never send the fixture bearer to a real provider, including catalogue
        // refreshes caused by /api/state. Port 9 refuses any optional request.
        let provider = crate::Provider::with_endpoints(
            &path,
            crate::Endpoints {
                api: "http://127.0.0.1:9/v1".into(),
                ..crate::Endpoints::default()
            },
            false,
        );
        App::open_with(&path, provider).unwrap()
    }
    fn open(&self) -> App {
        if !self.credentials().exists() {
            self.connect();
        }
        self.open_disconnected()
    }
}
impl Drop for Db {
    fn drop(&mut self) {
        recordings()
            .lock()
            .unwrap()
            .remove(&self.0.join("workspace.sqlite"));
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
fn disconnected_start_is_refused_without_recording_and_first_connection_can_start() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open_disconnected();
    let c = chat(&mut app);
    let result = app
        .api(
            "POST",
            "/api/demonstrations/start",
            json!({"bot_id":"grokbot","conversation_id":c,"consent":true})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(result.status, 409);
    assert!(result.message.contains("avant de montrer une tâche"));
    assert_eq!(
        app.connection
            .query_row("SELECT count(*) FROM wrapper_demonstrations", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert!(!recordings().lock().unwrap().contains_key(&app.db_path));
    db.connect();
    let id = start(&mut app, &c);
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "recording");
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
}
#[test]
fn mutable_profile_fields_keep_the_recording_but_another_account_discards_it() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open();
    let c = chat(&mut app);
    let id = start(&mut app, &c);
    post(&mut app, "/api/demonstrations/stop", json!({"id":id}));
    let mut credentials: Value =
        serde_json::from_slice(&std::fs::read(db.credentials()).unwrap()).unwrap();
    credentials["email"] = json!("profil-modifie@exemple.test");
    credentials["client_id"] = json!("client-complete-apres-connexion");
    std::fs::write(db.credentials(), credentials.to_string()).unwrap();
    app.tick(crate::now());
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "review");
    credentials["account_id"] = json!("autre-compte");
    std::fs::write(db.credentials(), credentials.to_string()).unwrap();
    app.tick(crate::now());
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "discarded");
}
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
    let summary = app
        .api("GET", &format!("/api/demonstrations/summary?id={id}"), b"")
        .unwrap();
    assert!(
        summary["demonstration"]["memory_image_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(summary["demonstration"]["saved_image_count"], 0);
    assert!(!summary["demonstration"]["summary"]
        .as_str()
        .unwrap()
        .contains("pixels"));
    assert!(summary["demonstration"]["notice"]
        .as_str()
        .unwrap()
        .starts_with("Ajoutez"));
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
    assert!(recordings().lock().unwrap().get(&app.db_path).is_none());
    assert_eq!(owned(&app.connection, &id).unwrap()["summary"], "");
    // Native overlay Stop reaches tick rather than the HTTP Stop route. Model
    // that signal with the fixture, without a real input observer or capture.
    let id = start(&mut app, &c);
    recordings()
        .lock()
        .unwrap()
        .get_mut(&app.db_path)
        .unwrap()
        .stopped = true;
    app.tick(crate::now());
    assert_eq!(
        owned(&app.connection, &id).unwrap()["notice"],
        REVIEW_NOTICE
    );
    post(&mut app, "/api/demonstrations/discard", json!({"id":id}));
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
    assert!(state(&app.feature_context())["session"]["capture_error"]
        .as_str()
        .unwrap()
        .contains("Erreur de capture"));
    assert!(owned(&app.connection, &id).unwrap()["notice"]
        .as_str()
        .unwrap()
        .contains("Erreur de capture"));
    assert!(owned(&app.connection, &id).unwrap()["summary"]
        .as_str()
        .unwrap()
        .contains("la démonstration peut être incomplète"));
    drop(app);
    let mut app = db.open();
    assert!(pending_id(&app.connection).is_none());
    assert!(recordings().lock().unwrap().get(&app.db_path).is_none());
    let id = start(&mut app, &c);
    account_changed(&app.feature_context()).unwrap();
    assert_eq!(owned(&app.connection, &id).unwrap()["status"], "discarded");
}
#[test]
fn restart_cancellation_notice_is_durable_until_acknowledged_once() {
    let _m = MAC.lock().unwrap();
    let db = Db::new();
    let mut app = db.open();
    let c = chat(&mut app);
    let id = start(&mut app, &c);
    drop(app);
    let mut app = db.open();
    let restarted = state(&app.feature_context());
    assert!(restarted["session"].is_null());
    assert_eq!(restarted["pending_notice"]["id"], id);
    assert!(restarted["pending_notice"]["text"]
        .as_str()
        .unwrap()
        .contains("redémarrage"));
    post(
        &mut app,
        "/api/demonstrations/acknowledge",
        json!({"id":id}),
    );
    assert!(state(&app.feature_context())["pending_notice"].is_null());
    drop(app);
    let app = db.open();
    assert!(state(&app.feature_context())["pending_notice"].is_null());
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
