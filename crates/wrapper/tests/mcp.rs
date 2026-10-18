mod support;
use grokbot_engine::{
    contracts::{RunEventKind, ToolOutcome},
    model::{Run, RunStatus},
    store,
};
use grokbot_forms::requests;
use grokbot_wrapper::{features, App, Provider};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Fixture {
    app: App,
    provider: Provider,
    ai: FakeOpenAi,
    path: PathBuf,
    conversation: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "mcp-integration-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("app.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let ai = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, ai.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        wait("catalog", || provider.status()["models_ready"] == true);
        let conversation = app
            .api(
                "POST",
                "/api/conversations",
                br#"{"title":"Connecteurs MCP"}"#,
            )
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            app,
            provider,
            ai,
            path,
            conversation,
        }
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn add(&mut self, mode: &str) -> String {
        self.post("/api/mcp/add",json!({"name":"Atelier local","command":"python3","arguments":[format!("{}/tests/fixtures/mcp_server.py",env!("CARGO_MANIFEST_DIR")),mode],"environment":{"TEST_TOKEN":"private-value-56789","SHORT":"🦜","SPACE":"  avec espaces  ","EMPTY":""}}))["id"].as_str().unwrap().to_owned()
    }
    fn db(&self) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(&self.path).unwrap();
        c.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
        c
    }
    fn request(&self, run: &str) -> requests::HumanInputRequest {
        requests::list_for_run(&self.db(), run).unwrap().remove(0)
    }
    fn decide(&mut self, run: &str, decision: Value) {
        let r = self.request(run);
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":format!("decision-{run}"),"decision":decision}));
        self.app.tick(0);
    }
    fn start(&mut self, id: &str) -> String {
        self.post("/api/mcp/restart", json!({"id":id}))["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn connect(&mut self, id: &str) {
        self.post(
            "/api/mcp/select",
            json!({"id":id,"bot_id":"grokbot","enabled":true}),
        );
        let run = self.start(id);
        self.decide(&run, json!("allow_once"));
        assert_eq!(self.state()["mcp"]["servers"][0]["status"], "connected");
    }
    fn tool(&mut self, name: &str) -> String {
        self.state()["mcp"]["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["namespace"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn script(&self, events: Vec<Value>) {
        self.ai
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn send(&mut self, name: &str, args: Value, id: &str) -> Run {
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed"})]);
        self.post("/api/send",json!({"conversation_id":self.conversation,"message_id":id,"body":"Utilise le connecteur"}));
        wait("approval", || self.provider.status()["titling"] == false);
        let conn = self.db();
        let raw:String=conn.query_row("SELECT r.record FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        serde_json::from_str(&raw).unwrap()
    }
    fn outcome(&self, run: &str) -> ToolOutcome {
        let conn = self.db();
        let event = store::events(&conn, run, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult)
            .expect("tool result");
        serde_json::from_value(event.payload["outcome"].clone()).unwrap()
    }
    fn finish(&self, run: &str) {
        wait("tool response", || {
            store::required(&self.db(), run)
                .unwrap()
                .status
                .is_terminal()
                && self.provider.status()["titling"] == false
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}

#[test]
fn connector_credentials_stay_private_after_vault_rotation_and_cannot_be_overwritten_by_a_model_form(
) {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let tool = f.tool("echo");
    let name = format!("MCP_{id}_ENV").to_uppercase();
    let rotated = json!({"variables":{"TEST_TOKEN":"rotated-private-key-9999"}}).to_string();
    features::forms::vault(&f.app.feature_context())
        .unwrap()
        .set(&name, grokbot_forms::vault::MCP_ENV_PURPOSE, &rotated)
        .unwrap();
    let run = f.send(&tool, json!({"text":"secret"}), "rotated-secret");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done {output} if output["content"][0]["text"]=="[masqué]")
    );
    assert!(!f.state().to_string().contains("private-value-56789"));
    let overwrite = f.send(
        "request_secret",
        json!({"name":name,"purpose":"Remplacer le secret"}),
        "overwrite-private-env",
    );
    assert!(
        matches!(f.outcome(&overwrite.run_id),ToolOutcome::Failed {reason} if reason.contains("réservé"))
    );
    assert!(requests::list_for_run(&f.db(), &overwrite.run_id)
        .unwrap()
        .is_empty());
    assert_eq!(
        features::forms::vault(&f.app.feature_context())
            .unwrap()
            .get(&name)
            .unwrap()
            .unwrap(),
        rotated
    );
}

#[test]
fn concurrent_ticks_resume_two_approvals_without_duplicate_or_crossed_results() {
    let mut f = Fixture::new();
    let id = f.add("notifications");
    f.connect(&id);
    let tool = f.tool("echo");
    let first = f.send(&tool, json!({"text":"slow-premier"}), "concurrent-first");
    f.conversation = f.post("/api/conversations", json!({"title":"Autre conversation"}))
        ["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = f.send(&tool, json!({"text":"slow-second"}), "concurrent-second");
    let mut other = App::open_with(&f.path, f.provider.clone()).unwrap();
    f.script(vec![json!({"type":"response.completed"})]);
    f.script(vec![json!({"type":"response.completed"})]);
    for run in [&first, &second] {
        let r = f.request(&run.run_id);
        f.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":"concurrent-answer","decision":"allow_once"}));
    }
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let handle = std::thread::spawn(move || {
        start.wait();
        other.tick(0);
        for _ in 0..4 {
            let state = other.api("GET", "/api/state", b"").unwrap();
            assert_eq!(state["mcp"]["servers"].as_array().unwrap().len(), 1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    barrier.wait();
    f.app.tick(0);
    handle.join().unwrap();
    for (run, expected) in [(&first, "slow-premier"), (&second, "slow-second")] {
        f.finish(&run.run_id);
        let events = store::events(&f.db(), &run.run_id, 0).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == RunEventKind::ToolResult)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == RunEventKind::Resumed)
                .count(),
            1
        );
        assert!(
            matches!(f.outcome(&run.run_id),ToolOutcome::Done {output} if output["content"][0]["text"]==expected)
        );
    }
}
#[test]
fn concurrent_restart_and_old_disconnect_notification_preserve_the_replacement_process() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let tool = f.tool("echo");
    for index in 0..3 {
        let run = f.send(
            &tool,
            json!({"text":"crash-after"}),
            &format!("late-crash-{index}"),
        );
        f.script(vec![json!({"type":"response.completed"})]);
        f.decide(&run.run_id, json!("allow_once"));
        f.finish(&run.run_id);
        std::thread::sleep(std::time::Duration::from_millis(100));
        let restart = f.start(&id);
        let request = f.request(&restart);
        let mut reader = App::open_with(&f.path, f.provider.clone()).unwrap();
        let mut runner = App::open_with(&f.path, f.provider.clone()).unwrap();
        f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":format!("restart-{index}"),"decision":"allow_once"}));
        let mut conn = f.db();
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let read_start = barrier.clone();
        let run_start = barrier.clone();
        let reading = std::thread::spawn(move || {
            read_start.wait();
            let value = reader.api("GET", "/api/state", b"").unwrap();
            assert_eq!(value["mcp"]["servers"].as_array().unwrap().len(), 1);
        });
        let restarting = std::thread::spawn(move || {
            run_start.wait();
            runner.tick(0);
        });
        barrier.wait();
        std::thread::sleep(std::time::Duration::from_millis(200));
        tx.commit().unwrap();
        reading.join().unwrap();
        restarting.join().unwrap();
        f.finish(&restart);
        assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
        // App::open advances the submission-journal generation. Reload that
        // session before sending the next prompt; keep the shared MCP manager.
        f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    }
}

#[test]
fn catalogue_notification_retries_after_a_failed_sql_write_without_losing_the_server() {
    let mut f = Fixture::new();
    let id = f.add("notifications");
    f.connect(&id);
    let tool = f.tool("echo");
    f.db().execute_batch("CREATE TRIGGER fail_mcp_catalogue BEFORE UPDATE OF tools_json ON wrapper_mcp_servers BEGIN SELECT RAISE(FAIL,'forced'); END;").unwrap();
    let run = f.send(&tool, json!({"text":"Bonjour"}), "catalogue-retry");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    let deferred = f.state();
    assert_eq!(deferred["mcp"]["servers"].as_array().unwrap().len(), 1);
    assert!(deferred["mcp"]["error"]
        .as_str()
        .unwrap()
        .contains("différée"));
    f.db()
        .execute_batch("DROP TRIGGER fail_mcp_catalogue;")
        .unwrap();
    let refreshed = f.state();
    assert!(refreshed["mcp"]["error"].is_null());
    assert!(refreshed["mcp"]["servers"][0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["description"] == "Catalogue actualisé."));
}

#[test]
fn a_connector_cannot_echo_a_different_connectors_environment_secret() {
    let mut f = Fixture::new();
    let _first = f.add("normal");
    let id=f.post("/api/mcp/add",json!({"name":"Alpha serveur","command":"python3","arguments":[format!("{}/tests/fixtures/mcp_server.py",env!("CARGO_MANIFEST_DIR")),"foreign"],"environment":{"TEST_TOKEN":"second-private-key-5555"}}))["id"].as_str().unwrap().to_owned();
    f.post(
        "/api/mcp/select",
        json!({"id":id,"bot_id":"grokbot","enabled":true}),
    );
    let start = f.start(&id);
    f.decide(&start, json!("allow_once"));
    let tool = f.tool("echo");
    let run = f.send(&tool, json!({"text":"secret"}), "foreign-secret");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done {output} if output["content"][0]["text"]=="[masqué]")
    );
    assert!(!f.state().to_string().contains("private-value-56789"));
    assert!(!f
        .ai
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .any(|b| b.to_string().contains("private-value-56789")));
}

#[test]
fn malformed_tool_result_is_an_explicit_error_not_a_broken_card() {
    let mut f = Fixture::new();
    let id = f.add("invalid-content");
    f.connect(&id);
    let tool = f.tool("echo");
    let run = f.send(&tool, json!({"text":"Bonjour"}), "invalid-result");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Failed {reason} if reason.contains("invalide") && reason.contains("déconnecté"))
    );
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "error");
}
#[test]
fn duplicate_names_are_refused_without_overwriting_the_first_servers_secrets() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    let error = f
        .app
        .api(
            "POST",
            "/api/mcp/add",
            br#"{"name":"ATELIER LOCAL","command":"python3"}"#,
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert!(error.message.contains("nom distinct"));
    assert_eq!(f.state()["mcp"]["servers"].as_array().unwrap().len(), 1);
    let vault = features::forms::vault(&f.app.feature_context()).unwrap();
    let raw = vault
        .get(&format!("MCP_{id}_ENV").to_uppercase())
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["variables"]["EMPTY"], "");
    assert_eq!(value["variables"]["SPACE"], "  avec espaces  ");
}

#[test]
fn vault_limit_and_private_schema_collisions_fail_closed() {
    let mut f = Fixture::new();
    let body = json!({"name":"Trop grand","command":"python3","environment":{"FIRST":"a".repeat(2048),"SECOND":"b".repeat(2048)}});
    let error = f
        .app
        .api("POST", "/api/mcp/add", body.to_string().as_bytes())
        .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("4 Ko"));
    assert!(f.state()["mcp"]["servers"].as_array().unwrap().is_empty());
    assert!(features::forms::vault(&f.app.feature_context())
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
    let id=f.post("/api/mcp/add",json!({"name":"Collision privée","command":"python3","arguments":[format!("{}/tests/fixtures/mcp_server.py",env!("CARGO_MANIFEST_DIR"))],"environment":{"TEST_TOKEN":"object"}}))["id"].as_str().unwrap().to_owned();
    let run = f.start(&id);
    f.decide(&run, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run),ToolOutcome::Failed {reason} if reason.contains("variable privée"))
    );
    assert!(f.state()["mcp"]["servers"][0]["tools"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn notifications_refresh_catalogue_and_unknown_tool_fails_in_french() {
    let mut f = Fixture::new();
    let id = f.add("notifications");
    f.connect(&id);
    let tool = f.tool("echo");
    let run = f.send(&tool, json!({"text":"Bonjour"}), "notify");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    assert!(f.state()["mcp"]["servers"][0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["description"] == "Catalogue actualisé."));
    let unknown = f.send("mcp_inconnu", json!({}), "unknown-call");
    assert!(unknown.status.is_terminal());
    assert!(
        matches!(f.outcome(&unknown.run_id),ToolOutcome::Failed {reason} if reason.contains("Outil MCP inconnu"))
    );
}
#[test]
fn server_crash_during_tool_and_disconnected_call_do_not_auto_restart() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let tool = f.tool("echo");
    let crash = f.send(&tool, json!({"text":"crash"}), "crash-call");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&crash.run_id, json!("allow_once"));
    f.finish(&crash.run_id);
    assert!(
        matches!(f.outcome(&crash.run_id),ToolOutcome::Failed {reason} if reason.contains("déconnecté"))
    );
    let absent = f.send(&tool, json!({"text":"Bonjour"}), "disconnected-call");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&absent.run_id, json!("allow_once"));
    f.finish(&absent.run_id);
    assert!(
        matches!(f.outcome(&absent.run_id),ToolOutcome::Failed {reason} if reason.contains("déconnecté"))
    );
    let restart = f.start(&id);
    f.decide(&restart, json!("allow_once"));
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
}

#[test]
fn launch_denial_persistence_restart_and_remove_use_the_gate() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "disconnected");
    let denied = f.start(&id);
    assert_eq!(
        store::required(&f.db(), &denied).unwrap().status,
        RunStatus::AwaitingApproval
    );
    assert!(
        matches!(f.request(&denied).kind,requests::InputKind::Approval {tool,..} if tool==format!("mcp_start_{id}"))
    );
    f.decide(&denied, json!("deny"));
    assert!(
        matches!(f.outcome(&denied),ToolOutcome::Failed {reason} if reason.contains("refusée"))
    );
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "disconnected");
    f.connect(&id);
    let state = f.state().to_string();
    assert!(!state.contains("private-value-56789"));
    assert!(!state.contains("avec espaces"));
    // A fresh process-local manager starts disconnected, with bot selection and
    // discovered tools durable. It must ask again before launching any command.
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "disconnected");
    assert_eq!(
        f.state()["mcp"]["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        f.state()["mcp"]["servers"][0]["enabled_bots"],
        json!(["grokbot"])
    );
    let restart = f.start(&id);
    f.decide(&restart, json!("allow_once"));
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
    let remove = f.post("/api/mcp/remove", json!({"id":id}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(f.state()["mcp"]["servers"].as_array().unwrap().len(), 1);
    f.decide(&remove, json!("allow_once"));
    assert!(f.state()["mcp"]["servers"].as_array().unwrap().is_empty());
    assert!(features::forms::vault(&f.app.feature_context())
        .unwrap()
        .list()
        .unwrap()
        .is_empty());
}
#[test]
fn model_tools_are_bot_scoped_approved_namespaced_and_secret_safe() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let tool = f.tool("echo");
    assert!(tool.starts_with(&format!("mcp_{id}_")));
    assert!(
        matches!(f.app.invoke_tool("grokbot",&tool,json!({"text":"bypass"})).unwrap(),ToolOutcome::Failed {reason} if reason.contains("approbation"))
    );
    let run = f.send(&tool, json!({"text":"secret"}), "echo-first");
    assert_eq!(run.status, RunStatus::AwaitingApproval);
    assert!(store::events(&f.db(), &run.run_id, 0)
        .unwrap()
        .iter()
        .all(|e| e.kind != RunEventKind::ToolResult));
    let body = f.ai.seen.lock().unwrap().bodies.last().unwrap().clone();
    assert!(body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == tool));
    assert!(body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| !t["name"].as_str().unwrap().starts_with("mcp_start_")));
    f.script(vec![
        json!({"type":"response.output_text.delta","delta":"Résultat reçu."}),
        json!({"type":"response.completed"}),
    ]);
    f.decide(&run.run_id, json!({"always_allow":{"scope":"global"}}));
    f.finish(&run.run_id);
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done {output} if output["content"][0]["text"]=="[masqué]")
    );
    assert!(!f.state().to_string().contains("private-value-56789"));
    assert!(!f
        .ai
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .any(|b| b.to_string().contains("private-value-56789")));
    // Global Always binds this server + tool + requesting bot, not arguments.
    f.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc-next","call_id":"next","name":tool,"arguments":"{\"text\":\"Bonjour\"}"}}),json!({"type":"response.completed"})]);
    f.script(vec![json!({"type":"response.completed"})]);
    f.post(
        "/api/send",
        json!({"conversation_id":f.conversation,"message_id":"second","body":"Encore"}),
    );
    wait("standing call", || f.provider.status()["titling"] == false);
    let runs = store::runs_for_conversation(&f.db(), &f.conversation).unwrap();
    let second = runs
        .iter()
        .find(|r| r.run_id != run.run_id && r.run_id.starts_with("run-"))
        .unwrap();
    assert!(requests::list_for_run(&f.db(), &second.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&second.run_id),ToolOutcome::Done {output} if output["content"][0]["text"]=="Bonjour")
    );
    // A different tool still asks: Always is server + tool, not whole server.
    let addition = f.tool("addition");
    let denied = f.send(&addition, json!({"a":1,"b":2}), "addition-denied");
    assert_eq!(denied.status, RunStatus::AwaitingApproval);
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&denied.run_id, json!("deny"));
    f.finish(&denied.run_id);
    assert!(
        matches!(f.outcome(&denied.run_id),ToolOutcome::Failed {reason} if reason.contains("refusée"))
    );
    f.post(
        "/api/bots",
        json!({"id":"autre","name":"Autre bot","color":"citron"}),
    );
    let ctx = f.app.feature_context();
    assert!(!features::mcp::scoped_tools(&features::Context {
        bot_id: Some("autre"),
        ..ctx
    })
    .iter()
    .any(|t| t.name == tool));
    f.post(
        "/api/mcp/select",
        json!({"id":id,"bot_id":"grokbot","enabled":false}),
    );
    let ctx = f.app.feature_context();
    assert!(!features::mcp::scoped_tools(&features::Context {
        bot_id: Some("grokbot"),
        ..ctx
    })
    .iter()
    .any(|t| t.name == tool));
}
#[test]
fn crash_and_launch_errors_are_french_and_restartable() {
    let mut f = Fixture::new();
    let id = f.add("crash");
    let run = f.start(&id);
    f.decide(&run, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run),ToolOutcome::Failed {reason} if reason.contains("déconnecté"))
    );
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "error");
    let restart = f.start(&id);
    assert_eq!(
        store::required(&f.db(), &restart).unwrap().status,
        RunStatus::AwaitingApproval
    );
    f.decide(&restart, json!("deny"));
    let missing = f.post(
        "/api/mcp/add",
        json!({"name":"Absent","command":"/not/a/real/command"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let run = f.start(&missing);
    f.decide(&run, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run),ToolOutcome::Failed {reason} if reason.contains("Impossible de démarrer"))
    );
}
