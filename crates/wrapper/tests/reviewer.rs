//! Production forms gate + real loopback Responses transport, without external effects.
mod support;
use grokbot_engine::{
    contracts::*,
    model::{self, Run, RunStatus},
    store,
};
use grokbot_forms::{
    requests::{self, RequestStatus},
    reviewer,
    rules::AutoReviewConfig,
};
use grokbot_wrapper::{
    features::{self, forms, Context},
    App, Provider,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    conversation: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "ezerbot-reviewer-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path.with_extension("chatgpt-auth.json"), json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        support::ask_before_acting(&mut app);
        wait("catalog", || provider.status()["models_ready"] == true);
        let conversation = app
            .api("POST", "/api/conversations", br#"{"title":"Relecteur"}"#)
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .into();
        let mut fixture = Self {
            app,
            provider,
            server,
            path,
            conversation,
        };
        fixture.config(
            true,
            "Autorise la lecture dans ~/Documents/projets, mais demande avant toute suppression.",
        );
        fixture
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn config(&mut self, enabled: bool, text: &str) -> Value {
        let version =
            self.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
        self.app.api("POST", "/api/approvals/reviewer", json!({"version":version,"config":{"enabled":enabled,"allow":[text],"block":["Refuse la suppression des archives."]}}).to_string().as_bytes()).unwrap()
    }
    fn pending(&self, id: &str, args: Value) -> String {
        let conn = self.db();
        let tx = features::write_transaction(&conn).unwrap();
        let run = model::start(
            &tx,
            &Run {
                run_id: id.into(),
                bot_id: "grokbot".into(),
                conversation_id: self.conversation.clone(),
                trigger: Trigger::UserMessage {
                    message_id: format!("msg-{id}"),
                    attempt: 0,
                },
                kind: RunKind::Chat,
                status: RunStatus::Running,
                task_status: None,
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at: now() as i64,
            },
        )
        .unwrap();
        tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,1,'gpt-test')",params![id,json!({"entry_id":format!("msg-{id}"),"role":"user","text":"Lire un fichier.","images":[],"tool_items":[]}).to_string()]).unwrap();
        let ctx = Context {
            connection: &tx,
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
            ..self.app.feature_context()
        };
        let call = ToolCall {
            call_id: format!("call-{id}"),
            // Generic SQL-only protected tool, not the actual local file API.
            name: "test_read_resource".into(),
            arguments: args,
        };
        assert_eq!(
            forms::check_tool_approval(&ctx, &run, &call).unwrap(),
            ApprovalCheck::Ask
        );
        let cause = forms::request_tool_approval(&ctx, &run, &call).unwrap();
        let SuspendCause::HumanInput { id: request_id } = &cause else {
            panic!()
        };
        let mut suspended = run.clone();
        suspended.status = RunStatus::AwaitingApproval;
        store::save(&tx, &suspended).unwrap();
        store::append_event(
            &tx,
            &run.run_id,
            RunEventKind::Suspended,
            &json!({"call_id":call.call_id,"cause":cause,"approval":true}),
        )
        .unwrap();
        tx.commit().unwrap();
        request_id.clone()
    }
    fn script(&self, reply: ScriptedResponse) {
        self.server.seen.lock().unwrap().responses.push_back(reply);
    }
    fn answer(&self, decision: &str) {
        self.script(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":json!({"decision":decision,"reason":"Cette action correspond à votre règle."}).to_string()}),
            json!({"type":"response.completed"})]));
    }
    fn review(&self, id: &str) -> reviewer::ReviewRecord {
        reviewer::get(&self.db(), id).unwrap().unwrap()
    }
    fn tick_review(&self, id: &str) {
        forms::tick(&self.app.feature_context());
        wait("review complete", || {
            reviewer::get(&self.db(), id)
                .unwrap()
                .is_some_and(|r| !r.pending)
        });
    }
    fn status(&self, id: &str) -> RequestStatus {
        requests::get(&self.db(), id).unwrap().unwrap().status
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn allow_deny_and_ask_are_bound_audited_and_persistent() {
    for decision in ["allow", "deny", "ask"] {
        let mut f = Fixture::new();
        let id = f.pending(decision, json!({"path":"~/Documents/projets/a.txt"}));
        let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
        f.answer(decision);
        f.tick_review(&id);
        assert_eq!(json!(f.review(&id).answer.decision), decision);
        assert_eq!(json!(f.review(&id).version), version);
        match decision {
            "allow" => assert!(matches!(f.status(&id), RequestStatus::Resolved { .. })),
            "deny" => assert!(matches!(f.status(&id), RequestStatus::Denied)),
            _ => assert_eq!(f.status(&id), RequestStatus::Pending),
        }
        let conn = f.db();
        assert_eq!(
            store::pending_wakes(&conn).unwrap().len(),
            if decision == "ask" { 0 } else { 1 }
        );
        let grants: i64 = conn
            .query_row("SELECT count(*) FROM forms_approval_grants", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(grants, if decision == "allow" { 1 } else { 0 });
        let bodies = f.server.seen.lock().unwrap().bodies.clone();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["tools"], json!([]));
        assert!(!bodies[0].to_string().contains("Lire un fichier.")); // no chat history
        f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
        assert_eq!(json!(f.review(&id).version), version);
        forms::tick(&f.app.feature_context());
        assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 1);
        if decision != "ask" {
            let conn = f.db();
            let tx = features::write_transaction(&conn).unwrap();
            let run = model::resume(&tx, decision).unwrap();
            let ctx = Context {
                connection: &tx,
                bot_id: Some(&run.bot_id),
                conversation_id: Some(&run.conversation_id),
                run_id: Some(&run.run_id),
                ..f.app.feature_context()
            };
            let call = ToolCall {
                call_id: format!("call-{decision}"),
                name: "test_read_resource".into(),
                arguments: json!({"path":"~/Documents/projets/a.txt"}),
            };
            let check = forms::check_tool_approval(&ctx, &run, &call).unwrap();
            if decision == "allow" {
                assert!(matches!(
                    check,
                    ApprovalCheck::Authorized {
                        by: AuthorizedBy::AllowOnce { .. }
                    }
                ));
                assert!(matches!(
                    forms::check_tool_approval(&ctx, &run, &call).unwrap(),
                    ApprovalCheck::Denied { .. }
                ));
            } else {
                assert_eq!(
                    check,
                    ApprovalCheck::Denied {
                        reason: f.review(&id).answer.reason
                    }
                );
            }
            tx.commit().unwrap();
        }
    }
}
#[test]
fn malformed_refusal_incomplete_and_http_error_never_grant() {
    for response in [
        ScriptedResponse::json(500, json!({"error":{"message":"private error"}})),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Oui\"}"}),
            json!({"type":"response.output_text.delta","delta":42}),
            json!({"type":"response.completed"}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Oui\"}"}),
            json!({"type":"response.completed","response":{"status":"incomplete"}}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Oui\"}"}),
            json!({"type":"response.completed"}),
            json!({"type":"response.completed"}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\"}"}),
            json!({"type":"response.completed"}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.refusal.delta","delta":"Non"}),
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Oui\"}"}),
            json!({"type":"response.completed"}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Oui\"}"}),
        ]),
    ] {
        let f = Fixture::new();
        let id = f.pending("error", json!({"path":"a"}));
        f.script(response);
        f.tick_review(&id);
        assert_eq!(f.review(&id).answer.decision, reviewer::ReviewDecision::Ask);
        assert_eq!(f.status(&id), RequestStatus::Pending);
        assert!(!f.review(&id).answer.reason.contains("private error"));
        assert!(store::pending_wakes(&f.db()).unwrap().is_empty());
    }
}
#[test]
fn disabled_and_sensitive_calls_never_reach_the_model() {
    let mut f = Fixture::new();
    f.config(false, "Autorise la lecture.");
    let disabled = f.pending("disabled", json!({"path":"a"}));
    forms::tick(&f.app.feature_context());
    assert!(reviewer::get(&f.db(), &disabled).unwrap().is_none());
    f.config(true, "Autorise la lecture.");
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set("TEST_TOKEN", "Test", "secret-factice-993311")
        .unwrap();
    for (id, args) in [
        ("vault", json!({"path":"secret-factice-993311"})),
        ("inline", json!({"token":"unknown-secret"})),
        ("header", json!({"api-key":"unknown-secret"})),
        (
            "url",
            json!({"url":"https://user:unknown-secret@example.invalid/?key=unknown-secret"}),
        ),
        ("content", json!({"content":"unknown-secret"})),
        (
            "command",
            json!({"command":"curl --password unknown-secret"}),
        ),
    ] {
        let request = f.pending(id, args);
        f.tick_review(&request);
        assert_eq!(
            f.review(&request).answer.decision,
            reviewer::ReviewDecision::Ask
        );
    }
    // Enabling is future-only: the older card never becomes an auto-approval.
    assert_eq!(
        f.review(&disabled).answer.decision,
        reviewer::ReviewDecision::Ask
    );
    let bodies = f.server.seen.lock().unwrap().bodies.clone();
    assert!(bodies.is_empty());
    assert!(bodies
        .iter()
        .all(|b| !b.to_string().contains("secret-factice-993311")
            && !b.to_string().contains("unknown-secret")));
    let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
    assert!(f.app.api("POST","/api/approvals/reviewer",json!({"version":version,"config":{"enabled":true,"allow":["secret-factice-993311"],"block":[]}}).to_string().as_bytes()).is_err());
}
#[test]
fn all_twenty_unicode_rules_per_list_fit_the_api_budget() {
    let mut f = Fixture::new();
    let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
    let config = AutoReviewConfig {
        enabled: false,
        allow: vec!["é🔒".repeat(500); 20],
        block: vec!["é🔒".repeat(500); 20],
    };
    let body = json!({"config":config,"version":version}).to_string();
    assert!(body.len() > 64 * 1024);
    assert!(f
        .app
        .api("POST", "/api/approvals/reviewer", body.as_bytes())
        .is_ok());
}

#[test]
fn settings_reject_stale_writes_and_survive_restart() {
    let mut f = Fixture::new();
    let first = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap();
    f.config(true, "Demande avant toute suppression.");
    assert!(f
        .app
        .api(
            "POST",
            "/api/approvals/reviewer",
            first.to_string().as_bytes()
        )
        .is_err());
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(
        f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["config"]["allow"][0],
        "Demande avant toute suppression."
    );
}
#[test]
fn edits_and_manual_decisions_win_over_inflight_allow() {
    for mode in ["edit", "manual", "standing"] {
        let mut f = Fixture::new();
        let id = f.pending("race", json!({"path":"a"}));
        f.server.seen.lock().unwrap().response_delay = Some(Duration::from_millis(300));
        f.answer("allow");
        forms::tick(&f.app.feature_context());
        wait("review request received", || {
            !f.server.seen.lock().unwrap().bodies.is_empty()
        });
        if mode == "edit" {
            f.config(true, "Demande toujours confirmation.");
        } else if mode == "manual" {
            let request = requests::get(&f.db(), &id).unwrap().unwrap();
            f.app.api("POST","/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":f.conversation,"submission_id":"manual","decision":"deny"}).to_string().as_bytes()).unwrap();
        } else {
            let request = requests::get(&f.db(), &id).unwrap().unwrap();
            let grokbot_forms::requests::InputKind::Approval {
                tool, args_digest, ..
            } = request.kind
            else {
                panic!()
            };
            f.db().execute("INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,?2,?3,'\"subject\"',0)",params![f.conversation,tool,args_digest]).unwrap();
        }
        wait("review complete", || !f.review(&id).pending);
        assert_eq!(f.review(&id).answer.decision, reviewer::ReviewDecision::Ask);
        assert!(f
            .db()
            .query_row("SELECT count(*)=0 FROM forms_approval_grants", [], |r| r
                .get::<_, bool>(
                0
            ))
            .unwrap());
        if mode == "edit" {
            assert_eq!(f.status(&id), RequestStatus::Pending);
        } else {
            assert!(matches!(f.status(&id), RequestStatus::Denied));
        }
    }
}
#[test]
fn timeout_falls_back_without_holding_the_database_lock() {
    let mut f = Fixture::new();
    let id = f.pending("timeout", json!({"path":"a"}));
    f.server.seen.lock().unwrap().response_delay = Some(Duration::from_millis(10_500));
    f.answer("allow");
    forms::tick(&f.app.feature_context());
    wait("review request received", || {
        !f.server.seen.lock().unwrap().bodies.is_empty()
    });
    // A settings write succeeds while the model is stalled: no writer lock is held.
    f.config(true, "Autorise seulement la lecture.");
    let deadline = std::time::Instant::now() + Duration::from_secs(14);
    while f.review(&id).pending {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.status(&id), RequestStatus::Pending);
    assert_eq!(f.review(&id).answer.decision, reviewer::ReviewDecision::Ask);
    assert!(store::pending_wakes(&f.db()).unwrap().is_empty());
}
#[test]
fn interrupted_attempts_do_not_retry_or_grant_after_restart() {
    let mut f = Fixture::new();
    let id = f.pending("interrupted", json!({"path":"a"}));
    let config: AutoReviewConfig = serde_json::from_value(
        f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["config"].clone(),
    )
    .unwrap();
    let record = reviewer::ReviewRecord {
        version: reviewer::version(&config).unwrap(),
        rules: config,
        model: "gpt-test".into(),
        started_at: now() as i64 - 20000,
        pending: true,
        answer: reviewer::ReviewAnswer::ask("En cours"),
    };
    with_tx(&f.db(), |c| reviewer::claim(c, &id, &record)).unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    forms::tick(&f.app.feature_context());
    assert_eq!(f.status(&id), RequestStatus::Pending);
    assert!(f.server.seen.lock().unwrap().bodies.is_empty());
}
