//! The panel forwards to the real durable send path; no Cocoa or global input.
use grokbot_wrapper::App;
use serde_json::{json, Value};

struct Fixture {
    app: App,
    root: std::path::PathBuf,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("floating");
        let root = guard.to_path_buf();
        Self {
            app: App::open(&root.join("workspace.sqlite")).unwrap(),
            root,
            _dir: guard,
        }
    }
    fn post(&mut self, path: &str, value: Value) -> Value {
        self.app
            .api("POST", path, value.to_string().as_bytes())
            .unwrap()
    }
    fn get(&mut self) -> Value {
        self.app.api("GET", "/api/floating", b"").unwrap()
    }
    fn conversation(&mut self) -> String {
        self.post("/api/conversations", json!({"title":"Panneau"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn error(&mut self, path: &str, value: Value) -> u16 {
        self.app
            .api("POST", path, value.to_string().as_bytes())
            .unwrap_err()
            .status
    }
}
#[test]
fn settings_and_default_scope_survive_restart_and_invalid_settings_do_not_mutate() {
    let mut f = Fixture::new();
    assert_eq!(
        f.get(),
        json!({"shortcut":"cmd+option+j","conversation_id":null,"bot_id":"grokbot"})
    );
    let id = f.conversation();
    f.post(
        "/api/floating/context",
        json!({"conversation_id":id,"bot_id":"grokbot"}),
    );
    f.post(
        "/api/floating/settings",
        json!({"shortcut":"ctrl+option+space"}),
    );
    assert_eq!(
        f.error(
            "/api/floating/settings",
            json!({"shortcut":"capture-all-keys"})
        ),
        400
    );
    assert_eq!(
        f.error(
            "/api/floating/settings",
            json!({"shortcut":"disabled","extra":true})
        ),
        400
    );
    f.app = App::open(&f.root.join("workspace.sqlite")).unwrap();
    assert_eq!(f.get()["conversation_id"], id);
    assert_eq!(f.get()["shortcut"], "ctrl+option+space");
    f.post("/api/floating/settings", json!({"shortcut":"disabled"}));
    assert_eq!(f.get()["shortcut"], "disabled");
}

#[test]
fn explicit_panel_target_and_nonce_share_main_transcript_even_after_selection_changes() {
    let mut f = Fixture::new();
    let id = f.conversation();
    let other = f.conversation();
    f.post(
        "/api/floating/context",
        json!({"conversation_id":other,"bot_id":"grokbot"}),
    );
    let send =
        json!({"conversation_id":id,"message_id":"panel-message","body":"Bonjour du panneau"});
    let first = f.post("/api/floating/send", send.clone());
    let repeated = f.post("/api/floating/send", send);
    assert_eq!(first["conversation"]["id"], id);
    assert_eq!(
        repeated["conversation"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["id"] == "panel-message")
            .count(),
        1
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let all = state["conversations"].as_array().unwrap();
    let target = all.iter().find(|c| c["id"] == id).unwrap();
    assert!(target["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "assistant"));
    assert_eq!(
        all.iter().find(|c| c["id"] == other).unwrap()["messages"],
        json!([])
    );
    assert_eq!(
        f.error(
            "/api/floating/send",
            json!({"conversation_id":id,"message_id":"panel-message","body":"Différent"})
        ),
        409
    );
}

#[test]
fn deleted_missing_foreign_and_task_targets_are_denied_without_redirect_or_send() {
    let mut f = Fixture::new();
    let id = f.conversation();
    f.post(
        "/api/floating/context",
        json!({"conversation_id":id,"bot_id":"grokbot"}),
    );
    f.post("/api/conversations/delete", json!({"id":id}));
    assert_eq!(f.get()["conversation_id"], Value::Null);
    for target in [&id, "missing-target"] {
        assert_eq!(
            f.error(
                "/api/floating/send",
                json!({"conversation_id":target,"message_id":"denied","body":"Pas envoyé"})
            ),
            404
        );
        assert_eq!(
            f.error(
                "/api/floating/context",
                json!({"conversation_id":target,"bot_id":"grokbot"})
            ),
            404
        );
    }
    let task = f.conversation();
    let conn = rusqlite::Connection::open(f.root.join("workspace.sqlite")).unwrap();
    conn.execute(
        "UPDATE wrapper_conversations SET kind='task' WHERE id=?1",
        [&task],
    )
    .unwrap();
    assert_eq!(
        f.error(
            "/api/floating/send",
            json!({"conversation_id":task,"message_id":"denied","body":"Pas envoyé"})
        ),
        404
    );
    assert_eq!(
        f.error(
            "/api/floating/send",
            json!({"conversation_id":"../../foreign","message_id":"denied","body":"Pas envoyé"})
        ),
        400
    );
    assert_eq!(
        f.error(
            "/api/floating/context",
            json!({"conversation_id":null,"bot_id":"absent"})
        ),
        404
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM wrapper_messages WHERE message_id='denied'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn panel_cannot_smuggle_approval_or_external_tool_options_into_a_send() {
    let mut f = Fixture::new();
    let id = f.conversation();
    assert_eq!(
        f.error(
            "/api/floating/send",
            json!({"conversation_id":id,"message_id":"blocked","body":"test","approved":true})
        ),
        400
    );
    assert_eq!(
        f.error(
            "/api/floating/send",
            json!({"conversation_id":id,"message_id":"blocked","body":"test","tool":"run_command"})
        ),
        400
    );
}
