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
        support::ask_before_acting(&mut app);
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
        wait("approval card", || {
            !requests::list_for_run(&self.db(), run).unwrap().is_empty()
        });
        requests::list_for_run(&self.db(), run).unwrap().remove(0)
    }
    fn decide(&mut self, run: &str, decision: Value) {
        let r = self.request(run);
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":format!("decision-{run}"),"decision":decision}));
        self.app.tick(0);
        self.finish(run);
    }
    fn start(&mut self, id: &str) -> String {
        let run = self.post("/api/mcp/restart", json!({"id":id}))["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.request(&run);
        run
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
    fn tool_card(&mut self, call: &str) -> Value {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["messages"].as_array().unwrap())
            .find(|m| m["kind"] == "tool" && m["payload"]["call"]["call_id"] == call)
            .expect("public tool card")["payload"]
            .clone()
    }
    fn group_peer(&mut self, server: &str) -> String {
        self.post(
            "/api/bots",
            json!({"id":"lecteur","name":"Lecteur","color":"cyan"}),
        );
        self.post(
            "/api/mcp/select",
            json!({"id":server,"bot_id":"lecteur","enabled":true}),
        );
        self.post(
            "/api/conversations",
            json!({"bot_ids":["grokbot","lecteur"],"title":"MCP en groupe"}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn send_peer(&mut self, group: &str, name: &str, text: &str, id: &str) -> Run {
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":json!({"text":text}).to_string()}}),json!({"type":"response.completed"})]);
        self.post("/api/send", json!({"conversation_id":group,"message_id":id,"body":"@Lecteur Appelle le connecteur","mentions":[{"id":"lecteur","label":"Lecteur"}]}));
        wait("peer run", || {
            self.app.tick(0);
            self.db().query_row("SELECT EXISTS(SELECT 1 FROM wrapper_group_turns WHERE root_id=?1 AND bot_id='lecteur' AND run_id IS NOT NULL)",[id],|r|r.get::<_,bool>(0)).unwrap()
        });
        let run: String = self
            .db()
            .query_row(
                "SELECT run_id FROM wrapper_group_turns WHERE root_id=?1 AND bot_id='lecteur'",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        wait("peer approval or result", || {
            let status = store::required(&self.db(), &run).unwrap().status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        store::required(&self.db(), &run).unwrap()
    }
    fn drain_group(&mut self) {
        wait("group turns complete", || {
            self.app.tick(0);
            self.db().query_row("SELECT COUNT(*)=0 FROM wrapper_group_turns WHERE status IN ('queued','running')",[],|r|r.get::<_,bool>(0)).unwrap()
        });
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
fn a_non_coordinator_group_peer_can_use_once_and_standing_mcp_approvals() {
    let mut f = Fixture::new();
    let server = f.add("normal");
    f.connect(&server);
    let echo = f.tool("echo");
    let group = f.group_peer(&server);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT bot_id FROM wrapper_conversations WHERE id=?1",
                [&group],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "grokbot"
    );
    for (id, decision) in [
        ("group-once", json!("allow_once")),
        ("group-always", json!({"always_allow":{"scope":"global"}})),
    ] {
        let run = f.send_peer(&group, &echo, id, id);
        assert_eq!(run.bot_id, "lecteur");
        assert_eq!(f.request(&run.run_id).route.requesting_bot_id, "lecteur");
        f.script(vec![json!({"type":"response.completed"})]);
        f.decide(&run.run_id, decision);
        assert!(
            matches!(f.outcome(&run.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]==id)
        );
        f.drain_group();
    }
    // Only this bot/server/tool standing rule authorizes the next group turn.
    let next = f.send_peer(&group, &echo, "pair automatique", "group-automatic");
    f.finish(&next.run_id);
    f.drain_group();
    assert!(requests::list_for_run(&f.db(), &next.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&next.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="pair automatique")
    );
    assert_eq!(
        f.tool_card("group-automatic")["authorized_by"]["kind"],
        "rule"
    );
    assert_eq!(f.tool_card("group-automatic")["requesting_bot"], "Lecteur");
}

#[test]
fn a_removed_group_peer_cannot_send_an_already_admitted_queued_mcp_call() {
    let mut f = Fixture::new();
    let gate = f.path.with_extension("release");
    let server = f.post("/api/mcp/add",json!({"name":"Barrière locale","command":"python3","arguments":[format!("{}/tests/fixtures/mcp_server.py",env!("CARGO_MANIFEST_DIR"))],"environment":{"CALL_GATE":gate.to_str().unwrap()}}))["id"].as_str().unwrap().to_owned();
    f.connect(&server);
    let echo = f.tool("echo");
    let group = f.group_peer(&server);
    let blocker = f.send(&echo, json!({"text":"wait-for-release"}), "hold-client");
    let approval = f.request(&blocker.run_id);
    // The blocker cannot request its model continuation until the gate opens;
    // do not enqueue that continuation ahead of the peer's initial tool call.
    f.post("/api/approvals/resolve",json!({"entry_id":approval.entry_id,"conversation_id":approval.route.target_conversation_id,"submission_id":"hold","decision":"allow_once"}));
    f.app.tick(0);
    wait("client held", || {
        f.state()["mcp"]["servers"][0]["status"] == "busy"
    });
    let peer = f.send_peer(&group, &echo, "must-not-run", "removed-peer");
    let approval = f.request(&peer.run_id);
    f.post("/api/approvals/resolve",json!({"entry_id":approval.entry_id,"conversation_id":approval.route.target_conversation_id,"submission_id":"queued-peer","decision":"allow_once"}));
    f.app.tick(0);
    wait("peer admitted before removal", || {
        f.db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM engine_tool_admissions WHERE run_id=?1)",
                [&peer.run_id],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    });
    // Persisted membership changes while the call waits for the busy client.
    // Keep the run Running to exercise the adapter's independent membership guard.
    f.db()
        .execute(
            "UPDATE wrapper_members SET active=0 WHERE conversation_id=?1 AND bot_id='lecteur'",
            [&group],
        )
        .unwrap();
    std::fs::write(&gate, b"release").unwrap();
    f.finish(&blocker.run_id);
    f.finish(&peer.run_id);
    f.drain_group();
    assert!(
        matches!(f.outcome(&peer.run_id),ToolOutcome::Failed{reason} if reason.contains("n’a pas été autorisée"))
    );
    let probe = f.send(&echo, json!({"text":"call-count"}), "count-after-removal");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&probe.run_id, json!("allow_once"));
    // The server saw only the blocker and this probe, never the removed peer.
    assert!(
        matches!(f.outcome(&probe.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="2")
    );
}

#[test]
fn slow_calls_initialization_and_catalogue_refresh_leave_reads_writes_and_stop_responsive() {
    use std::time::{Duration, Instant};
    for mode in ["normal", "slow-start", "slow-list"] {
        let mut f = Fixture::new();
        let id = f.add(mode);
        let (run, message) = if mode == "slow-start" {
            (f.start(&id), None)
        } else {
            f.connect(&id);
            let tool = f.tool("echo");
            let text = if mode == "normal" {
                "slow-two-seconds"
            } else {
                "Bonjour"
            };
            let run = f.send(&tool, json!({"text":text}), "slow-ui");
            f.script(vec![json!({"type":"response.completed"})]);
            (run.run_id, Some("slow-ui"))
        };
        let request = f.request(&run);
        f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"slow-admit","decision":"allow_once"}));
        let before = Instant::now();
        f.app.tick(0);
        assert!(before.elapsed() < Duration::from_millis(200), "tick {mode}");
        if mode == "slow-list" {
            f.finish(&run);
            f.app.tick(0);
        }
        wait("busy MCP", || {
            if mode == "slow-list" {
                f.app.tick(0);
            }
            matches!(
                f.state()["mcp"]["servers"][0]["status"].as_str(),
                Some("busy" | "starting")
            )
        });
        for _ in 0..3 {
            let before = Instant::now();
            f.app.api_response("GET", "/api/state", b"").unwrap();
            assert!(
                before.elapsed() < Duration::from_millis(200),
                "GET /api/state {mode}"
            );
        }
        let before = Instant::now();
        let other = f.post("/api/conversations", json!({"title":"Pendant MCP"}))["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            before.elapsed() < Duration::from_millis(200),
            "new conversation {mode}"
        );
        f.script(vec![json!({"type":"response.completed"})]);
        let before = Instant::now();
        f.post(
            "/api/send",
            json!({"conversation_id":other,"message_id":"during-mcp","body":"Bonjour pendant MCP"}),
        );
        assert!(before.elapsed() < Duration::from_millis(200), "send {mode}");
        if mode == "normal" {
            let before = Instant::now();
            f.post(
                "/api/stop",
                json!({"conversation_id":f.conversation,"message_id":message.unwrap()}),
            );
            assert!(before.elapsed() < Duration::from_millis(200), "stop");
            assert_eq!(
                store::required(&f.db(), &run).unwrap().status,
                RunStatus::Cancelled
            );
        }
        f.finish(&run);
        if mode == "slow-list" {
            wait("catalogue idle", || {
                f.state()["mcp"]["servers"][0]["status"] == "connected"
            });
        }
    }
}

#[test]
fn short_settings_do_not_corrupt_other_connectors_or_json_scalar_types() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    f.post("/api/mcp/add",json!({"name":"Réglages inactifs","command":"python3","arguments":["script1.py"],"environment":{"DEBUG":"1","PORT":"3000","VERBOSE":"false"}}));
    let addition = f.tool("addition");
    let run = f.send(&addition, json!({"a":1000,"b":2000}), "numeric-short");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="3000")
    );
    let name = format!("MCP_{id}_ENV").to_uppercase();
    features::forms::vault(&f.app.feature_context()).unwrap().set(&name,grokbot_forms::vault::MCP_ENV_PURPOSE,&json!({"variables":{"DEBUG":"1","PORT":"3000","VERBOSE":"false","TEST_TOKEN":"jeton-prive-test-489","NUMERIC_TOKEN":"30000000"}}).to_string()).unwrap();
    let restart = f.start(&id);
    f.decide(&restart, json!("allow_once"));
    let echo = f.tool("echo");
    let run = f.send(&echo, json!({"text":"typed"}), "typed-short");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done{output} if output["isError"]==false && output["structuredContent"]["port"]==3000 && output["structuredContent"]["flag"]==false)
    );
    let run = f.send(&echo, json!({"text":"env:VERBOSE"}), "own-short");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="[masqué]" && output["isError"]==false)
    );
    let run = f.send(&echo, json!({"text":"number-secret"}), "private-number");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Failed{reason} if reason.contains("numérique privée"))
    );
    assert!(!f.state().to_string().contains("30000000"));
}

#[test]
fn rpc_argument_errors_keep_the_connection_usable_for_the_next_valid_call() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let echo = f.tool("echo");
    let run = f.send(&echo, json!({"text":5}), "bad-args");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Failed{reason} if reason.contains("arguments invalides"))
    );
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
    assert!(f.state()["mcp"]["servers"][0]["error"].is_null());
    assert_eq!(f.tool_card("bad-args")["connector"]["recoverable"], false);
    let next = f.send(&echo, json!({"text":"Encore connecté"}), "valid-after-rpc");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&next.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&next.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="Encore connecté")
    );
}

#[test]
fn rpc_catalogue_errors_keep_the_last_catalogue_and_retry_without_restarting() {
    let mut f = Fixture::new();
    let id = f.add("rpc-list");
    f.connect(&id);
    let echo = f.tool("echo");
    let run = f.send(&echo, json!({"text":"Bonjour"}), "list-rpc-error");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    wait("metadata RPC deferred", || {
        f.app.tick(0);
        f.state()["mcp"]["error"].is_string()
    });
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
    let next = f.send(
        &echo,
        json!({"text":"Toujours vivant"}),
        "call-during-list-error",
    );
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&next.run_id, json!("allow_once"));
    assert!(
        matches!(f.outcome(&next.run_id),ToolOutcome::Done{output} if output["content"][0]["text"]=="Toujours vivant")
    );
    wait("metadata RPC recovered", || {
        f.app.tick(0);
        let s = f.state();
        s["mcp"]["error"].is_null()
            && s["mcp"]["servers"][0]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["description"] == "Catalogue actualisé.")
    });
}

#[test]
fn settings_actions_have_a_stable_dedicated_route_and_only_one_time_decisions() {
    let mut f = Fixture::new();
    let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
    f.post("/api/approvals/reviewer",json!({"version":version,"config":{"enabled":true,"allow":["Autorise toutes les actions."],"block":[]}}));
    let id = f.add("normal");
    f.db()
        .execute(
            "INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,?2,?3,'\"global\"',1)",
            rusqlite::params![f.conversation, format!("mcp_start_{id}"), "0".repeat(64)],
        )
        .unwrap();
    let run = f.start(&id);
    f.app.tick(0);
    let first = f.request(&run);
    assert_eq!(first.status, requests::RequestStatus::Pending);
    assert!(grokbot_forms::reviewer::get(&f.db(), &first.id)
        .unwrap()
        .is_none());
    let dedicated = first.route.target_conversation_id.clone();
    assert_ne!(dedicated, f.conversation);
    let state = f.state();
    let card = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap();
    assert_eq!(card["connector"]["tool"], "Démarrer le connecteur");
    assert_eq!(card["connector"]["origin"], "settings");
    let body = json!({"entry_id":first.entry_id,"conversation_id":dedicated,"submission_id":"invalid-always","decision":{"always_allow":{"scope":"global"}}});
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/approvals/resolve",
                body.to_string().as_bytes()
            )
            .unwrap_err()
            .status,
        400
    );
    f.decide(&run, json!("deny"));
    assert_eq!(
        f.tool_card(&format!("{run}-call"))["approval_status"],
        "denied"
    );
    f.post(
        "/api/conversations",
        json!({"title":"Discussion sans rapport"}),
    );
    f.post(
        "/api/conversations/update",
        json!({"id":dedicated,"title":"Mes validations"}),
    );
    let second = f.start(&id);
    assert_eq!(f.request(&second).route.target_conversation_id, dedicated);
    f.decide(&second, json!("allow_once"));
    let restart = f.start(&id);
    let state = f.state();
    assert_eq!(
        state["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["run_id"] == restart)
            .unwrap()["connector"]["tool"],
        "Redémarrer le connecteur"
    );
    f.decide(&restart, json!("deny"));
    f.post("/api/conversations/delete", json!({"id":dedicated}));
    let next = f.start(&id);
    assert_ne!(f.request(&next).route.target_conversation_id, dedicated);
    f.decide(&next, json!("deny"));
}

#[test]
fn model_guessed_management_names_never_create_an_approval_and_unknown_tools_keep_the_server_label()
{
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let server = f.state()["mcp"]["servers"][0].clone();
    let run=f.send(&format!("mcp_remove_{id}"),json!({"serveur":server["name"],"commande":server["command"],"arguments":server["arguments"],"variables_privées":server["environment_names"],"action":"Supprimer le connecteur"}),"guessed-management");
    assert!(requests::list_for_run(&f.db(), &run.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&run.run_id),ToolOutcome::Failed{reason} if reason.contains("inconnu"))
    );
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "connected");
    let echo = f.tool("echo");
    let invalid = f.send(&echo, json!("incomplets"), "non-object-mcp");
    assert!(requests::list_for_run(&f.db(), &invalid.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&invalid.run_id),ToolOutcome::Failed{reason} if reason.contains("Arguments MCP invalides"))
    );
    let unknown = f.send(
        &format!("mcp_{id}_ffffffffffffffff"),
        json!({}),
        "unknown-known-server",
    );
    assert!(requests::list_for_run(&f.db(), &unknown.run_id)
        .unwrap()
        .is_empty());
    let card = f.tool_card("unknown-known-server");
    assert_eq!(
        card["connector"],
        json!({"server":"Atelier local","tool":"outil inconnu","recoverable":false})
    );
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
    wait("deferred catalogue", || {
        f.app.tick(0);
        f.state()["mcp"]["error"].is_string()
    });
    let deferred = f.state();
    assert_eq!(deferred["mcp"]["servers"].as_array().unwrap().len(), 1);
    assert!(deferred["mcp"]["error"]
        .as_str()
        .unwrap()
        .contains("différée"));
    f.db()
        .execute_batch("DROP TRIGGER fail_mcp_catalogue;")
        .unwrap();
    wait("retried catalogue", || {
        f.app.tick(0);
        let s = f.state();
        s["mcp"]["error"].is_null()
            && s["mcp"]["servers"][0]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["description"] == "Catalogue actualisé.")
    });
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
    wait("catalogue notification", || {
        f.app.tick(0);
        f.state()["mcp"]["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["description"] == "Catalogue actualisé.")
    });
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
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "crashed");
    let absent = f.send(&tool, json!({"text":"Bonjour"}), "disconnected-call");
    assert!(requests::list_for_run(&f.db(), &absent.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&absent.run_id),ToolOutcome::Failed{reason} if reason.contains("inconnu"))
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
    let ctx = f.app.feature_context();
    assert!(!features::mcp::scoped_tools(&features::Context {
        bot_id: Some("grokbot"),
        ..ctx
    })
    .iter()
    .any(|t| t.name.starts_with(&format!("mcp_{id}_"))));
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
    assert_eq!(f.tool_card("next")["authorized_by"]["kind"], "rule");
    assert_eq!(f.tool_card("next")["requesting_bot"], "Ezerbot");
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
fn outbound_and_destructive_connector_actions_ignore_permanent_rules() {
    let mut f = Fixture::new();
    let id = f.add("protected");
    f.connect(&id);
    for name in [
        "send_email",
        "post_message",
        "create_comment",
        "delete_file",
        "sendEmail",
        "deleteFile",
        "chat.postMessage",
    ] {
        let tool = f.tool(name);
        let first_id = format!("ask-{name}");
        let run = f.send(&tool, json!({"text":"bonjour"}), &first_id);
        assert_eq!(run.status, RunStatus::AwaitingApproval, "{name}");
        let card = f.request(&run.run_id);
        let state = f.state();
        let payload = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["messages"].as_array().unwrap().clone())
            .find(|m| m["id"] == card.entry_id)
            .expect("carte")["payload"]
            .clone();
        assert_eq!(payload["policy"]["remember"], false, "{name}");
        let expected = if name.to_ascii_lowercase().contains("delete") {
            "destructive"
        } else {
            "outbound"
        };
        assert_eq!(payload["policy"]["protection"], expected, "{name}");
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{name}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"global"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{name}");
        f.db().execute(
            "INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,?2,'','\"global\"',1)",
            rusqlite::params![f.conversation, tool],
        ).unwrap();
        f.script(vec![json!({"type":"response.completed"})]);
        f.decide(&run.run_id, json!("allow_once"));
        assert!(
            matches!(f.outcome(&run.run_id), ToolOutcome::Done { .. }),
            "{name}"
        );
        let second_id = format!("again-{name}");
        let again = f.send(&tool, json!({"text":"encore"}), &second_id);
        assert_eq!(
            again.status,
            RunStatus::AwaitingApproval,
            "{name} reste protégé"
        );
        assert!(store::events(&f.db(), &again.run_id, 0)
            .unwrap()
            .iter()
            .all(|e| e.kind != RunEventKind::ToolResult));
        f.script(vec![json!({"type":"response.completed"})]);
        f.decide(&again.run_id, json!("deny"));
    }
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
    assert_eq!(f.state()["mcp"]["servers"][0]["status"], "crashed");
    assert_eq!(
        f.tool_card(&format!("{run}-call"))["connector"]["recoverable"],
        true
    );
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

/// What a remote server saw: the Authorization header and the JSON-RPC method.
type Exchanges = Arc<Mutex<Vec<(Option<String>, String)>>>;
/// A Streamable HTTP MCP server on 127.0.0.1 that records what it is asked.
struct RemoteServer {
    base: String,
    server: Arc<tiny_http::Server>,
    seen: Exchanges,
}
impl RemoteServer {
    /// `redirect_to` answers every request with a 302 instead of serving tools.
    fn start(redirect_to: Option<String>) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let base = format!(
            "http://127.0.0.1:{}",
            server.server_addr().to_ip().unwrap().port()
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (listener, log) = (server.clone(), seen.clone());
        std::thread::spawn(move || {
            for mut request in listener.incoming_requests() {
                let mut body = String::new();
                std::io::Read::read_to_string(request.as_reader(), &mut body).unwrap();
                let frame: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
                let authorization = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.to_string());
                let method = frame["method"].as_str().unwrap_or("").to_owned();
                log.lock().unwrap().push((authorization, method.clone()));
                if let Some(target) = &redirect_to {
                    let location =
                        tiny_http::Header::from_bytes("Location", target.as_bytes()).unwrap();
                    let _ = request.respond(tiny_http::Response::empty(302).with_header(location));
                    continue;
                }
                let result = match method.as_str() {
                    "initialize" => {
                        json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"Distant","version":"1"}})
                    }
                    "tools/list" => {
                        json!({"tools":[{"name":"echo","description":"Répéter un texte.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]})
                    }
                    "tools/call" => {
                        json!({"content":[{"type":"text","text":frame["params"]["arguments"]["text"]}]})
                    }
                    _ => {
                        let _ = request.respond(tiny_http::Response::empty(202));
                        continue;
                    }
                };
                let json_type =
                    tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap();
                let _ = request.respond(
                    tiny_http::Response::from_string(
                        json!({"jsonrpc":"2.0","id":frame["id"],"result":result}).to_string(),
                    )
                    .with_header(json_type),
                );
            }
        });
        Self { base, server, seen }
    }
    fn calls(&self, method: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, m)| m == method)
            .count()
    }
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn authorizations(&self) -> Vec<Option<String>> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|(a, _)| a.clone())
            .collect()
    }
}
impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.server.unblock();
    }
}
impl Fixture {
    fn try_post(&mut self, path: &str, body: Value) -> Result<Value, grokbot_wrapper::ApiError> {
        self.app.api("POST", path, body.to_string().as_bytes())
    }
    fn add_url(&mut self, url: &str, token: &str, on_this_mac: bool) -> String {
        self.post(
            "/api/mcp/add",
            json!({"name":"Serveur distant","url":url,"token":token,"on_this_mac":on_this_mac}),
        )["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    /// The SQLite file and its journals, as text, to look for a secret. The test
    /// vault is a disposable file next to them and is deliberately not read.
    fn database_text(&self) -> String {
        let name = self.path.file_name().unwrap().to_str().unwrap().to_owned();
        let mut all = String::new();
        for entry in std::fs::read_dir(self.path.parent().unwrap()).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name().to_str().unwrap().starts_with(&name) {
                let bytes = std::fs::read(entry.path()).unwrap_or_default();
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        all
    }
}

#[test]
fn only_https_or_confirmed_loopback_addresses_are_accepted() {
    let mut f = Fixture::new();
    for (url, confirmed, expected) in [
        ("http://exemple.fr/mcp", false, "https://"),
        ("http://exemple.fr/mcp", true, "https://"),
        ("http://localhost:8080/mcp", true, "https://"),
        ("http://127.0.0.1:8080/mcp", false, "sur ce Mac"),
        ("ftp://exemple.fr/mcp", false, "https://"),
        ("exemple.fr/mcp", false, "invalide"),
        (
            "https://utilisateur:mot@exemple.fr/mcp",
            false,
            "identifiants",
        ),
        ("https://exemple.fr/mcp?cle=abc", false, "paramètres"),
    ] {
        let error = f
            .try_post(
                "/api/mcp/add",
                json!({"name":"Distant","url":url,"on_this_mac":confirmed}),
            )
            .unwrap_err();
        assert!(error.message.contains(expected), "{url}: {}", error.message);
    }
    assert!(f.state()["mcp"]["servers"].as_array().unwrap().is_empty());
    let id = f.add_url("https://exemple.fr/mcp", "", false);
    let server = &f.state()["mcp"]["servers"][0];
    assert_eq!(server["id"], id);
    assert_eq!(server["url"], "https://exemple.fr/mcp");
    let local = f.post(
        "/api/mcp/add",
        json!({"name":"Sur ce Mac","url":"http://127.0.0.1:8080/mcp","on_this_mac":true}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(f.state()["mcp"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == local && s["url"] == "http://127.0.0.1:8080/mcp"));
    let error = f
        .try_post(
            "/api/mcp/add",
            json!({"name":"Mixte","url":"https://exemple.fr/mcp","command":"python3"}),
        )
        .unwrap_err();
    assert!(error.message.contains("pas les deux"));
    let error = f
        .try_post(
            "/api/mcp/add",
            json!({"name":"Local","command":"python3","token":"jeton-secret-bravo"}),
        )
        .unwrap_err();
    assert!(error.message.contains("par adresse"));
    // A token too short to be masked inside a sentence is refused, not stored.
    let error = f
        .try_post(
            "/api/mcp/add",
            json!({"name":"Court","url":"https://exemple.fr/mcp","token":"abc123"}),
        )
        .unwrap_err();
    assert!(error.message.contains("trop court"), "{}", error.message);
    assert!(!f.state()["mcp"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["name"] == "Court"));
}

#[test]
fn a_remote_server_keeps_its_secret_in_the_vault_and_asks_before_every_new_tool() {
    let remote = RemoteServer::start(None);
    let mut f = Fixture::new();
    let secret = "jeton-distant-charlie";
    let id = f.add_url(&format!("{}/mcp", remote.base), secret, true);
    // Saved, not contacted: the secret is nowhere but the vault, and not shown again.
    assert_eq!(remote.requests(), 0);
    assert!(!f.state().to_string().contains(secret));
    assert!(!f.database_text().contains(secret));
    assert_eq!(
        features::forms::vault(&f.app.feature_context())
            .unwrap()
            .list()
            .unwrap()
            .len(),
        1
    );
    f.connect(&id);
    assert!(remote
        .authorizations()
        .iter()
        .all(|a| a.as_deref() == Some(&format!("Bearer {secret}")[..])));
    let tool = f.tool("echo");
    assert!(tool.starts_with(&format!("mcp_{id}_")));
    // Discovery listed the tool; calling it opens the approval and calls nothing.
    assert_eq!(remote.calls("tools/call"), 0);
    let run = f.send(&tool, json!({"text":"Bonjour"}), "remote-first");
    assert_eq!(run.status, RunStatus::AwaitingApproval);
    assert!(
        matches!(f.request(&run.run_id).kind, requests::InputKind::Approval { tool: asked, .. } if asked == tool)
    );
    assert_eq!(
        f.tool_card("remote-first")["call"]["arguments"]["text"],
        "Bonjour"
    );
    assert_eq!(remote.calls("tools/call"), 0);
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("deny"));
    f.finish(&run.run_id);
    assert_eq!(remote.calls("tools/call"), 0);
    // Accepting is what calls the server, once.
    let run = f.send(&tool, json!({"text":"Bonjour"}), "remote-second");
    assert_eq!(remote.calls("tools/call"), 0);
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run.run_id, json!("allow_once"));
    f.finish(&run.run_id);
    assert_eq!(remote.calls("tools/call"), 1);
    assert!(
        matches!(f.outcome(&run.run_id), ToolOutcome::Done { output } if output["content"][0]["text"] == "Bonjour")
    );
    let bodies =
        f.ai.seen
            .lock()
            .unwrap()
            .bodies
            .iter()
            .map(Value::to_string)
            .collect::<String>();
    assert!(!bodies.contains(secret));
    assert!(!f.state().to_string().contains(secret));
    assert!(!f.database_text().contains(secret));
}

#[test]
fn a_remote_server_without_a_token_receives_no_authorization_header() {
    let remote = RemoteServer::start(None);
    let mut f = Fixture::new();
    let id = f.add_url(&format!("{}/mcp", remote.base), "", true);
    f.connect(&id);
    assert!(remote.requests() > 0);
    assert!(remote.authorizations().iter().all(Option::is_none));
}

#[test]
fn a_redirect_is_refused_and_the_token_never_reaches_another_host() {
    let elsewhere = RemoteServer::start(None);
    let remote = RemoteServer::start(Some(format!("{}/mcp", elsewhere.base)));
    let mut f = Fixture::new();
    let id = f.add_url(
        &format!("{}/mcp", remote.base),
        "jeton-redirige-delta",
        true,
    );
    f.post(
        "/api/mcp/select",
        json!({"id":id,"bot_id":"grokbot","enabled":true}),
    );
    let run = f.start(&id);
    f.decide(&run, json!("allow_once"));
    let server = f.state()["mcp"]["servers"][0].clone();
    assert_ne!(server["status"], "connected");
    assert!(
        server["error"].as_str().unwrap().contains("redirection"),
        "{server}"
    );
    assert!(server["tools"].as_array().unwrap().is_empty());
    assert!(remote.requests() > 0);
    assert_eq!(elsewhere.requests(), 0);
}

#[test]
fn yolo_never_reopens_a_connector_a_bot_turned_off_and_keeps_the_lifecycle_card() {
    let mut f = Fixture::new();
    let id = f.add("normal");
    f.connect(&id);
    let echo = f.tool("echo");
    f.post("/api/yolo/mode", json!({"mode":"yolo"}));
    // Outils activés : le Mode YOLO lève la carte, comme avant.
    let run = f.send(&echo, json!({"text":"yolo"}), "echo-yolo");
    f.finish(&run.run_id);
    assert!(requests::list_for_run(&f.db(), &run.run_id)
        .unwrap()
        .is_empty());
    assert!(
        matches!(f.outcome(&run.run_id), ToolOutcome::Done { output } if output["content"][0]["text"] == "yolo")
    );
    let card = f.tool_card("echo-yolo");
    assert_eq!(card["authorized_by"]["kind"], "yolo");
    assert_eq!(card["connector"]["server"], "Atelier local");
    // Outils décochés : le Mode YOLO ne les rouvre pas, l'appel est refusé.
    f.post(
        "/api/mcp/select",
        json!({"id":id,"bot_id":"grokbot","enabled":false}),
    );
    let refused = f.send(&echo, json!({"text":"yolo-off"}), "echo-yolo-off");
    f.finish(&refused.run_id);
    assert!(
        matches!(f.outcome(&refused.run_id), ToolOutcome::Failed { reason } if reason.contains("désactivé")),
    );
    // Redémarrer un connecteur reste une décision de l'utilisateur.
    let restart = f.start(&id);
    assert_eq!(
        requests::list_for_run(&f.db(), &restart).unwrap()[0].status,
        requests::RequestStatus::Pending
    );
}
