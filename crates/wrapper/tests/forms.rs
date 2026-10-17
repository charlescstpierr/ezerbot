//! Wrapper-only integration tests. Real wake delivery is exercised through the
//! engine's model API here; host-driven resumption belongs to forms_resume.
use grokbot_engine::{
    contracts::{
        with_tx, RunBudgets, RunEventKind, RunKind, SuspendCause, ToolCall, ToolOutcome, Trigger,
        TurnEnd,
    },
    model::{self, Run, RunStatus},
    store,
};
use grokbot_forms::{
    approvals,
    requests::{self, HumanInputRequest, RequestStatus},
};
use grokbot_wrapper::{
    features::{self, forms},
    App,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    app: App,
    conn: Connection,
    directory: PathBuf,
    conversation: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "grokbot-forms-wrapper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("app.sqlite");
        let mut app = App::open(&path).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let fixture = Self {
            app,
            conn,
            directory,
            conversation,
        };
        fixture.start("run", "grokbot", &fixture.conversation);
        fixture
    }
    fn start(&self, id: &str, bot: &str, conversation: &str) {
        with_tx(&self.conn, |conn| {
            model::start(
                conn,
                &Run {
                    run_id: id.into(),
                    bot_id: bot.into(),
                    conversation_id: conversation.into(),
                    trigger: Trigger::UserMessage {
                        message_id: format!("message-{id}"),
                        attempt: 0,
                    },
                    kind: RunKind::Chat,
                    status: RunStatus::Running,
                    task_status: None,
                    turn_end: None,
                    budgets: RunBudgets::default(),
                    started_at: self.app.feature_context().now_ms as i64,
                },
            )
        })
        .unwrap();
    }
    fn call_for(&self, run: &str, call_id: &str, name: &str, args: Value) -> ToolOutcome {
        let run = store::required(&self.conn, run).unwrap();
        let tx = features::write_transaction(&self.conn).unwrap();
        let mut ctx = self.app.feature_context();
        ctx.connection = &tx;
        ctx.run_id = Some(&run.run_id);
        ctx.bot_id = Some(&run.bot_id);
        ctx.conversation_id = Some(&run.conversation_id);
        let result = features::call_tool(
            &ctx,
            &ToolCall {
                call_id: call_id.into(),
                name: name.into(),
                arguments: args,
            },
        )
        .unwrap();
        // Failed outcomes are also committed by the actual run loop. The forms
        // hook must roll its own partial request back, not rely on the probe.
        tx.commit().unwrap();
        result
    }
    fn request(&self, call_id: &str, name: &str, args: Value) -> HumanInputRequest {
        let outcome = self.call_for("run", call_id, name, args);
        let ToolOutcome::Suspend {
            cause: SuspendCause::HumanInput { id },
        } = outcome
        else {
            panic!("expected suspension: {outcome:?}")
        };
        requests::get(&self.conn, &id).unwrap().unwrap()
    }
    fn form(&self) -> HumanInputRequest {
        self.request("form", "request_form", form_args())
    }
    fn submit(&mut self, request: &HumanInputRequest, submission: &str, values: Value) -> Value {
        self.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":submission,"conversation_id":request.route.target_conversation_id,"values":values}))
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap()
    }
    fn suspend(&self, request: &HumanInputRequest) {
        with_tx(
            &self.conn,
            |conn| -> Result<(), grokbot_engine::contracts::Error> {
                let mut run = store::required(conn, &request.run_id)?;
                run.status = if matches!(
                    request.kind,
                    grokbot_forms::requests::InputKind::Approval { .. }
                ) {
                    RunStatus::AwaitingApproval
                } else {
                    RunStatus::AwaitingHumanInput
                };
                store::save(conn, &run)?;
                let event = store::append_event(
                    conn,
                    &request.run_id,
                    RunEventKind::Suspended,
                    &json!({"cause":SuspendCause::HumanInput { id: request.id.clone() }}),
                )?;
                let mut ctx = self.app.feature_context();
                ctx.connection = conn;
                forms::on_run_event(&ctx, &event);
                Ok(())
            },
        )
        .unwrap();
    }
    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
    fn approval(&self, call: &str, arguments: Value) -> HumanInputRequest {
        self.request(
            call,
            "request_approval",
            json!({"tool":"write_file","arguments":arguments}),
        )
    }
    fn decide(&mut self, request: &HumanInputRequest, submission: &str, decision: Value) -> Value {
        self.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"submission_id":submission,"conversation_id":request.route.target_conversation_id,"decision":decision}))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn form_args() -> Value {
    json!({"fields":[{"name":"city","label":"City","field_type":{"kind":"text"},"required":true}]})
}

#[test]
fn submit_writes_resolution_and_pending_wake_atomically() {
    let mut f = Fixture::new();
    let request = f.form();
    f.suspend(&request);
    f.conn.execute_batch("CREATE TRIGGER fail_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'injected wake failure'); END;").unwrap();
    let input = json!({"entry_id":request.entry_id,"submission_id":"submit","conversation_id":f.conversation,"values":{"city":"Paris"}});
    assert!(f
        .app
        .api(
            "POST",
            "/api/forms/submit",
            &serde_json::to_vec(&input).unwrap()
        )
        .is_err());
    assert_eq!(f.count("forms_resolutions"), 0);
    assert_eq!(f.count("engine_run_wakes"), 0);
    assert_eq!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert_eq!(
        store::awaiting_references(&f.conn, "run").unwrap(),
        vec![request.id.clone()]
    );
    f.conn.execute_batch("DROP TRIGGER fail_wake").unwrap();
    let response = f.post("/api/forms/submit", input);
    assert_eq!(response["receipt"]["wake"]["status"], "pending");
    assert_eq!(f.count("forms_resolutions"), 1);
    assert_eq!(f.count("engine_run_wakes"), 1);
    let wake = &store::pending_wakes(&f.conn).unwrap()[0];
    assert_eq!(wake.run_id, "run");
    assert_eq!(
        wake.reference,
        response["receipt"]["resolution_id"].as_str().unwrap()
    );
    assert_eq!(
        store::awaiting_references(&f.conn, "run").unwrap(),
        vec![wake.reference.clone()]
    );
}

#[test]
fn resubmit_returns_same_receipt() {
    let mut f = Fixture::new();
    let request = f.form();
    let first = f.submit(&request, "nonce", json!({"city":"Paris"}));
    let second = f.submit(&request, "nonce", json!({}));
    assert_eq!(first, second);
    assert_eq!(
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap(),
        json!({"city":"Paris"})
    );
    assert_eq!(f.count("forms_resolutions"), 1);
    assert_eq!(f.count("engine_run_wakes"), 1);
    let changed = json!({"entry_id":request.entry_id,"submission_id":"other","conversation_id":f.conversation,"values":{"city":"Lyon"}});
    assert!(f
        .app
        .api(
            "POST",
            "/api/forms/submit",
            &serde_json::to_vec(&changed).unwrap()
        )
        .is_err());
}

#[test]
fn receipt_reflects_engine_delivery_not_just_polling() {
    let mut f = Fixture::new();
    let request = f.form();
    f.suspend(&request);
    let first = f.submit(&request, "nonce", json!({"city":"Paris"}));
    assert_eq!(f.submit(&request, "nonce", json!({})), first);
    with_tx(&f.conn, |conn| model::resume(conn, "run")).unwrap();
    let delivered = f.submit(&request, "nonce", json!({}));
    assert_eq!(
        delivered["receipt"]["resolution_id"],
        first["receipt"]["resolution_id"]
    );
    assert_eq!(delivered["receipt"]["wake"]["status"], "delivered");
    assert_eq!(delivered["receipt"]["wake"]["run_id"], "run");
    let seq = delivered["receipt"]["wake"]["first_event_seq"]
        .as_u64()
        .unwrap();
    assert_eq!(
        store::events(&f.conn, "run", seq - 1).unwrap()[0].kind,
        RunEventKind::Resumed
    );
    assert_eq!(f.count("engine_runs"), 1);
    assert_eq!(f.count("engine_run_wakes"), 1);
}

#[test]
fn secret_value_absent_from_state_export_and_events() {
    let mut f = Fixture::new();
    let request = f.request(
        "secret",
        "request_secret",
        json!({"name":"api_token","purpose":"Authenticate"}),
    );
    assert!(
        matches!(&request.kind,grokbot_forms::requests::InputKind::Secret { name,.. } if name == "API_TOKEN")
    );
    with_tx(&f.conn, |conn| -> Result<(), grokbot_engine::contracts::Error> {
        store::append_event(conn, "run", RunEventKind::ToolCall, &json!({
            "call": {"call_id":"secret","name":"request_secret","arguments":{"name":"api_token","purpose":"Authenticate"}}
        }))?;
        store::append_event(conn, "run", RunEventKind::ToolResult, &json!({
            "call_id":"secret","outcome":ToolOutcome::Suspend { cause:SuspendCause::HumanInput { id:request.id.clone() } }
        }))?;
        Ok(())
    }).unwrap();
    let secret = "line-one\npa\"ss\\word99";
    let response = f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"secret-submit","conversation_id":f.conversation,"value":secret}));
    let resolution = response["receipt"]["resolution_id"].as_str().unwrap();
    assert_eq!(
        requests::resolved_values(&f.conn, resolution).unwrap(),
        json!({"name":"API_TOKEN"})
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    // app.js exportConversation serializes exactly the selected conversation.
    let export = serde_json::to_string(&state["conversations"]).unwrap();
    let all = json!({"response":response,"state":state,"export":export,"events":store::events(&f.conn,"run",0).unwrap()});
    let encoded = serde_json::to_string(&all).unwrap();
    assert!(store::events(&f.conn, "run", 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == RunEventKind::ToolResult));
    assert!(!encoded.contains(secret));
    let escaped = serde_json::to_string(secret).unwrap();
    assert!(!encoded.contains(&escaped[1..escaped.len() - 1]));
    assert!(encoded.contains("API_TOKEN"));
    let metadata = f.app.api("GET", "/api/secrets", b"").unwrap();
    assert_eq!(metadata["secrets"][0]["name"], "API_TOKEN");
    assert!(metadata["secrets"][0].get("value").is_none());
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(vault.get("API_TOKEN").unwrap().as_deref(), Some(secret));
    let retry = f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"secret-submit","conversation_id":f.conversation,"values":{}}));
    assert_eq!(retry, response);
    let replaced = f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"secret-submit","conversation_id":f.conversation,"value":"other-token-value"}));
    assert_eq!(replaced, response);
    assert_eq!(vault.get("API_TOKEN").unwrap().as_deref(), Some(secret));
    let deleted = f.post("/api/secrets/delete", json!({"name":"api_token"}));
    assert_eq!(deleted, json!({"secrets":[]}));
    assert_eq!(vault.get("API_TOKEN").unwrap(), None);
    assert!(!std::fs::read(f.directory.join("app.sqlite"))
        .unwrap()
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    assert!(!std::fs::read(f.directory.join("app.sqlite-wal"))
        .unwrap()
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
}

#[test]
fn approval_for_modified_args_is_refused() {
    let mut f = Fixture::new();
    let args = json!({"path":"safe","content":"hello"});
    let request = f.approval("approval", args.clone());
    let modified = json!({"entry_id":request.entry_id,"submission_id":"allow","conversation_id":f.conversation,"decision":"allow_once","tool":"write_file","arguments":{"path":"unsafe","content":"hello"}});
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/approvals/resolve",
                &serde_json::to_vec(&modified).unwrap()
            )
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(f.count("forms_resolutions"), 0);
    let accepted = f.decide(&request, "allow", json!("allow_once"));
    assert_eq!(accepted["receipt"]["wake"]["status"], "pending");
    assert!(with_tx(&f.conn, |conn| approvals::consume_allow_once(
        conn,
        &request.id,
        "run",
        "write_file",
        &json!({"path":"unsafe","content":"hello"}),
        0
    ))
    .is_err());
    with_tx(&f.conn, |conn| {
        approvals::consume_allow_once(conn, &request.id, "run", "write_file", &args, 0)
    })
    .unwrap();
    assert!(with_tx(&f.conn, |conn| approvals::consume_allow_once(
        conn,
        &request.id,
        "run",
        "write_file",
        &args,
        0
    ))
    .is_err());
}

#[test]
fn child_and_routine_approvals_are_displayed_and_decided_in_main_chat() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"child","name":"Child","color":"cyan","description":"","instructions":""}),
    );
    for (run, kind, parent) in [
        ("child-run", "task", Some(f.conversation.clone())),
        ("routine-run", "routine", None),
    ] {
        let conversation = format!("conversation-{run}");
        f.conn.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) VALUES ('local-workspace',?1,'Worker','child',?2,?3,1,1)",params![conversation,kind,parent]).unwrap();
        f.start(run, "child", &conversation);
        let outcome = f.call_for(
            run,
            "approval",
            "request_approval",
            json!({"tool":"write_file","arguments":{"path":run}}),
        );
        let ToolOutcome::Suspend {
            cause: SuspendCause::HumanInput { id },
        } = outcome
        else {
            panic!("{outcome:?}")
        };
        let request = requests::get(&f.conn, &id).unwrap().unwrap();
        assert_eq!(request.route.target_bot_id, "grokbot");
        assert_eq!(request.route.target_conversation_id, f.conversation);
        assert_eq!(request.route.requesting_bot_id, "child");
        let forged = json!({"entry_id":id,"submission_id":"decide","conversation_id":conversation,"decision":"allow_once"});
        assert_eq!(
            f.app
                .api(
                    "POST",
                    "/api/approvals/resolve",
                    &serde_json::to_vec(&forged).unwrap()
                )
                .unwrap_err()
                .status,
            403
        );
        f.decide(&request, "decide", json!("deny"));
        let wakes = store::pending_wakes(&f.conn).unwrap();
        assert!(wakes.iter().any(|wake| wake.run_id == run));
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        let conversations = state["conversations"].as_array().unwrap();
        let main = conversations
            .iter()
            .find(|c| c["id"] == f.conversation)
            .unwrap();
        assert!(main["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == id && m["kind"] == "approval"));
        let worker = conversations
            .iter()
            .find(|c| c["id"] == conversation)
            .unwrap();
        assert!(worker["messages"].as_array().unwrap().is_empty());
    }
}

#[test]
fn always_permissions_survive_restart_and_auto_review_binds_each_new_action() {
    let mut f = Fixture::new();
    let first = f.approval("first", json!({"path":"one"}));
    f.decide(&first, "always", json!({"always_allow":{"scope":"global"}}));
    f.app = App::open(&f.directory.join("app.sqlite")).unwrap();
    let second = f.approval("second", json!({"path":"two"}));
    assert!(matches!(second.status, RequestStatus::Resolved { .. }));
    assert_eq!(f.count("forms_approval_rules"), 1);
    assert_eq!(f.count("engine_run_wakes"), 2);
    f.suspend(&second); // auto-review preceded suspension; the event hook reconciles the wait.
    assert!(model::ready_to_resume(&f.conn, "run").unwrap());
    with_tx(&f.conn, |conn| {
        approvals::consume_allow_once(
            conn,
            &second.id,
            "run",
            "write_file",
            &json!({"path":"two"}),
            i64::MAX,
        )
    })
    .unwrap();
    assert!(with_tx(&f.conn, |conn| approvals::consume_allow_once(
        conn,
        &second.id,
        "run",
        "write_file",
        &json!({"path":"three"}),
        i64::MAX
    ))
    .is_err());
}

#[test]
fn cancellation_is_per_card_idempotent_and_enqueues_a_wake() {
    let mut f = Fixture::new();
    let request = f.form();
    let other = f.request("other", "request_form", form_args());
    let input = json!({"entry_id":request.entry_id,"submission_id":"cancel","conversation_id":f.conversation});
    let first = f.post("/api/forms/cancel", input.clone());
    assert_eq!(first, f.post("/api/forms/cancel", input));
    assert_eq!(first["receipt"]["wake"]["status"], "pending");
    assert_eq!(
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap(),
        json!("cancelled")
    );
    assert!(matches!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Cancelled {
            resolution_id: Some(_)
        }
    ));
    assert_eq!(
        requests::get(&f.conn, &other.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert_eq!(f.count("engine_run_wakes"), 1);
}

#[test]
fn expiration_is_atomic_wakeable_and_does_not_rewrite_granted_history() {
    let mut f = Fixture::new();
    let expiry = f.app.feature_context().now_ms as i64 + 600_000;
    let pending = f.request(
        "expires",
        "request_approval",
        json!({"tool":"write_file","arguments":{},"expires_at":expiry}),
    );
    f.suspend(&pending);
    f.conn.execute_batch("CREATE TRIGGER fail_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'wake failure'); END").unwrap();
    f.app.tick(expiry as u64);
    assert_eq!(
        requests::get(&f.conn, &pending.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert_eq!(f.count("forms_resolutions"), 0);
    f.conn.execute_batch("DROP TRIGGER fail_wake").unwrap();
    f.app.tick(expiry as u64);
    assert!(matches!(
        requests::get(&f.conn, &pending.id).unwrap().unwrap().status,
        RequestStatus::Expired { .. }
    ));
    assert!(model::ready_to_resume(&f.conn, "run").unwrap());
    let receipt = requests::receipt(&f.conn, &pending.entry_id, "expiry").unwrap();
    assert_eq!(
        requests::resolved_values(&f.conn, &receipt.resolution_id).unwrap(),
        json!("expired")
    );
    f.app.tick(expiry as u64 + 1);
    assert_eq!(f.count("engine_run_wakes"), 1);
    with_tx(&f.conn, |conn| model::resume(conn, "run")).unwrap();
    let granted = f.approval("granted", json!({"path":"one"}));
    f.decide(&granted, "allow", json!("allow_once"));
    f.app.tick(u64::MAX);
    assert!(matches!(
        requests::get(&f.conn, &granted.id).unwrap().unwrap().status,
        RequestStatus::Resolved { .. }
    ));
    with_tx(&f.conn, |conn| {
        approvals::consume_allow_once(
            conn,
            &granted.id,
            "run",
            "write_file",
            &json!({"path":"one"}),
            i64::MAX,
        )
    })
    .unwrap();
}

#[test]
fn invalid_secret_names_and_purposes_are_rejected_before_request_creation() {
    let f = Fixture::new();
    for (name, purpose) in [
        ("PATH", "purpose".to_owned()),
        ("api_token", "😀".repeat(251)),
        ("api_token", " ".into()),
    ] {
        assert!(matches!(
            f.call_for(
                "run",
                name,
                "request_secret",
                json!({"name":name,"purpose":purpose})
            ),
            ToolOutcome::Failed { .. }
        ));
    }
    assert_eq!(f.count("forms_requests"), 0);
    assert_eq!(f.count("wrapper_messages"), 0);
    assert!(matches!(
        f.app
            .invoke_tool(
                "grokbot",
                "request_secret",
                json!({"name":"TOKEN","purpose":"purpose"})
            )
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
}

#[test]
fn redaction_cannot_create_an_unsatisfiable_secret_or_rewrite_an_approval() {
    let f = Fixture::new();
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set("OTHER", "purpose", "API_TOKEN")
        .unwrap();
    assert!(matches!(
        f.call_for(
            "run",
            "secret",
            "request_secret",
            json!({"name":"api_token","purpose":"purpose"})
        ),
        ToolOutcome::Failed { .. }
    ));
    assert!(matches!(
        f.call_for(
            "run",
            "approval",
            "request_approval",
            json!({"tool":"API_TOKEN","arguments":{}})
        ),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(f.count("forms_requests"), 0);
}

#[test]
fn request_replay_keeps_one_card_and_changed_call_is_refused() {
    let f = Fixture::new();
    let first = f.form();
    assert_eq!(f.form().id, first.id);
    assert_eq!(f.count("forms_requests"), 1);
    assert_eq!(f.count("wrapper_messages"), 1);
    assert!(matches!(
        f.call_for("run", "form", "request_form", json!({"fields":[]})),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(f.count("forms_requests"), 1);
}

#[test]
fn request_failure_rolls_back_partial_effect_even_when_caller_commits_failed_outcome() {
    let f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fail_card BEFORE INSERT ON wrapper_messages WHEN NEW.kind='form' BEGIN SELECT RAISE(ABORT,'card failure'); END").unwrap();
    assert!(matches!(
        f.call_for("run", "form", "request_form", form_args()),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(f.count("forms_requests"), 0);
    assert_eq!(f.count("wrapper_forms_calls"), 0);
}

#[test]
fn secret_failure_rolls_back_sql_and_wake_failure_never_overwrites_vault() {
    let mut f = Fixture::new();
    let request = f.request(
        "secret",
        "request_secret",
        json!({"name":"TOKEN","purpose":"purpose"}),
    );
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    vault.set("TOKEN", "purpose", "old-token-value").unwrap();
    for value in [None, Some("short")] {
        let body = json!({"entry_id":request.entry_id,"submission_id":"secret","conversation_id":f.conversation,"value":value});
        assert!(f
            .app
            .api(
                "POST",
                "/api/forms/submit",
                &serde_json::to_vec(&body).unwrap()
            )
            .is_err());
        assert_eq!(f.count("forms_resolutions"), 0);
        assert_eq!(f.count("engine_run_wakes"), 0);
    }
    f.conn.execute_batch("CREATE TRIGGER fail_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'wake failure'); END").unwrap();
    let body = json!({"entry_id":request.entry_id,"submission_id":"secret","conversation_id":f.conversation,"value":"new-token-value"});
    assert!(f
        .app
        .api(
            "POST",
            "/api/forms/submit",
            &serde_json::to_vec(&body).unwrap()
        )
        .is_err());
    assert_eq!(
        vault.get("TOKEN").unwrap().as_deref(),
        Some("old-token-value")
    );
    assert_eq!(f.count("forms_resolutions"), 0);
}

#[test]
fn reserved_submission_ids_and_model_resolution_tools_cannot_approve() {
    let mut f = Fixture::new();
    let request = f.approval("approval", json!({}));
    for submission in ["expiry", "auto:spoof", ""] {
        let body = json!({"entry_id":request.entry_id,"submission_id":submission,"conversation_id":f.conversation,"decision":"allow_once"});
        assert_eq!(
            f.app
                .api(
                    "POST",
                    "/api/approvals/resolve",
                    &serde_json::to_vec(&body).unwrap()
                )
                .unwrap_err()
                .status,
            400
        );
    }
    assert!(f
        .app
        .invoke_tool(
            "grokbot",
            "resolve_approval",
            json!({"entry_id":request.entry_id})
        )
        .is_err());
    assert_eq!(f.count("forms_approval_grants"), 0);
}

#[test]
fn known_secret_is_redacted_before_form_value_persistence() {
    let mut f = Fixture::new();
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    vault
        .set("TOKEN", "purpose", "sensitive-token-value")
        .unwrap();
    let request = f.form();
    let receipt = f.submit(&request, "nonce", json!({"city":"sensitive-token-value"}));
    let resolution = receipt["receipt"]["resolution_id"].as_str().unwrap();
    vault.remove("TOKEN").unwrap();
    let values = requests::resolved_values(&f.conn, resolution).unwrap();
    assert!(!values.to_string().contains("sensitive-token-value"));
}

#[test]
fn stopping_a_run_invalidates_waiting_grants_without_rewriting_consumed_history() {
    let mut f = Fixture::new();
    let used = f.approval("used", json!({"path":"one"}));
    f.decide(&used, "allow-used", json!("allow_once"));
    with_tx(&f.conn, |conn| {
        approvals::consume_allow_once(
            conn,
            &used.id,
            "run",
            "write_file",
            &json!({"path":"one"}),
            0,
        )
    })
    .unwrap();
    let unused = f.approval("unused", json!({"path":"two"}));
    f.decide(&unused, "allow-unused", json!("allow_once"));
    with_tx(
        &f.conn,
        |conn| -> Result<(), grokbot_engine::contracts::Error> {
            model::cancel(conn, "run", TurnEnd::Stopped)?;
            let event = store::events(conn, "run", 0)?.pop().unwrap();
            let mut ctx = f.app.feature_context();
            ctx.connection = conn;
            forms::on_run_event(&ctx, &event);
            Ok(())
        },
    )
    .unwrap();
    assert!(matches!(
        requests::get(&f.conn, &used.id).unwrap().unwrap().status,
        RequestStatus::Resolved { .. }
    ));
    assert!(matches!(
        requests::get(&f.conn, &unused.id).unwrap().unwrap().status,
        RequestStatus::Cancelled { .. }
    ));
}
