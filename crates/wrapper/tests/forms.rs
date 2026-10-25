//! Targeted wrapper integration. Suspensions and resumptions use the real durable
//! run loop with production approval adapters, not hand-written lifecycle states.
//! ModelRequest capture proves this boundary; fake HTTP/provider proof is forms_resume.
use grokbot_engine::{
    contracts::{
        with_tx, ApprovalCheck, Error, ModelEvent, ModelGateway, ModelRequest,
        Result as EngineResult, RunBudgets, RunEvent, RunEventKind, RunKind, SuspendCause,
        TaskStatus, ToolCall, ToolOutcome, ToolSpec, Trigger, TurnEnd,
    },
    model::{self, Run, RunStatus},
    run_loop::{self, RunProgress, RunServices},
    store,
};
use grokbot_forms::{
    approvals,
    requests::{self, HumanInputRequest, RequestStatus},
};
use grokbot_wrapper::{
    features::{self, forms, Context},
    ApiError, App,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{collections::VecDeque, path::PathBuf, sync::Mutex};

#[path = "support/skill_proposals_cases.rs"]
mod skill_proposals_cases;

// This SQL-only fake must not collide with the real filesystem tool/preview.
const GUARDED_TOOL: &str = "test_protected_write";
const FREE_TOOL: &str = "test_free_write";
struct Fixture {
    app: App,
    conn: Connection,
    directory: PathBuf,
    conversation: String,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("grokbot-forms-wrapper");
        let directory = guard.to_path_buf();
        let path = directory.join("app.sqlite");
        let mut app = App::open(&path).unwrap();
        app.api("POST", "/api/yolo/mode", br#"{"mode":"ask"}"#)
            .unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        conn.execute_batch("CREATE TABLE test_protected_effects(call_id TEXT PRIMARY KEY, arguments TEXT NOT NULL)").unwrap();
        let fixture = Self {
            app,
            conn,
            directory,
            conversation,
            _dir: guard,
        };
        fixture.start("run", "grokbot", &fixture.conversation);
        fixture
    }
    fn start(&self, id: &str, bot: &str, conversation: &str) {
        self.start_kind(id, bot, conversation, RunKind::Chat);
    }
    fn start_kind(&self, id: &str, bot: &str, conversation: &str, kind: RunKind) {
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
                    kind,
                    status: RunStatus::Running,
                    task_status: (kind == RunKind::Task).then_some(TaskStatus::Working),
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
        let ctx = tool_context(&self.app, &tx, &run);
        let result = features::call_tool(
            &ctx,
            &ToolCall {
                call_id: call_id.into(),
                name: name.into(),
                arguments: args,
            },
        )
        .unwrap();
        // The engine also commits Failed outcomes. Hooks must undo partial writes.
        tx.commit().unwrap();
        result
    }
    fn request(&self, call_id: &str, name: &str, args: Value) -> HumanInputRequest {
        let outcome = self.call_for("run", call_id, name, args);
        let ToolOutcome::Suspend {
            cause: SuspendCause::HumanInput { id },
        } = outcome
        else {
            panic!("{outcome:?}")
        };
        requests::get(&self.conn, &id).unwrap().unwrap()
    }
    fn form(&self) -> HumanInputRequest {
        self.request("form", "request_form", form_args())
    }
    fn approval_for(&self, run: &str, call_id: &str, args: Value) -> HumanInputRequest {
        self.approval_call(run, &guarded_call(call_id, args))
    }
    fn approval_call(&self, run: &str, call: &ToolCall) -> HumanInputRequest {
        let run = store::required(&self.conn, run).unwrap();
        let tx = features::write_transaction(&self.conn).unwrap();
        let ctx = tool_context(&self.app, &tx, &run);
        let cause = forms::request_tool_approval(&ctx, &run, call).unwrap();
        let SuspendCause::HumanInput { id } = cause else {
            panic!("not human input")
        };
        let request = requests::get(&tx, &id).unwrap().unwrap();
        tx.commit().unwrap();
        request
    }
    fn approval(&self, call: &str, args: Value) -> HumanInputRequest {
        self.approval_for("run", call, args)
    }
    fn check(&self, call: &str, args: Value) -> ApprovalCheck {
        with_tx(&self.conn, |conn| -> Result<_, ApiError> {
            let run = store::required(conn, "run")?;
            forms::check_tool_approval(
                &tool_context(&self.app, conn, &run),
                &run,
                &guarded_call(call, args.clone()),
            )
        })
        .unwrap()
    }
    fn drive(&self, run: &str, gateway: &ScriptedGateway) -> RunProgress {
        run_loop::run(
            &self.conn,
            gateway,
            &mut Services {
                app: &self.app,
                force_first_ask: false,
            },
            run,
        )
        .unwrap()
    }
    fn suspended_request(
        &self,
        run: &str,
        call_id: &str,
        name: &str,
        args: Value,
    ) -> HumanInputRequest {
        let progress = self.drive(run, &ScriptedGateway::call(call_id, name, args));
        let RunProgress::Suspended(SuspendCause::HumanInput { id }) = progress else {
            panic!("{progress:?}")
        };
        requests::get(&self.conn, &id).unwrap().unwrap()
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap()
    }
    fn submit(&mut self, request: &HumanInputRequest, nonce: &str, values: Value) -> Value {
        self.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":nonce,"conversation_id":request.route.target_conversation_id,"values":values}))
    }
    fn decide(&mut self, request: &HumanInputRequest, nonce: &str, decision: Value) -> Value {
        self.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"submission_id":nonce,"conversation_id":request.route.target_conversation_id,"decision":decision}))
    }
    fn cancel(&mut self, request: &HumanInputRequest, nonce: &str) -> Value {
        self.post("/api/forms/cancel",json!({"entry_id":request.entry_id,"submission_id":nonce,"conversation_id":request.route.target_conversation_id}))
    }
    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }
}
fn form_args() -> Value {
    json!({"fields":[{"name":"city","label":"City","field_type":{"kind":"text"},"required":true}]})
}
fn guarded_call(id: &str, args: Value) -> ToolCall {
    ToolCall {
        call_id: id.into(),
        name: GUARDED_TOOL.into(),
        arguments: args,
    }
}
fn tool_context<'a>(app: &'a App, conn: &'a Connection, run: &'a Run) -> Context<'a> {
    Context {
        connection: conn,
        bot_id: Some(&run.bot_id),
        conversation_id: Some(&run.conversation_id),
        run_id: Some(&run.run_id),
        ..app.feature_context()
    }
}
fn engine_error(error: ApiError) -> Error {
    Error::InvalidInput(error.to_string())
}

struct Services<'a> {
    app: &'a App,
    force_first_ask: bool,
}
impl RunServices for Services<'_> {
    fn context(&mut self, _: &Run) -> EngineResult<ModelRequest> {
        Ok(ModelRequest {
            model: "scripted".into(),
            instructions: "Test contract".into(),
            input: Vec::new(),
            tools: Vec::new(),
            effort: None,
            service_tier: None,
            prompt_cache_key: None,
        })
    }
    fn tools(&self) -> Vec<ToolSpec> {
        let mut tools = features::tools();
        tools.push(ToolSpec {
            name: GUARDED_TOOL.into(),
            description: "Test guarded action".into(),
            schema: json!({"type":"object"}),
            requires_approval: true,
        });
        tools.push(ToolSpec {
            name: FREE_TOOL.into(),
            description: "Test free action".into(),
            schema: json!({"type":"object"}),
            requires_approval: false,
        });
        tools
    }
    fn call_tool(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &ToolCall,
    ) -> EngineResult<ToolOutcome> {
        if call.name == GUARDED_TOOL || call.name == FREE_TOOL {
            conn.execute(
                "INSERT INTO test_protected_effects(call_id,arguments) VALUES (?1,?2)",
                params![call.call_id, call.arguments.to_string()],
            )?;
            return Ok(ToolOutcome::Done {
                output: json!({"written":true}),
            });
        }
        Ok(features::call_tool(&tool_context(self.app, conn, run), call).expect("known tool"))
    }
    fn check_approval(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &ToolCall,
    ) -> EngineResult<ApprovalCheck> {
        // Exercise auto_review resolving between the service's Ask and suspension.
        // Normal production checks below admit a matching standing rule directly.
        if std::mem::take(&mut self.force_first_ask) {
            return Ok(ApprovalCheck::Ask);
        }
        forms::check_tool_approval(&tool_context(self.app, conn, run), run, call)
            .map_err(engine_error)
    }
    fn request_approval(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &ToolCall,
    ) -> EngineResult<SuspendCause> {
        forms::request_tool_approval(&tool_context(self.app, conn, run), run, call)
            .map_err(engine_error)
    }
    fn continue_without_approval(&mut self, conn: &Connection, run: &Run) -> EngineResult<bool> {
        forms::continues_without_approval(conn, run).map_err(engine_error)
    }
    fn is_cancelled(&self, _: &str) -> bool {
        false
    }
    fn on_run_event(&mut self, conn: &Connection, event: &RunEvent) -> EngineResult<()> {
        let ctx = Context {
            connection: conn,
            ..self.app.feature_context()
        };
        forms::on_run_event_checked(&ctx, event).map_err(engine_error)
    }
    fn resolution(
        &mut self,
        conn: &Connection,
        run: &Run,
        call_id: &str,
    ) -> EngineResult<Option<Value>> {
        let id = format!(
            "input_{}",
            approvals::args_digest("human_input", &json!([run.run_id, call_id]))?
        );
        let Some(request) = requests::get(conn, &id)? else {
            return Ok(None);
        };
        match request.status {
            RequestStatus::Resolved { resolution_id } => {
                Ok(Some(requests::resolved_values(conn, &resolution_id)?))
            }
            _ => Ok(None),
        }
    }
}
#[derive(Default)]
struct ScriptedGateway {
    replies: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl ScriptedGateway {
    fn call(id: &str, name: &str, args: Value) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from([vec![
                ModelEvent::ToolCall {
                    call_id: id.into(),
                    name: name.into(),
                    arguments: args,
                },
                ModelEvent::Completed,
            ]])),
            requests: Mutex::default(),
        }
    }
}
impl ScriptedGateway {
    fn calls(calls: Vec<(&str, &str, Value)>) -> Self {
        let mut events: Vec<ModelEvent> = calls
            .into_iter()
            .map(|(id, name, args)| ModelEvent::ToolCall {
                call_id: id.into(),
                name: name.into(),
                arguments: args,
            })
            .collect();
        events.push(ModelEvent::Completed);
        Self {
            replies: Mutex::new(VecDeque::from([events])),
            requests: Mutex::default(),
        }
    }
}
impl ModelGateway for ScriptedGateway {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> EngineResult<()>,
    ) -> EngineResult<()> {
        self.requests.lock().unwrap().push(request.clone());
        let events = self.replies.lock().unwrap().pop_front().unwrap_or_else(|| {
            vec![
                ModelEvent::TextDelta {
                    text: "Done".into(),
                },
                ModelEvent::Completed,
            ]
        });
        for event in events {
            emit(event)?;
        }
        Ok(())
    }
}

#[test]
fn submit_writes_resolution_and_pending_wake_atomically() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "form", "request_form", form_args());
    f.conn.execute_batch("CREATE TRIGGER fail_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'wake failure'); END").unwrap();
    let body = json!({"entry_id":request.entry_id,"submission_id":"submit","conversation_id":f.conversation,"values":{"city":"Paris"}});
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
    assert_eq!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert_eq!(
        store::awaiting_references(&f.conn, "run").unwrap(),
        vec![request.id.clone()]
    );
    f.conn.execute_batch("DROP TRIGGER fail_wake").unwrap();
    let response = f.post("/api/forms/submit", body);
    let wake = &store::pending_wakes(&f.conn).unwrap()[0];
    assert_eq!(wake.run_id, "run");
    assert_eq!(response["receipt"]["wake"]["status"], "pending");
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
    assert_eq!(f.submit(&request, "nonce", json!({})), first);
    assert_eq!(
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap(),
        json!({"city":"Paris"})
    );
    let changed = json!({"entry_id":request.entry_id,"submission_id":"other","conversation_id":f.conversation,"values":{"city":"Lyon"}});
    assert!(f
        .app
        .api(
            "POST",
            "/api/forms/submit",
            &serde_json::to_vec(&changed).unwrap()
        )
        .is_err());
    assert_eq!(f.count("forms_resolutions"), 1);
    assert_eq!(f.count("engine_run_wakes"), 1);
}

#[test]
fn receipt_reflects_engine_delivery_not_just_polling() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "form", "request_form", form_args());
    let first = f.submit(&request, "nonce", json!({"city":"Paris"}));
    assert_eq!(f.submit(&request, "nonce", json!({})), first);
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
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
}

#[test]
fn secret_value_absent_from_state_export_and_events() {
    let mut f = Fixture::new();
    let request = f.suspended_request(
        "run",
        "secret",
        "request_secret",
        json!({"name":"api_token","purpose":"Authenticate"}),
    );
    assert!(
        matches!(&request.kind,grokbot_forms::requests::InputKind::Secret { name,.. } if name == "API_TOKEN")
    );
    let secret = "line-one\npa\"ss\\word99";
    let first = f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"secret-submit","conversation_id":f.conversation,"value":secret}));
    assert_eq!(
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap(),
        json!({"name":"API_TOKEN"})
    );
    let gateway = ScriptedGateway::default();
    assert_eq!(
        f.drive("run", &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    // app.js export serializes this conversation DTO. Events are now produced by
    // the actual run loop; the resumed model input is captured at its gateway.
    // This does NOT claim HTTP/provider-boundary redaction (forms_resume owns it).
    let export = serde_json::to_string(&state["conversations"]).unwrap();
    let events = store::events(&f.conn, "run", 0).unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind == RunEventKind::ToolResult));
    let seen = gateway.requests.lock().unwrap();
    assert!(seen[0]
        .input
        .iter()
        .flat_map(|item| &item.tool_items)
        .any(|item| serde_json::to_value(item)
            .unwrap()
            .to_string()
            .contains("API_TOKEN")));
    let all = json!({"response":first,"state":state,"export":export,"events":events,"model_requests":*seen});
    let encoded = all.to_string();
    let escaped = serde_json::to_string(secret).unwrap();
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains(&escaped[1..escaped.len() - 1]));
    assert!(encoded.contains("API_TOKEN"));
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(vault.path(), f.directory.join("app.secrets.json"));
    assert_eq!(
        vault
            .get_for_bot("grokbot", "API_TOKEN")
            .unwrap()
            .as_deref(),
        Some(secret)
    );
    let retry = f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"secret-submit","conversation_id":f.conversation,"value":"other-token-value"}));
    assert_eq!(
        retry["receipt"]["resolution_id"],
        first["receipt"]["resolution_id"]
    );
    assert_eq!(
        vault
            .get_for_bot("grokbot", "API_TOKEN")
            .unwrap()
            .as_deref(),
        Some(secret)
    );
    assert!(f.app.api("GET", "/api/secrets", b"").unwrap()["secrets"][0]
        .get("value")
        .is_none());
    assert_eq!(
        f.post(
            "/api/secrets/delete",
            json!({"name":"api_token","bot_id":"grokbot"})
        ),
        json!({"secrets":[]})
    );
    let stale = post_result(
        &mut f.app,
        "/api/forms/submit",
        json!({"entry_id":request.entry_id,"submission_id":"secret-again","conversation_id":f.conversation,"value":"autre phrase refusee ici"}),
    )
    .unwrap_err();
    assert_eq!(stale.status, 409);
    assert_eq!(stale.message, "Ce formulaire n’a pas été accepté.");
    assert!(forms::vault(&f.app.feature_context())
        .unwrap()
        .get_for_bot("grokbot", "API_TOKEN")
        .unwrap()
        .is_none());
    for name in ["app.sqlite", "app.sqlite-wal"] {
        assert!(!std::fs::read(f.directory.join(name))
            .unwrap()
            .windows(secret.len())
            .any(|bytes| bytes == secret.as_bytes()));
    }
    assert!(!bytes_contain(
        &f.directory.join("app.secrets.json"),
        "autre phrase refusee ici"
    ));
}

#[test]
fn conversation_secret_card_records_the_name_and_rejects_another_submission() {
    const STALE: &str = "Ce formulaire n’a pas été accepté.";
    const VALUE: &str = "phrase stripe du formulaire";
    const OTHER: &str = "autre phrase du formulaire";
    let mut f = Fixture::new();
    let request = f.suspended_request(
        "run",
        "stripe",
        "request_secret",
        json!({"name":"stripe_key","purpose":"Clé API Stripe"}),
    );
    assert!(matches!(
        &request.kind,
        grokbot_forms::requests::InputKind::Secret { name, purpose }
            if name == "STRIPE_KEY" && purpose == "Clé API Stripe"
    ));
    let pending = f.app.api("GET", "/api/state", b"").unwrap();
    let card = pending["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|conversation| conversation["messages"].as_array().unwrap())
        .find(|message| message["kind"] == "secret")
        .unwrap();
    assert_eq!(card["body"], "");
    assert_eq!(card["payload"]["kind"]["name"], "STRIPE_KEY");
    assert_eq!(card["payload"]["kind"]["purpose"], "Clé API Stripe");
    assert_eq!(card["payload"]["status"]["status"], "pending");
    assert!(!card.to_string().contains(VALUE));

    let first = f.post(
        "/api/forms/submit",
        json!({"entry_id":request.entry_id,"submission_id":"stripe-submit","conversation_id":f.conversation,"value":VALUE}),
    );
    assert_eq!(
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap(),
        json!({"name":"STRIPE_KEY"})
    );
    let gateway = ScriptedGateway::default();
    assert_eq!(
        f.drive("run", &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    let seen = gateway.requests.lock().unwrap();
    let model = seen[0]
        .input
        .iter()
        .flat_map(|item| &item.tool_items)
        .map(|item| serde_json::to_value(item).unwrap().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(model.contains("STRIPE_KEY"));
    assert!(!model.contains(VALUE));
    drop(seen);

    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let resolved = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|conversation| conversation["messages"].as_array().unwrap())
        .find(|message| message["kind"] == "secret")
        .unwrap();
    assert_eq!(resolved["payload"]["status"]["status"], "resolved");
    assert_eq!(resolved["payload"]["kind"]["name"], "STRIPE_KEY");
    assert_eq!(resolved["body"], "");
    let events = store::events(&f.conn, "run", 0).unwrap();
    let encoded = json!({"state":state,"events":events}).to_string();
    assert!(encoded.contains("STRIPE_KEY"));
    assert!(encoded.contains("Clé API Stripe"));
    assert!(!encoded.contains(VALUE));
    let listed = post_result(&mut f.app, "/api/secrets/list", json!({"bot_id":"grokbot"})).unwrap();
    assert_eq!(listed["secrets"][0]["name"], "STRIPE_KEY");
    assert_eq!(listed["secrets"][0]["description"], "Clé API Stripe");
    assert!(listed["secrets"][0].get("value").is_none());
    assert!(!listed.to_string().contains(VALUE));
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(
        vault
            .get_for_bot("grokbot", "STRIPE_KEY")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );

    let replay = f.post(
        "/api/forms/submit",
        json!({"entry_id":request.entry_id,"submission_id":"stripe-submit","conversation_id":f.conversation,"value":OTHER}),
    );
    assert_eq!(
        replay["receipt"]["resolution_id"],
        first["receipt"]["resolution_id"]
    );
    assert_eq!(
        vault
            .get_for_bot("grokbot", "STRIPE_KEY")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );

    let resolutions = f.count("forms_resolutions");
    let stale = post_result(
        &mut f.app,
        "/api/forms/submit",
        json!({"entry_id":request.entry_id,"submission_id":"stripe-autre","conversation_id":f.conversation,"value":OTHER}),
    )
    .unwrap_err();
    assert_eq!(stale.status, 409);
    assert_eq!(stale.message, STALE);
    assert_eq!(f.count("forms_resolutions"), resolutions);
    assert_eq!(
        vault
            .get_for_bot("grokbot", "STRIPE_KEY")
            .unwrap()
            .as_deref(),
        Some(VALUE)
    );
    assert!(!bytes_contain(vault.path(), OTHER));
    for name in ["app.sqlite", "app.sqlite-wal"] {
        assert!(!bytes_contain(&f.directory.join(name), VALUE));
        assert!(!bytes_contain(&f.directory.join(name), OTHER));
    }
}

#[test]
fn approval_for_modified_args_is_refused() {
    let mut f = Fixture::new();
    let args = json!({"path":"safe","content":"hello"});
    let request = f.approval("approval", args.clone());
    let modified = json!({"entry_id":request.entry_id,"submission_id":"allow","conversation_id":f.conversation,"decision":"allow_once","tool":GUARDED_TOOL,"arguments":{"path":"unsafe","content":"hello"}});
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
    f.decide(&request, "allow", json!("allow_once"));
    assert_eq!(
        f.check("approval", json!({"path":"unsafe","content":"hello"})),
        ApprovalCheck::Ask
    );
    assert!(matches!(
        f.check("approval", args.clone()),
        ApprovalCheck::Authorized { .. }
    ));
    assert!(matches!(
        f.check("approval", args),
        ApprovalCheck::Denied { .. }
    ));
}

#[test]
fn guarded_tool_allow_once_resumes_the_original_call_and_executes_once() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "guarded", GUARDED_TOOL, json!({"path":"safe"}));
    assert!(request.id.starts_with("approval:"));
    assert_eq!(
        store::required(&f.conn, "run").unwrap().status,
        RunStatus::AwaitingApproval
    );
    f.decide(&request, "allow", json!("allow_once"));
    assert!(model::ready_to_resume(&f.conn, "run").unwrap());
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.count("test_protected_effects"), 1);
    assert_eq!(f.count("forms_requests"), 1); // no second request for the real action
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.count("test_protected_effects"), 1);
    assert!(store::events(&f.conn, "run", 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == RunEventKind::ToolResult
            && event.payload["call_id"] == "guarded"
            && event.payload["authorized_by"]["request_id"] == request.id));
}

#[test]
fn denied_expired_and_cancelled_approvals_resume_without_effect_or_new_request() {
    for decision in ["deny", "expiry", "cancel"] {
        let mut f = Fixture::new();
        let request = f.suspended_request("run", "guarded", GUARDED_TOOL, json!({"path":"safe"}));
        match decision {
            "deny" => {
                f.decide(&request, "deny", json!("deny"));
            }
            "expiry" => {
                f.app.tick(u64::MAX);
            }
            _ => {
                f.cancel(&request, "cancel");
            }
        }
        assert!(
            model::ready_to_resume(&f.conn, "run").unwrap(),
            "{decision}"
        );
        assert_eq!(
            f.drive("run", &ScriptedGateway::default()),
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(f.count("test_protected_effects"), 0);
        assert_eq!(f.count("forms_requests"), 1);
        assert_eq!(f.count("engine_run_wakes"), 1);
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        assert_eq!(
            state["forms"]["requests"][0]["receipt"]["wake"]["status"],
            "delivered"
        );
    }
}

#[test]
fn state_reports_how_each_approval_was_decided() {
    let mut f = Fixture::new();
    let once = f.approval("once", json!({"path":"one"}));
    let always = f.approval("always", json!({"path":"two"}));
    let waiting = f.approval("waiting", json!({"path":"three"}));
    f.decide(&once, "once", json!("allow_once"));
    f.decide(
        &always,
        "always",
        json!({"always_allow":{"scope":"global"}}),
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let requests = state["forms"]["requests"].as_array().unwrap();
    let decision =
        |id: &str| requests.iter().find(|request| request["id"] == id).unwrap()["decision"].clone();
    assert_eq!(decision(&once.id), json!("allow_once"));
    assert!(decision(&always.id)["always_allow"].is_object());
    assert!(decision(&waiting.id).is_null());
}

#[test]
fn auto_review_before_real_gate_suspension_is_wakeable() {
    let mut f = Fixture::new();
    let first = f.approval("first", json!({"path":"one"}));
    f.decide(&first, "always", json!({"always_allow":{"scope":"global"}}));
    let gateway = ScriptedGateway::call("guarded", GUARDED_TOOL, json!({"path":"two"}));
    let progress = run_loop::run(
        &f.conn,
        &gateway,
        &mut Services {
            app: &f.app,
            force_first_ask: true,
        },
        "run",
    )
    .unwrap();
    assert!(matches!(progress, RunProgress::Suspended(_)));
    assert_eq!(
        store::required(&f.conn, "run").unwrap().status,
        RunStatus::AwaitingApproval
    );
    assert!(model::ready_to_resume(&f.conn, "run").unwrap());
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.count("test_protected_effects"), 1);
    assert_eq!(f.count("forms_requests"), 2);
}

#[test]
fn hidden_worker_inputs_all_route_to_principal_and_wake_the_originating_run() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"child","name":"Child","color":"cyan","description":"","instructions":""}),
    );
    for kind in ["task", "routine"] {
        for name in ["request_form", "request_secret", GUARDED_TOOL] {
            let run = format!("{kind}-{name}");
            let conversation = format!("conversation-{run}");
            f.conn.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) VALUES ('local-workspace',?1,'Worker','child',?2,?3,1,1)",params![conversation,kind,f.conversation]).unwrap();
            f.start(&run, "child", &conversation);
            let args = match name {
                "request_form" => form_args(),
                "request_secret" => json!({"name":"TOKEN","purpose":"purpose"}),
                _ => json!({"path":"safe"}),
            };
            let request = f.suspended_request(&run, &run, name, args);
            assert_eq!(request.route.target_bot_id, "grokbot");
            assert_eq!(request.route.target_conversation_id, f.conversation);
            assert_eq!(request.route.requesting_bot_id, "child");
            if name == GUARDED_TOOL {
                let forged = json!({"entry_id":request.entry_id,"submission_id":"deny","conversation_id":conversation,"decision":"deny"});
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
                f.decide(&request, "deny", json!("deny"));
            } else {
                f.cancel(&request, "cancel");
            }
            assert!(model::ready_to_resume(&f.conn, &run).unwrap());
            assert!(store::pending_wakes(&f.conn)
                .unwrap()
                .iter()
                .any(|wake| wake.run_id == run));
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
                .any(|m| m["id"] == request.id));
            assert!(conversations
                .iter()
                .find(|c| c["id"] == conversation)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .is_empty());
        }
    }
}

#[test]
fn approval_card_shows_exact_action_with_secret_values_redacted_before_persistence() {
    let mut f = Fixture::new();
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    vault.set("TOKEN", "purpose", "secret-token-value").unwrap();
    let path = f.directory.join("preview.txt");
    let args = json!({"path":path,"content":"secret-token-value"});
    let request = f.approval_call(
        "run",
        &ToolCall {
            call_id: "preview".into(),
            name: "write_file".into(),
            arguments: args.clone(),
        },
    );
    assert_eq!(
        match &request.kind {
            grokbot_forms::requests::InputKind::Approval { args_digest, .. } => args_digest.clone(),
            _ => panic!(),
        },
        approvals::args_digest("write_file", &args).unwrap()
    );
    let stored: String = f
        .conn
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&request.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!stored.contains("secret-token-value"));
    vault.remove("TOKEN").unwrap(); // old value must stay redacted after removal
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let display = &state["forms"]["requests"][0]["arguments"];
    assert_eq!(display["path"], json!(path));
    assert_eq!(display["content"], "[...]");
    assert_eq!(
        display["apercu_local"]["extrait"],
        "@@ -0,0 +1,1 @@\n+[...]\n"
    );
    assert_eq!(display["apercu_local"]["lignes_ajoutees"], 1);
    let main = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == f.conversation)
        .unwrap();
    assert_eq!(
        main["messages"][0]["payload"]["arguments"]["path"],
        json!(path)
    );
}

#[test]
fn always_permissions_survive_restart_and_can_be_revoked_without_cancelling_grants() {
    let mut f = Fixture::new();
    let first = f.approval("first", json!({"path":"one"}));
    f.decide(&first, "always", json!({"always_allow":{"scope":"global"}}));
    f.app = App::open(&f.directory.join("app.sqlite")).unwrap();
    // Future cutover may recover an actively running record as Interrupted.
    if store::required(&f.conn, "run").unwrap().status == RunStatus::Interrupted {
        with_tx(&f.conn, |conn| model::resume(conn, "run")).unwrap();
    }
    assert!(matches!(
        f.check("standing", json!({"path":"two"})),
        ApprovalCheck::Authorized { .. }
    ));
    let admitted = f.approval("admitted", json!({"path":"three"}));
    assert!(matches!(admitted.status, RequestStatus::Resolved { .. }));
    let response = f.post("/api/approvals/rules/revoke",json!({"requesting_bot_id":"grokbot","target_bot_id":"grokbot","tool":GUARDED_TOOL,"args_digest":"","scope":"global"}));
    assert_eq!(response, json!({"rules":[]}));
    assert!(matches!(
        f.check("admitted", json!({"path":"three"})),
        ApprovalCheck::Authorized { .. }
    ));
    assert_eq!(
        f.check("future", json!({"path":"four"})),
        ApprovalCheck::Ask
    );
    let future = f.approval("future", json!({"path":"four"}));
    assert_eq!(future.status, RequestStatus::Pending);
    assert_eq!(future.route.target_conversation_id, f.conversation);
}

#[test]
fn never_subject_permission_can_be_retired_for_future_calls() {
    let mut f = Fixture::new();
    let args = json!({"path":"same"});
    let request = f.approval("blocked", args.clone());
    f.decide(&request, "never", json!({"never":{"scope":"subject"}}));
    assert!(matches!(
        f.check("next", args.clone()),
        ApprovalCheck::Denied { .. }
    ));
    let digest = approvals::args_digest(GUARDED_TOOL, &args).unwrap();
    let response = f.post("/api/approvals/rules/revoke",json!({"requesting_bot_id":"grokbot","target_bot_id":"grokbot","tool":GUARDED_TOOL,"args_digest":digest,"scope":"subject"}));
    assert_eq!(response, json!({"rules":[]}));
    assert_eq!(f.check("next", args.clone()), ApprovalCheck::Ask);
    assert_eq!(
        f.approval("next", args.clone()).status,
        RequestStatus::Pending
    );
    // Retirement never rewrites the old per-call refusal.
    assert!(matches!(
        f.check("blocked", args),
        ApprovalCheck::Denied { .. }
    ));
}

#[test]
fn terminal_decision_is_per_call_and_not_overridden_by_later_standing_permission() {
    let mut f = Fixture::new();
    let denied = f.approval("denied", json!({"path":"same"}));
    f.decide(&denied, "deny", json!("deny"));
    let allowed = f.approval("allowed", json!({"path":"same"}));
    assert_ne!(denied.id, allowed.id);
    f.decide(
        &allowed,
        "always",
        json!({"always_allow":{"scope":"global"}}),
    );
    assert!(matches!(
        f.check("denied", json!({"path":"same"})),
        ApprovalCheck::Denied { .. }
    ));
    assert!(matches!(
        f.check("allowed", json!({"path":"same"})),
        ApprovalCheck::Authorized { .. }
    ));
}

#[test]
fn cancellation_is_per_card_and_idempotent() {
    let mut f = Fixture::new();
    let request = f.form();
    let other = f.request("other", "request_form", form_args());
    let first = f.cancel(&request, "cancel");
    assert_eq!(f.cancel(&request, "cancel"), first);
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
fn expiration_rolls_back_on_wake_failure_and_preserves_granted_history() {
    let mut f = Fixture::new();
    let pending = f.suspended_request("run", "guarded", GUARDED_TOOL, json!({"path":"safe"}));
    f.conn.execute_batch("CREATE TRIGGER fail_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'wake failure'); END").unwrap();
    f.app.tick(u64::MAX);
    assert_eq!(
        requests::get(&f.conn, &pending.id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
    assert_eq!(f.count("forms_resolutions"), 0);
    f.conn.execute_batch("DROP TRIGGER fail_wake").unwrap();
    f.app.tick(u64::MAX);
    assert!(model::ready_to_resume(&f.conn, "run").unwrap());
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    f.start("next", "grokbot", &f.conversation);
    let granted = f.approval_for("next", "granted", json!({"path":"one"}));
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
            "next",
            GUARDED_TOOL,
            &json!({"path":"one"}),
            i64::MAX,
        )
    })
    .unwrap();
}

#[test]
fn invalid_secret_names_purposes_and_raw_name_redaction_cannot_create_a_request() {
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
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set("OTHER", "purpose", "API_TOKEN")
        .unwrap();
    assert!(matches!(
        f.call_for(
            "run",
            "redacted",
            "request_secret",
            json!({"name":"api_token","purpose":"purpose"})
        ),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(f.count("forms_requests"), 0);
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
fn request_replay_keeps_one_card_and_rebinding_a_normal_call_is_refused() {
    let f = Fixture::new();
    assert_eq!(f.form().id, f.form().id);
    assert_eq!(f.count("forms_requests"), 1);
    assert_eq!(f.count("wrapper_messages"), 1);
    assert!(matches!(
        f.call_for("run", "form", "request_form", json!({"fields":[]})),
        ToolOutcome::Failed { .. }
    ));
    let first = f.approval("guarded", json!({"path":"one"}));
    assert_eq!(f.approval("guarded", json!({"path":"one"})).id, first.id);
    assert_ne!(f.approval("guarded", json!({"path":"two"})).id, first.id);
}

#[test]
fn request_failure_rolls_back_partial_effect_even_if_failed_outcome_is_committed() {
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
    for nonce in ["expiry", "auto:spoof", ""] {
        let body = json!({"entry_id":request.entry_id,"submission_id":nonce,"conversation_id":f.conversation,"decision":"allow_once"});
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
    assert!(f
        .app
        .invoke_tool(
            "grokbot",
            "request_approval",
            json!({"tool":GUARDED_TOOL,"arguments":{}})
        )
        .is_err());
    assert!(features::tools()
        .iter()
        .all(|tool| tool.name != "request_approval"));
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
    vault.remove("TOKEN").unwrap();
    assert!(!requests::resolved_values(
        &f.conn,
        receipt["receipt"]["resolution_id"].as_str().unwrap()
    )
    .unwrap()
    .to_string()
    .contains("sensitive-token-value"));
}

#[test]
fn checked_event_reaction_propagates_failure_and_rolls_back_cancellation() {
    let mut f = Fixture::new();
    let request = f.approval("approval", json!({"path":"one"}));
    f.decide(&request, "allow", json!("allow_once"));
    f.conn.execute_batch("CREATE TRIGGER fail_invalidation BEFORE UPDATE ON forms_requests WHEN NEW.status='cancelled' BEGIN SELECT RAISE(ABORT,'invalidation failure'); END").unwrap();
    let mut services = Services {
        app: &f.app,
        force_first_ask: false,
    };
    let result = run_loop::with_run_events(&f.conn, &mut services, "run", |conn, _| {
        model::cancel(conn, "run", TurnEnd::Stopped)
    });
    assert!(result.is_err());
    assert_eq!(
        store::required(&f.conn, "run").unwrap().status,
        RunStatus::Running
    );
    assert!(!store::events(&f.conn, "run", 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == RunEventKind::Cancelled));
    f.conn
        .execute_batch("DROP TRIGGER fail_invalidation")
        .unwrap();
    run_loop::with_run_events(&f.conn, &mut services, "run", |conn, _| {
        model::cancel(conn, "run", TurnEnd::Stopped)
    })
    .unwrap();
    assert!(matches!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Cancelled { .. }
    ));
}

fn post_result(app: &mut App, path: &str, body: Value) -> Result<Value, ApiError> {
    app.api("POST", path, &serde_json::to_vec(&body).unwrap())
}

fn bytes_contain(path: &std::path::Path, needle: &str) -> bool {
    std::fs::read(path)
        .map(|bytes| {
            bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        })
        .unwrap_or(false)
}

/// T22 : liste sans valeur, trousseau, formulaire périmé, demande ancienne, bot B.
#[test]
fn bot_secret_panel_lists_metadata_only_keeps_the_vault_and_rejects_stale_forms() {
    const STALE: &str = "Ce formulaire n’a pas été accepté.";
    const PANEL: &str = "panneau-secret-unique-8801";
    const OLD: &str = "ancien-secret-unique-8802";
    const REPLACED: &str = "remplace-secret-unique-8803";
    const RESTARTED: &str = "redemarre-secret-unique-8804";
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"bot-b","name":"Bot B","color":"cyan","description":"","instructions":""}),
    );

    let empty = post_result(&mut f.app, "/api/secrets/list", json!({"bot_id":"grokbot"})).unwrap();
    assert_eq!(empty["secrets"], json!([]));

    let added = post_result(
        &mut f.app,
        "/api/secrets/put",
        json!({"bot_id":"grokbot","name":"cle_panneau","description":"Jeton du panneau","value":PANEL}),
    )
    .unwrap();
    assert!(added["secrets"][0].get("value").is_none());
    assert_eq!(added["secrets"][0]["name"], "CLE_PANNEAU");
    assert_eq!(added["secrets"][0]["description"], "Jeton du panneau");
    assert!(added["secrets"][0]["created_at"].as_i64().unwrap() > 0);
    assert!(
        added["secrets"][0]["updated_at"].as_i64().unwrap()
            >= added["secrets"][0]["created_at"].as_i64().unwrap()
    );
    assert!(!added.to_string().contains(PANEL));
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(
        vault
            .get_for_bot("grokbot", "CLE_PANNEAU")
            .unwrap()
            .as_deref(),
        Some(PANEL)
    );
    let file = std::fs::read_to_string(vault.path()).unwrap_or_default();
    match vault.storage_status().backend {
        "keychain" => assert!(
            !file.contains(PANEL),
            "un trousseau actif ne laisse pas la valeur dans le fichier"
        ),
        "file" | "file_fallback" => assert!(
            file.contains(PANEL),
            "la valeur est dans le coffre qui alimente le trousseau"
        ),
        other => panic!("coffre inattendu: {other}"),
    }
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let export = serde_json::to_string(&state["conversations"]).unwrap();
    let template = post_result(
        &mut f.app,
        "/api/templates/export",
        json!({"bot_id":"grokbot"}),
    )
    .unwrap();
    for haystack in [state.to_string(), export, template.to_string()] {
        assert!(!haystack.contains(PANEL));
    }
    for name in ["app.sqlite", "app.sqlite-wal"] {
        assert!(!bytes_contain(&f.directory.join(name), PANEL));
    }
    let other = post_result(&mut f.app, "/api/secrets/list", json!({"bot_id":"bot-b"})).unwrap();
    assert_eq!(other["secrets"], json!([]));
    assert!(!other.to_string().contains("CLE_PANNEAU"));
    let removed = post_result(
        &mut f.app,
        "/api/secrets/delete",
        json!({"bot_id":"grokbot","name":"cle_panneau"}),
    )
    .unwrap();
    assert_eq!(removed["secrets"], json!([]));
    assert!(forms::vault(&f.app.feature_context())
        .unwrap()
        .get_for_bot("grokbot", "CLE_PANNEAU")
        .unwrap()
        .is_none());

    let older = f.request(
        "secret-old",
        "request_secret",
        json!({"name":"jeton_ancien","purpose":"Ancien jeton"}),
    );
    let now = f.app.feature_context().now_ms as i64;
    let shifted = now - 31 * 60 * 1000;
    f.conn
        .execute(
            "UPDATE forms_requests SET created_at=?1 WHERE id=?2",
            params![shifted, older.id],
        )
        .unwrap();
    let stored: i64 = f
        .conn
        .query_row(
            "SELECT created_at FROM forms_requests WHERE id=?1",
            [&older.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored <= now - 30 * 60 * 1000);
    let accepted = f.post(
        "/api/forms/submit",
        json!({"entry_id":older.entry_id,"submission_id":"old-secret","conversation_id":older.route.target_conversation_id,"value":OLD}),
    );
    assert_eq!(accepted["accepted"], true);
    let listed = post_result(&mut f.app, "/api/secrets/list", json!({"bot_id":"grokbot"})).unwrap();
    assert!(listed["secrets"][0].get("value").is_none());
    assert_eq!(listed["secrets"][0]["name"], "JETON_ANCIEN");
    assert_eq!(listed["secrets"][0]["description"], "Ancien jeton");
    assert_eq!(
        forms::vault(&f.app.feature_context())
            .unwrap()
            .get_for_bot("grokbot", "JETON_ANCIEN")
            .unwrap()
            .as_deref(),
        Some(OLD)
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(!state.to_string().contains(OLD));
    assert!(!serde_json::to_string(&state["conversations"])
        .unwrap()
        .contains(OLD));
    assert_eq!(
        post_result(&mut f.app, "/api/secrets/list", json!({"bot_id":"bot-b"})).unwrap()["secrets"],
        json!([])
    );

    let replaced = f.request(
        "secret-replaced",
        "request_secret",
        json!({"name":"jeton_remplace","purpose":"Demande remplacée"}),
    );
    let current = f.request(
        "secret-current",
        "request_secret",
        json!({"name":"jeton_courant","purpose":"Demande suivante"}),
    );
    let resolutions = f.count("forms_resolutions");
    let stale = post_result(
        &mut f.app,
        "/api/forms/submit",
        json!({"entry_id":replaced.entry_id,"submission_id":"replaced-secret","conversation_id":replaced.route.target_conversation_id,"value":REPLACED}),
    )
    .unwrap_err();
    assert_eq!(stale.status, 409);
    assert_eq!(stale.message, STALE);
    assert_eq!(f.count("forms_resolutions"), resolutions);
    assert!(forms::vault(&f.app.feature_context())
        .unwrap()
        .get_for_bot("grokbot", "JETON_REMPLACE")
        .unwrap()
        .is_none());
    assert!(!f
        .app
        .api("GET", "/api/state", b"")
        .unwrap()
        .to_string()
        .contains(REPLACED));

    f.app = App::open(&f.directory.join("app.sqlite")).unwrap();
    let restarted = post_result(
        &mut f.app,
        "/api/forms/submit",
        json!({"entry_id":current.entry_id,"submission_id":"restarted-secret","conversation_id":current.route.target_conversation_id,"value":RESTARTED}),
    )
    .unwrap_err();
    assert_eq!(restarted.status, 409);
    assert_eq!(restarted.message, STALE);
    assert_eq!(f.count("forms_resolutions"), resolutions);
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    assert!(vault
        .get_for_bot("grokbot", "JETON_COURANT")
        .unwrap()
        .is_none());
    assert_eq!(
        vault
            .get_for_bot("grokbot", "JETON_ANCIEN")
            .unwrap()
            .as_deref(),
        Some(OLD)
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(!state.to_string().contains(RESTARTED));
    assert!(!state.to_string().contains(REPLACED));
    let bot_b = state["forms"]["bot_secrets"]["bot-b"].as_array().unwrap();
    assert!(bot_b.is_empty());
    assert!(state["forms"]["bot_secrets"]["grokbot"]
        .as_array()
        .unwrap()
        .iter()
        .all(|secret| secret.get("value").is_none()));
}

#[test]
fn site_form_keeps_values_outside_bot_and_replays_without_writing() {
    const SENTINEL: &str = "SENTINEL_SITE_PRIVATE_171";
    // The native host writes the wrapper's stderr to logs/engine.log. Run in
    // isolation to inspect both output streams without redirecting other tests.
    const CHILD: &str = "EZERBOT_SITE_FORM_LOG_CAPTURE";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "site_form_keeps_values_outside_bot_and_replays_without_writing",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        for bytes in [&output.stdout, &output.stderr] {
            assert!(!bytes
                .windows(SENTINEL.len())
                .any(|w| w == SENTINEL.as_bytes()));
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let mut f = Fixture::new();
    let request = f.suspended_request("run", "site", "request_site_form",
        json!({"origin":"https://example.org","fields":[{"name":"password","label":"Mot de passe","field_type":{"kind":"text"},"required":true}]}));
    let sentinel = SENTINEL;
    let body = json!({"entry_id":request.entry_id,"submission_id":"site-1","conversation_id":f.conversation,
        "origin":"https://example.org","values":{"password":sentinel}});
    let first = f.post("/api/forms/site-submit", body.clone());
    let result =
        requests::resolved_values(&f.conn, first["receipt"]["resolution_id"].as_str().unwrap())
            .unwrap();
    assert_eq!(result, json!("enregistré dans le coffre local"));
    let vault = forms::vault(&f.app.feature_context()).unwrap();
    let saved = vault
        .get(&format!("SITE_{}", request.id.to_uppercase()))
        .unwrap()
        .unwrap();
    assert!(saved.contains(sentinel));
    assert_eq!(
        grokbot_forms::redact::redact(&vault, sentinel).unwrap(),
        "[...]"
    );
    let mut persisted: Value =
        serde_json::from_slice(&std::fs::read(vault.path()).unwrap()).unwrap();
    // More than 30 minutes old, including across a fresh vault open (#81).
    let old = f.app.feature_context().now_ms as i64 - 31 * 60 * 1000;
    persisted[format!("SITE_{}", request.id.to_uppercase())]["updated_at"] = json!(old);
    std::fs::write(vault.path(), serde_json::to_vec(&persisted).unwrap()).unwrap();

    let reopened = forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(
        reopened
            .get(&format!("SITE_{}", request.id.to_uppercase()))
            .unwrap(),
        Some(saved.clone())
    );

    assert_eq!(
        grokbot_forms::redact::redact(&reopened, sentinel).unwrap(),
        "[...]"
    );
    vault
        .set(
            "SITE_SHORT",
            "Formulaire de site privé",
            r#"{"origin":"https://example.org","values":{"pin":"a","empty":"","count":42,"code":"long-code-77"}}"#,
        )
        .unwrap();
    // Values shorter than the minimum never join global masking: JSON keys stay readable.
    assert_eq!(
        grokbot_forms::redact::redact(&vault, r#"{"name":"a","id":42}"#).unwrap(),
        r#"{"name":"a","id":42}"#
    );
    assert_eq!(
        grokbot_forms::redact::redact(&vault, "code long-code-77").unwrap(),
        "code [...]"
    );

    let mut replay = body;
    replay["values"] = json!({"password":"replacement"});
    assert_eq!(
        f.post("/api/forms/site-submit", replay)["receipt"],
        first["receipt"]
    );
    assert_eq!(
        vault
            .get(&format!("SITE_{}", request.id.to_uppercase()))
            .unwrap()
            .unwrap(),
        saved
    );
    let gateway = ScriptedGateway::default();
    assert_eq!(
        f.drive("run", &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let export = f.post(
        "/api/templates/export",
        json!({"bot_id":"grokbot","include_memories":true}),
    );
    // Also inspect files emitted by application diagnostics, if any.
    if let Ok(logs) = std::fs::read_dir(f.directory.join("logs")) {
        for log in logs {
            let path = log.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !bytes
                    .windows(sentinel.len())
                    .any(|w| w == sentinel.as_bytes()),
                "secret in {}",
                path.display()
            );
        }
    }
    let evidence = json!([
        state,
        export,
        store::events(&f.conn, "run", 0).unwrap(),
        *gateway.requests.lock().unwrap()
    ]);
    assert!(!evidence.to_string().contains(sentinel));
    for name in ["app.sqlite", "app.sqlite-wal"] {
        let bytes = std::fs::read(f.directory.join(name)).unwrap_or_default();
        assert!(!bytes
            .windows(sentinel.len())
            .any(|window| window == sentinel.as_bytes()));
    }
    assert!(evidence
        .to_string()
        .contains("enregistré dans le coffre local"));
}

#[test]
fn site_form_expiry_refusal_and_target_change_have_no_vault_effect() {
    for (origin, refused, expired, expected) in [
        ("https://other.org", false, false, "cible changée"),
        ("https://example.org", true, false, "refusé"),
        ("https://example.org", false, true, "demande périmée"),
    ] {
        let mut f = Fixture::new();
        let request = f.request(
            "site",
            "request_site_form",
            json!({"origin":"https://example.org","fields":[]}),
        );
        if expired {
            f.conn.execute("UPDATE forms_requests SET kind_json=json_set(kind_json,'$.expires_at',1) WHERE id=?1", [&request.id]).unwrap();
            forms::tick(&f.app.feature_context());
        }
        let response = f.post("/api/forms/site-submit", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":f.conversation,"origin":origin,"refused":refused}));
        assert_eq!(
            requests::resolved_values(
                &f.conn,
                response["receipt"]["resolution_id"].as_str().unwrap()
            )
            .unwrap(),
            json!(expected)
        );
        assert!(forms::vault(&f.app.feature_context())
            .unwrap()
            .get(&format!("SITE_{}", request.id.to_uppercase()))
            .unwrap()
            .is_none());
        if expired {
            assert!(matches!(
                requests::get(&f.conn, &request.id).unwrap().unwrap().status,
                RequestStatus::Expired { .. }
            ));
        }
    }
}

#[test]
fn site_forms_reject_oversized_envelope_before_writing() {
    let mut f = Fixture::new();
    let request = f.suspended_request("run", "site", "request_site_form",
        json!({"origin":"https://example.org","fields":[{"name":"password","label":"Mot de passe","field_type":{"kind":"text"},"required":true}]}));
    let body = json!({"entry_id":request.entry_id,"submission_id":"large","conversation_id":f.conversation,
        "origin":"https://example.org","values":{"password":"é".repeat(2040)}});
    let error = f
        .app
        .api(
            "POST",
            "/api/forms/site-submit",
            &serde_json::to_vec(&body).unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.status, 413);
    assert!(error
        .message
        .starts_with("Formulaire trop volumineux : limite de 4096 octets"));
    assert!(forms::vault(&f.app.feature_context())
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
    let count: i64 = f
        .conn
        .query_row("SELECT COUNT(*) FROM forms_resolutions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

// --- Approbation non bloquante : le tour continue pendant que la carte attend ---

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;

impl Fixture {
    fn waiting(&mut self, body: Value) -> Value {
        self.post("/api/approvals/waiting", body)
    }
    fn reopen(&mut self) {
        self.app = App::open(&self.directory.join("app.sqlite")).unwrap();
    }
    fn request_row(&self, id: &str) -> (String, i64, i64, i64) {
        self.conn
            .query_row(
                "SELECT status, detached, json_extract(kind_json,'$.expires_at'), created_at
                 FROM forms_requests WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }
    fn protected_effects(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT count(*) FROM test_protected_effects WHERE call_id<>'free'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    /// A task run whose model asks for a protected call and then a free one.
    fn continue_task(&self) -> (String, RunProgress) {
        self.start_kind("task", "grokbot", &self.conversation, RunKind::Task);
        let gateway = ScriptedGateway::calls(vec![
            ("guarded", GUARDED_TOOL, json!({"path":"x"})),
            ("free", FREE_TOOL, json!({})),
        ]);
        let progress = self.drive("task", &gateway);
        let id = requests::list_for_run(&self.conn, "task").unwrap()[0]
            .id
            .clone();
        (id, progress)
    }
}

#[test]
fn waiting_defaults_are_on_for_tasks_and_off_for_direct_chat() {
    let mut f = Fixture::new();
    f.start_kind("task", "grokbot", &f.conversation, RunKind::Task);
    let task = store::required(&f.conn, "task").unwrap();
    let chat = store::required(&f.conn, "run").unwrap();
    assert!(forms::continues_without_approval(&f.conn, &task).unwrap());
    assert!(!forms::continues_without_approval(&f.conn, &chat).unwrap());
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        state["forms"]["waiting"],
        json!({"task":true,"chat":false,"validity":"24h","reminder":"1h"})
    );
    // Turning the setting off puts tasks back on the historical behaviour.
    f.waiting(json!({"task":false,"chat":false,"validity":"24h","reminder":"1h"}));
    assert!(!forms::continues_without_approval(&f.conn, &task).unwrap());
    f.waiting(json!({"task":true,"chat":true,"validity":"24h","reminder":"1h"}));
    assert!(forms::continues_without_approval(&f.conn, &chat).unwrap());
}

#[test]
fn waiting_validity_accepts_four_values_and_refuses_the_rest() {
    let mut f = Fixture::new();
    for validity in ["1h", "6h", "24h", "7d"] {
        let saved =
            f.waiting(json!({"task":true,"chat":false,"validity":validity,"reminder":"1h"}));
        assert_eq!(saved["validity"], validity);
    }
    for reminder in ["30min", "1h", "3h", "never"] {
        let saved =
            f.waiting(json!({"task":true,"chat":false,"validity":"7d","reminder":reminder}));
        assert_eq!(saved["reminder"], reminder);
    }
    let rejected = |f: &mut Fixture, body: Value| {
        f.app
            .api(
                "POST",
                "/api/approvals/waiting",
                &serde_json::to_vec(&body).unwrap(),
            )
            .is_err()
    };
    for validity in ["2h", "24H", "", "86400000", "30d"] {
        assert!(
            rejected(
                &mut f,
                json!({"task":true,"chat":false,"validity":validity,"reminder":"1h"})
            ),
            "{validity}"
        );
    }
    assert!(rejected(
        &mut f,
        json!({"task":true,"chat":false,"validity":"24h","reminder":"2h"})
    ));
    assert!(rejected(
        &mut f,
        json!({"task":false,"chat":true,"validity":"24h","reminder":"1h"})
    ));
    assert!(rejected(
        &mut f,
        json!({"task":true,"chat":false,"validity":"24h","reminder":"1h","extra":1})
    ));
    // A refused save leaves the last accepted setting in place.
    let state = f.app.api("GET", "/api/approvals/waiting", b"").unwrap();
    assert_eq!(
        state,
        json!({"task":true,"chat":false,"validity":"7d","reminder":"never"})
    );
    // An unreadable stored value falls back to the defaults, never to something broader.
    f.conn
        .execute("UPDATE wrapper_approval_waiting SET settings='{\"task\":true,\"chat\":true,\"validity\":\"99h\",\"reminder\":\"1h\"}'", [])
        .unwrap();
    let state = f.app.api("GET", "/api/approvals/waiting", b"").unwrap();
    assert_eq!(
        state,
        json!({"task":true,"chat":false,"validity":"24h","reminder":"1h"})
    );
}

#[test]
fn a_task_turn_continues_past_a_protected_call_without_any_effect() {
    let f = Fixture::new();
    let (id, progress) = f.continue_task();
    assert_eq!(progress, RunProgress::Ended(TurnEnd::Finished));
    // The free call ran; the protected one did not, and the model is told so.
    assert_eq!(f.count("test_protected_effects"), 1);
    assert_eq!(f.protected_effects(), 0);
    let events = store::events(&f.conn, "task", 0).unwrap();
    assert!(!events.iter().any(|e| e.kind == RunEventKind::Suspended));
    let result = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "guarded")
        .unwrap();
    assert_eq!(result.payload["outcome"]["status"], "failed");
    let reason = result.payload["outcome"]["reason"].as_str().unwrap();
    assert!(reason.contains("En attente d’approbation"), "{reason}");
    assert!(reason.contains("rien n’a été exécuté"), "{reason}");
    // The card was created and is still waiting once the turn is over.
    let (status, detached, expires, created) = f.request_row(&id);
    assert_eq!((status.as_str(), detached), ("pending", 1));
    assert_eq!(expires - created, (24 * HOUR) as i64);
    assert_eq!(
        requests::get(&f.conn, &id).unwrap().unwrap().status,
        RequestStatus::Pending
    );
}

#[test]
fn a_waiting_card_outlives_ten_minutes_and_expires_after_the_chosen_validity_with_no_effect() {
    let mut f = Fixture::new();
    f.waiting(json!({"task":true,"chat":false,"validity":"6h","reminder":"1h"}));
    let (id, _) = f.continue_task();
    let (_, _, expires, created) = f.request_row(&id);
    assert_eq!(expires - created, (6 * HOUR) as i64);
    let created = created as u64;
    f.app.tick(created + 11 * MINUTE);
    assert_eq!(f.request_row(&id).0, "pending");
    f.app.tick(created + 6 * HOUR - 1);
    assert_eq!(f.request_row(&id).0, "pending");
    f.app.tick(created + 6 * HOUR);
    assert!(matches!(
        requests::get(&f.conn, &id).unwrap().unwrap().status,
        RequestStatus::Expired { .. }
    ));
    assert_eq!(f.protected_effects(), 0);
    assert_eq!(
        f.drive("task", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 0);
}

#[test]
fn a_restart_keeps_the_waiting_card_pending_and_executes_nothing() {
    let mut f = Fixture::new();
    let (id, _) = f.continue_task();
    f.reopen();
    assert_eq!(f.request_row(&id).0, "pending");
    let created = f.request_row(&id).3 as u64;
    f.app.tick(created + 11 * MINUTE);
    assert_eq!(f.request_row(&id).0, "pending");
    assert_eq!(
        store::required(&f.conn, "task").unwrap().status,
        RunStatus::Finished
    );
    assert_eq!(f.protected_effects(), 0);
    f.reopen();
    assert_eq!(f.protected_effects(), 0);
}

#[test]
fn without_the_setting_a_protected_call_still_suspends_for_ten_minutes() {
    let mut f = Fixture::new();
    // Direct chat is off by default.
    let chat = f.suspended_request("run", "guarded", GUARDED_TOOL, json!({"path":"x"}));
    let (status, detached, expires, created) = f.request_row(&chat.id);
    assert_eq!((status.as_str(), detached), ("pending", 0));
    assert_eq!(expires - created, (10 * MINUTE) as i64);
    f.app.tick(created as u64 + 10 * MINUTE);
    assert!(matches!(
        requests::get(&f.conn, &chat.id).unwrap().unwrap().status,
        RequestStatus::Expired { .. }
    ));
    // A task with the setting off behaves the same way.
    f.waiting(json!({"task":false,"chat":false,"validity":"24h","reminder":"1h"}));
    f.start_kind("task", "grokbot", &f.conversation, RunKind::Task);
    let progress = f.drive(
        "task",
        &ScriptedGateway::call("guarded", GUARDED_TOOL, json!({"path":"y"})),
    );
    assert!(matches!(
        progress,
        RunProgress::Suspended(SuspendCause::HumanInput { .. })
    ));
    assert_eq!(
        store::required(&f.conn, "task").unwrap().status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(f.protected_effects(), 0);
}

#[test]
fn sends_and_deletions_keep_their_confirmation_and_never_offer_always_allow() {
    let mut f = Fixture::new();
    f.waiting(json!({"task":true,"chat":true,"validity":"7d","reminder":"1h"}));
    f.start_kind("task", "grokbot", &f.conversation, RunKind::Task);
    for (name, command) in [("send", "sendmail bob"), ("delete", "rm note")] {
        let call = ToolCall {
            call_id: name.into(),
            name: "run_shell".into(),
            arguments: json!({"command": command}),
        };
        // The standing-rule gate is untouched: a protected call is always asked.
        let decision = with_tx(&f.conn, |conn| -> Result<_, ApiError> {
            let run = store::required(conn, "task")?;
            forms::check_tool_approval(&tool_context(&f.app, conn, &run), &run, &call)
        })
        .unwrap();
        assert_eq!(decision, ApprovalCheck::Ask, "{name}");
        let request = f.approval_call("task", &call);
        for decision in [
            json!({"always_allow":{"scope":"global"}}),
            json!({"never":{"scope":"global"}}),
        ] {
            let body = json!({"entry_id":request.entry_id,"submission_id":format!("{name}-always"),"conversation_id":request.route.target_conversation_id,"decision":decision});
            assert!(
                f.app
                    .api(
                        "POST",
                        "/api/approvals/resolve",
                        &serde_json::to_vec(&body).unwrap()
                    )
                    .is_err(),
                "{name}"
            );
        }
        assert_eq!(f.request_row(&request.id).0, "pending", "{name}");
        assert_eq!(f.count("forms_approval_rules"), 0, "{name}");
    }
}

#[test]
fn stopping_a_running_task_still_cancels_its_waiting_card() {
    let f = Fixture::new();
    f.start_kind("task", "grokbot", &f.conversation, RunKind::Task);
    let request = f.approval_for("task", "guarded", json!({"path":"x"}));
    assert_eq!(f.request_row(&request.id).1, 1);
    let mut services = Services {
        app: &f.app,
        force_first_ask: false,
    };
    run_loop::with_run_events(&f.conn, &mut services, "task", |conn, _| {
        model::cancel(conn, "task", TurnEnd::Stopped)
    })
    .unwrap();
    assert_eq!(f.request_row(&request.id).0, "cancelled");
    assert_eq!(f.protected_effects(), 0);
}

#[test]
fn host_authored_runs_keep_waiting_for_the_decision_even_with_the_chat_option_on() {
    let mut f = Fixture::new();
    f.waiting(json!({"task":true,"chat":true,"validity":"24h","reminder":"1h"}));
    f.start_kind(
        "mcp-host-settings",
        "grokbot",
        &f.conversation,
        RunKind::Chat,
    );
    let host = store::required(&f.conn, "mcp-host-settings").unwrap();
    assert!(!forms::continues_without_approval(&f.conn, &host).unwrap());
    // An ordinary chat run does continue with the option on.
    let chat = store::required(&f.conn, "run").unwrap();
    assert!(forms::continues_without_approval(&f.conn, &chat).unwrap());
    // The settings flow suspends and its card keeps the historical ten minutes.
    let progress = f.drive(
        "mcp-host-settings",
        &ScriptedGateway::call("guarded", GUARDED_TOOL, json!({"path":"x"})),
    );
    let RunProgress::Suspended(SuspendCause::HumanInput { id }) = progress else {
        panic!("{progress:?}")
    };
    let (status, detached, expires, created) = f.request_row(&id);
    assert_eq!((status.as_str(), detached), ("pending", 0));
    assert_eq!(expires - created, (10 * MINUTE) as i64);
    assert_eq!(
        store::required(&f.conn, "mcp-host-settings")
            .unwrap()
            .status,
        RunStatus::AwaitingApproval
    );
}

// --- Approbation non bloquante : la décision arrive après la fin du tour ---

impl Fixture {
    fn late_run(&self) -> String {
        self.conn
            .query_row("SELECT run_id FROM wrapper_late_approvals", [], |row| {
                row.get(0)
            })
            .unwrap()
    }
    fn late_runs(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT count(*) FROM engine_runs WHERE run_id LIKE 'late-%'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn run_input_text(&self, run: &str) -> String {
        let input: String = self
            .conn
            .query_row(
                "SELECT input_json FROM wrapper_engine_runs WHERE run_id=?1",
                [run],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&input).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn resolve_error(&mut self, request: &HumanInputRequest, nonce: &str) -> ApiError {
        let body = json!({"entry_id":request.entry_id,"submission_id":nonce,"conversation_id":request.route.target_conversation_id,"decision":"allow_once"});
        self.app
            .api(
                "POST",
                "/api/approvals/resolve",
                &serde_json::to_vec(&body).unwrap(),
            )
            .unwrap_err()
    }
    /// A finished task with its protected call still waiting, and that card.
    fn waiting_card(&self) -> HumanInputRequest {
        let (id, progress) = self.continue_task();
        assert_eq!(progress, RunProgress::Ended(TurnEnd::Finished));
        assert_eq!(self.protected_effects(), 0);
        requests::get(&self.conn, &id).unwrap().unwrap()
    }
}

#[test]
fn a_card_approved_after_its_turn_ended_runs_the_action_once_in_the_same_conversation() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.decide(&request, "late", json!("allow_once"));
    // The decision runs nothing by itself: it opens a turn where the bot was working.
    assert_eq!(f.protected_effects(), 0);
    let follow = f.late_run();
    let turn = store::required(&f.conn, &follow).unwrap();
    assert_eq!(turn.conversation_id, f.conversation);
    assert_eq!(turn.bot_id, "grokbot");
    let input = f.run_input_text(&follow);
    assert!(
        input.contains("Charles a approuvé : test_protected_write"),
        "{input}"
    );
    assert!(input.contains(r#""path":"x""#), "{input}");
    let gateway = ScriptedGateway::default();
    assert_eq!(
        f.drive(&follow, &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    // Exactly once, with the arguments the card showed.
    assert_eq!(f.protected_effects(), 1);
    let arguments: String = f
        .conn
        .query_row(
            "SELECT arguments FROM test_protected_effects WHERE call_id='guarded'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(arguments, r#"{"path":"x"}"#);
    let events = store::events(&f.conn, &follow, 0).unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult)
        .unwrap();
    assert_eq!(
        result.payload["authorized_by"],
        json!({"kind":"allow_once","request_id":request.id})
    );
    // The bot is asked after the result, in the conversation, so it can report it.
    assert_eq!(gateway.requests.lock().unwrap().len(), 1);
    let thread: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM wrapper_messages WHERE conversation_id=?1 AND kind='engine_reply' AND message_id=?2",
            params![f.conversation, follow],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(thread, 1);
    // A second « Approuver » (another device) has no effect, and neither has a retry.
    let again = f.resolve_error(&request, "second-device");
    assert_eq!(again.status, 409);
    assert!(
        again.message.contains("déjà reçu une réponse"),
        "{}",
        again.message
    );
    f.decide(&request, "late", json!("allow_once"));
    assert_eq!(f.late_runs(), 1);
    assert_eq!(
        f.drive(&follow, &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 1);
    assert!(approvals::consume_allow_once(
        &f.conn,
        &request.id,
        &request.run_id,
        GUARDED_TOOL,
        &json!({"path":"x"}),
        0
    )
    .is_err());
}

#[test]
fn a_refusal_after_the_turn_ended_tells_the_bot_and_runs_nothing() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.decide(&request, "refuse", json!("deny"));
    let follow = f.late_run();
    let input = f.run_input_text(&follow);
    assert!(
        input.contains("Charles a refusé : test_protected_write"),
        "{input}"
    );
    assert!(input.contains("Rien n’a été exécuté"), "{input}");
    // The turn carries no call at all: nothing can run from it.
    assert!(!store::events(&f.conn, &follow, 0)
        .unwrap()
        .iter()
        .any(|e| e.kind == RunEventKind::ToolCall));
    let gateway = ScriptedGateway::default();
    assert_eq!(
        f.drive(&follow, &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 0);
    assert_eq!(gateway.requests.lock().unwrap().len(), 1);
    assert_eq!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Denied
    );
}

#[test]
fn the_bot_cannot_run_the_approved_tool_with_other_arguments_in_the_follow_up_turn() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.decide(&request, "late", json!("allow_once"));
    let follow = f.late_run();
    // After the approved call, the model asks for the same tool with a changed path.
    let gateway = ScriptedGateway::calls(vec![("again", GUARDED_TOOL, json!({"path":"changed"}))]);
    assert_eq!(
        f.drive(&follow, &gateway),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 1);
    let events = store::events(&f.conn, &follow, 0).unwrap();
    let refused = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "again")
        .unwrap();
    assert_eq!(refused.payload["outcome"]["status"], "failed");
    assert!(refused.payload["outcome"]["reason"]
        .as_str()
        .unwrap()
        .contains("ne sont pas ceux que Charles a approuvés"));
    // Nothing new was asked of Charles and nothing of the other arguments ran.
    assert!(requests::list_for_run(&f.conn, &follow).unwrap().is_empty());
    let changed: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM test_protected_effects WHERE arguments LIKE '%changed%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(changed, 0);
}

#[test]
fn an_expired_or_cancelled_card_cannot_be_approved_and_nothing_runs() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    let created = f.request_row(&request.id).3 as u64;
    f.app.tick(created + 24 * HOUR);
    assert!(matches!(
        requests::get(&f.conn, &request.id).unwrap().unwrap().status,
        RequestStatus::Expired { .. }
    ));
    let refused = f.resolve_error(&request, "too-late");
    assert_eq!(refused.status, 409);
    assert_eq!(
        refused.message,
        "Cette demande a expiré. Aucune action n’a été faite."
    );
    assert_eq!(f.count("wrapper_late_approvals"), 0);
    assert_eq!(f.late_runs(), 0);
    assert_eq!(f.protected_effects(), 0);
    // A card cancelled by an explicit stop answers in the same way.
    f.start_kind("stopped", "grokbot", &f.conversation, RunKind::Task);
    let stopped = f.approval_for("stopped", "guarded", json!({"path":"y"}));
    let mut services = Services {
        app: &f.app,
        force_first_ask: false,
    };
    run_loop::with_run_events(&f.conn, &mut services, "stopped", |conn, _| {
        model::cancel(conn, "stopped", TurnEnd::Stopped)
    })
    .unwrap();
    let cancelled = f.resolve_error(&stopped, "after-stop");
    assert_eq!(cancelled.status, 409);
    assert_eq!(
        cancelled.message,
        "Cette demande a été annulée. Aucune action n’a été faite."
    );
    assert_eq!(f.late_runs(), 0);
    assert_eq!(f.protected_effects(), 0);
}

#[test]
fn a_restart_between_the_end_of_the_turn_and_the_decision_still_runs_the_action_once() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.reopen();
    f.decide(&request, "late", json!("allow_once"));
    let follow = f.late_run();
    f.reopen();
    assert_eq!(
        f.drive(&follow, &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 1);
    f.reopen();
    assert_eq!(
        f.drive(&follow, &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 1);
    assert_eq!(f.late_runs(), 1);
}

#[test]
fn a_retried_start_finds_the_follow_up_turn_and_does_not_write_its_call_twice() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.decide(&request, "late", json!("allow_once"));
    let follow = f.late_run();
    // The host stopped before it marked the turn as started.
    f.conn
        .execute("UPDATE wrapper_late_approvals SET started=0", [])
        .unwrap();
    forms::tick(&f.app.feature_context());
    forms::tick(&f.app.feature_context());
    assert_eq!(f.late_runs(), 1);
    let calls = store::events(&f.conn, &follow, 0)
        .unwrap()
        .iter()
        .filter(|e| e.kind == RunEventKind::ToolCall)
        .count();
    assert_eq!(calls, 1);
    f.drive(&follow, &ScriptedGateway::default());
    assert_eq!(f.protected_effects(), 1);
}

#[test]
fn a_card_decided_while_its_turn_blocks_keeps_the_ordinary_resume() {
    let mut f = Fixture::new();
    // Direct chat stays blocking by default: the decision resumes that very turn.
    let request = f.suspended_request("run", "guarded", GUARDED_TOOL, json!({"path":"x"}));
    f.decide(&request, "now", json!("allow_once"));
    assert_eq!(f.count("wrapper_late_approvals"), 0);
    assert_eq!(f.late_runs(), 0);
    assert_eq!(
        f.drive("run", &ScriptedGateway::default()),
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(f.protected_effects(), 1);
}

#[test]
fn the_follow_up_turn_of_a_late_decision_cannot_be_regenerated() {
    let mut f = Fixture::new();
    let request = f.waiting_card();
    f.decide(&request, "late", json!("allow_once"));
    let follow = f.late_run();
    f.drive(&follow, &ScriptedGateway::default());
    assert_eq!(f.protected_effects(), 1);
    let error = f
        .app
        .api(
            "POST",
            "/api/regenerate",
            &serde_json::to_vec(&json!({"conversation_id":f.conversation,"message_id":follow}))
                .unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert!(
        error.message.contains("décision tardive"),
        "{}",
        error.message
    );
    // The turn stays as it ended and nothing the card approved runs again.
    assert_eq!(
        store::required(&f.conn, &follow).unwrap().status,
        RunStatus::Finished
    );
    assert_eq!(f.late_runs(), 1);
    assert_eq!(f.protected_effects(), 1);
}

#[test]
fn a_forbidden_action_is_denied_whatever_a_card_a_rule_or_yolo_says() {
    let mut f = Fixture::new();
    let call = |id: &str, name: &str| ToolCall {
        call_id: id.into(),
        name: name.into(),
        arguments: json!({"amount": 10}),
    };
    let check = |f: &Fixture, call: &ToolCall| {
        with_tx(&f.conn, |conn| -> Result<_, ApiError> {
            let run = store::required(conn, "run")?;
            forms::check_tool_approval(&tool_context(&f.app, conn, &run), &run, call)
        })
        .unwrap()
    };
    let card_accepted = |f: &mut Fixture, call: &ToolCall| {
        let card = f.approval_call("run", call);
        f.decide(&card, &format!("oui-{}", call.call_id), json!("allow_once"));
    };
    // Control: the same accepted card admits a harmless tool, so the refusals below
    // come from the forbidden list and not from the setup.
    let harmless = call("lecture", "stripe_list_payments");
    card_accepted(&mut f, &harmless);
    assert!(matches!(
        check(&f, &harmless),
        ApprovalCheck::Authorized { .. }
    ));
    let payment = call("paiement", "stripe_create_payment");
    let refused = |decision: ApprovalCheck| matches!(decision, ApprovalCheck::Denied { reason } if reason.starts_with("Je ne peux pas faire ça : les paiements sont interdits aux bots"));
    card_accepted(&mut f, &payment);
    assert!(refused(check(&f, &payment)));
    f.conn
        .execute(
            "INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,'stripe_create_payment','','\"global\"',1)",
            params![f.conversation],
        )
        .unwrap();
    assert!(refused(check(&f, &call("autre", "stripe_create_payment"))));
    f.post("/api/yolo/mode", json!({"mode": "yolo"}));
    assert!(refused(check(&f, &call("encore", "stripe_create_payment"))));
    assert_eq!(f.count("forms_requests"), 2, "aucune carte pour un refus");
}
