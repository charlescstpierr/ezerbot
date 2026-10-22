//! Private iPhone boundary: real App dispatcher, no Tailscale, account or push.
use grokbot_wrapper::{features::iphone, ApiResponse, App};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    app: App,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "iphone-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("app.sqlite");
        Self {
            app: App::open(&path).unwrap(),
            path,
        }
    }
    fn post(&mut self, path: &str, value: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&value).unwrap())
            .unwrap()
    }
    fn remote(
        &mut self,
        method: &str,
        path: &str,
        value: Value,
        cookie: Option<&str>,
    ) -> Result<Value, grokbot_wrapper::ApiError> {
        let headers = cookie
            .map(|token| vec![("Cookie".into(), format!("ezerbot_device={token}"))])
            .unwrap_or_default();
        match iphone::remote(
            &mut self.app,
            method,
            path,
            &serde_json::to_vec(&value).unwrap(),
            &headers,
        )? {
            ApiResponse::Json(value) => Ok(value),
            _ => panic!("expected JSON"),
        }
    }
    fn pair(&mut self) -> (String, String) {
        self.post("/api/iphone/enabled", json!({"enabled":true}));
        let code = self.post("/api/iphone/code", json!({}))["code"].clone();
        let receipt = self
            .remote(
                "POST",
                "/api/iphone/claim",
                json!({"code":code,"name":"iPhone de test"}),
                None,
            )
            .unwrap()["receipt"]
            .clone();
        let status = self.app.api("GET", "/api/iphone/status", b"").unwrap();
        let id = status["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["status"] == "pending")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.post("/api/iphone/confirm", json!({"id":id,"allow":true}));
        let token = self
            .remote(
                "POST",
                "/api/iphone/exchange",
                json!({"receipt":receipt}),
                None,
            )
            .unwrap()["credential"]
            .as_str()
            .unwrap()
            .to_owned();
        (id, token)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
#[test]
fn presence_reports_only_the_authenticated_devices_push_state() {
    let mut f = Fixture::new();
    let (id, token) = f.pair();
    let body = json!({"conversation_id":null,"visible":false});
    assert_eq!(
        f.remote("POST", "/api/iphone/presence", body.clone(), Some(&token))
            .unwrap(),
        json!({"ok":true,"push":false})
    );
    f.app
        .feature_context()
        .connection
        .execute(
            "UPDATE iphone_devices SET subscription='{}' WHERE id=?1",
            [id],
        )
        .unwrap();
    let state = f
        .remote("POST", "/api/iphone/presence", body, Some(&token))
        .unwrap();
    assert_eq!(state, json!({"ok":true,"push":true}));
    f.remote(
        "POST",
        "/api/iphone/push",
        json!({"subscription":null}),
        Some(&token),
    )
    .unwrap();
    assert_eq!(
        f.remote(
            "POST",
            "/api/iphone/presence",
            json!({"conversation_id":null,"visible":false}),
            Some(&token)
        )
        .unwrap()["push"],
        false
    );
}
#[test]
fn off_by_default_confirmation_refusal_restart_and_revocation() {
    let mut f = Fixture::new();
    assert!(!iphone::enabled(&f.app));
    assert_eq!(
        f.remote("GET", "/api/state", json!({}), None)
            .unwrap_err()
            .status,
        403
    );
    f.post("/api/iphone/enabled", json!({"enabled":true}));
    assert_eq!(
        f.remote("GET", "/api/state", json!({}), None)
            .unwrap_err()
            .status,
        401
    );
    let code = f.post("/api/iphone/code", json!({}))["code"].clone();
    let claim = f
        .remote(
            "POST",
            "/api/iphone/claim",
            json!({"code":code,"name":"Appareil refusé"}),
            None,
        )
        .unwrap();
    let exchange = json!({"receipt":claim["receipt"]});
    assert_eq!(
        f.remote("POST", "/api/iphone/exchange", exchange.clone(), None)
            .unwrap()["pending"],
        true
    );
    assert_eq!(
        f.remote(
            "POST",
            "/api/iphone/claim",
            json!({"code":code,"name":"Rejeu"}),
            None
        )
        .unwrap_err()
        .status,
        403
    );
    let status = f.app.api("GET", "/api/iphone/status", b"").unwrap();
    let denied = status["devices"][0]["id"].clone();
    f.post("/api/iphone/confirm", json!({"id":denied,"allow":false}));
    assert_eq!(
        f.remote("POST", "/api/iphone/exchange", exchange, None)
            .unwrap_err()
            .status,
        403
    );
    let (id, token) = f.pair();
    let state = f
        .remote("GET", "/api/state", json!({}), Some(&token))
        .unwrap();
    assert_eq!(state["iphone"], json!({"remote":true}));
    assert!(state["iphone"].get("devices").is_none());
    assert_eq!(
        f.remote("POST", "/api/iphone/revoke", json!({"id":id}), Some(&token))
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        f.remote("POST", "/api/screen/grant", json!({}), Some(&token))
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        f.remote("POST", "/api/provider/signin", json!({}), Some(&token))
            .unwrap_err()
            .status,
        403
    );
    // Exactly the original approval dispatcher: invalid/missing requests are NOT
    // granted, and a secret cannot be submitted without its existing card.
    for path in ["/api/approvals/resolve", "/api/forms/submit"] {
        assert!(f.remote("POST", path, json!({}), Some(&token)).is_err());
    }
    let replacement = App::open(&f.path).unwrap();
    f.app = replacement;
    assert!(iphone::enabled(&f.app));
    assert!(f
        .remote("GET", "/api/state", json!({}), Some(&token))
        .is_ok());
    let local = f.app.api("GET", "/api/iphone/status", b"").unwrap();
    assert_eq!(local["devices"][0]["name"], "iPhone de test");
    assert_eq!(local["devices"][0]["connected"], true);
    f.post("/api/iphone/revoke", json!({"id":id}));
    assert_eq!(
        f.remote("GET", "/api/state", json!({}), Some(&token))
            .unwrap_err()
            .status,
        401
    );
}
#[test]
fn codes_expire_and_attempt_limits_survive_restart_and_new_codes() {
    let mut f = Fixture::new();
    f.post("/api/iphone/enabled", json!({"enabled":true}));
    let code = f.post("/api/iphone/code", json!({}))["code"].clone();
    f.app
        .feature_context()
        .connection
        .execute("UPDATE iphone_settings SET expires=0", [])
        .unwrap();
    assert_eq!(
        f.remote(
            "POST",
            "/api/iphone/claim",
            json!({"code":code,"name":"Test"}),
            None
        )
        .unwrap_err()
        .status,
        403
    );
    for _ in 0..4 {
        assert_eq!(
            f.remote(
                "POST",
                "/api/iphone/claim",
                json!({"code":"invalid","name":"Test"}),
                None
            )
            .unwrap_err()
            .status,
            403
        );
    }
    let replacement = App::open(&f.path).unwrap();
    f.app = replacement;
    let code = f.post("/api/iphone/code", json!({}))["code"].clone();
    assert_eq!(
        f.remote(
            "POST",
            "/api/iphone/claim",
            json!({"code":code,"name":"Test"}),
            None
        )
        .unwrap_err()
        .status,
        429
    );
    f.app
        .feature_context()
        .connection
        .execute("UPDATE iphone_settings SET attempt_window=0", [])
        .unwrap();
    let claim = f
        .remote(
            "POST",
            "/api/iphone/claim",
            json!({"code":code,"name":"Test"}),
            None,
        )
        .unwrap();
    f.app
        .feature_context()
        .connection
        .execute("UPDATE iphone_devices SET expires=0", [])
        .unwrap();
    assert_eq!(
        f.remote(
            "POST",
            "/api/iphone/exchange",
            json!({"receipt":claim["receipt"]}),
            None
        )
        .unwrap_err()
        .status,
        403
    );
}
#[test]
fn port_conflict_refuses_publication_before_any_cli_is_run() {
    let mut f = Fixture::new();
    f.post("/api/iphone/enabled", json!({"enabled":true}));
    iphone::listener_error(&f.app, Some("Port occupé par une autre application"));
    assert_eq!(
        f.app
            .api("POST", "/api/iphone/serve", br#"{"start":true}"#)
            .unwrap_err()
            .status,
        409
    );
}
#[test]
fn remote_origin_and_duplicate_headers_cannot_cross_the_gate() {
    let headers = |origin: &str| {
        vec![
            ("Host".into(), "mac.tail123.ts.net".into()),
            ("Origin".into(), origin.into()),
            ("X-Grokbot-Request".into(), "local".into()),
            ("Content-Type".into(), "application/json".into()),
        ]
    };
    assert!(iphone::validate_headers("POST", &headers("https://mac.tail123.ts.net")).is_ok());
    assert_eq!(
        iphone::validate_headers("POST", &headers("https://evil.test"))
            .unwrap_err()
            .status,
        403
    );
    let mut duplicate = headers("https://mac.tail123.ts.net");
    duplicate.push(("Host".into(), "evil.test".into()));
    assert!(iphone::validate_headers("GET", &duplicate).is_err());
    assert!(
        iphone::validate_headers("POST", &[("Host".into(), "mac.tail123.ts.net".into())]).is_err()
    );
    assert!(iphone::validate_headers(
        "GET",
        &[("Host".into(), "mac.tail123.ts.net.evil.test".into())]
    )
    .is_err());
}

#[test]
fn remote_decisions_use_the_same_digest_bound_approval_gate() {
    use grokbot_engine::{
        contracts::{ApprovalCheck, RunBudgets, RunKind, ToolCall, Trigger},
        model::{self, Run, RunStatus},
    };
    use grokbot_forms::requests;
    use grokbot_wrapper::features::{self, forms, Context};
    let mut f = Fixture::new();
    let (_, token) = f.pair();
    let conversation = f.post("/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let run = Run {
        run_id: "remote-approval".into(),
        bot_id: "grokbot".into(),
        conversation_id: conversation.clone(),
        trigger: Trigger::UserMessage {
            message_id: "remote-message".into(),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Running,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: 1,
    };
    let call = ToolCall {
        call_id: "terminal-test".into(),
        name: "shell".into(),
        arguments: json!({"command":"test-only-no-execution"}),
    };
    let tx = features::write_transaction(&conn).unwrap();
    model::start(&tx, &run).unwrap();
    let ctx = Context {
        connection: &tx,
        bot_id: Some("grokbot"),
        conversation_id: Some(&conversation),
        run_id: Some(&run.run_id),
        ..f.app.feature_context()
    };
    let cause = forms::request_tool_approval(&ctx, &run, &call).unwrap();
    let grokbot_engine::contracts::SuspendCause::HumanInput { id } = cause else {
        panic!()
    };
    let request = requests::get(&tx, &id).unwrap().unwrap();
    tx.commit().unwrap();
    let payload = json!({"entry_id":request.entry_id,"submission_id":"phone-test-decision","conversation_id":conversation,"decision":"deny"});
    assert_eq!(
        f.remote("POST", "/api/approvals/resolve", payload.clone(), None)
            .unwrap_err()
            .status,
        401
    );
    f.remote("POST", "/api/approvals/resolve", payload, Some(&token))
        .unwrap();
    let tx = features::write_transaction(&conn).unwrap();
    let ctx = Context {
        connection: &tx,
        bot_id: Some("grokbot"),
        conversation_id: Some(&conversation),
        run_id: Some(&run.run_id),
        ..f.app.feature_context()
    };
    assert!(matches!(
        forms::check_tool_approval(&ctx, &run, &call).unwrap(),
        ApprovalCheck::Denied { .. }
    ));
    tx.commit().unwrap();
}

#[test]
fn paired_phone_can_stop_a_background_task() {
    let mut f = Fixture::new();
    let (_, token) = f.pair();
    f.app.feature_context().connection.execute(
        "INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('run','call','en cours')", [],
    ).unwrap();
    let body = json!({"run_id":"run","call_id":"call"});
    assert_eq!(
        f.remote("POST", "/api/local/tasks/stop", body.clone(), None)
            .unwrap_err()
            .status,
        401
    );
    let result = f
        .remote("POST", "/api/local/tasks/stop", body, Some(&token))
        .unwrap();
    assert_eq!(result["status"], "arrêtée");
}
