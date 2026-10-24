//! Wrapper/engine integration, never another worker's target/ or database.
mod support;
use grokbot_delegation::graph::{self, Delegation, DelegationStatus};
use grokbot_engine::{
    contracts::*,
    model::{self, Run, RunStatus},
    store,
};
use grokbot_forms::vault::Vault;
use grokbot_wrapper::{
    features::{self, delegation, Context},
    App, Provider,
};
use rusqlite::Connection;
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
    server: FakeOpenAi,
    path: PathBuf,
    now_ms: u64,
}
impl Fixture {
    fn new(live: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "grokbot-delegation-wrapper-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ))
            .join("app.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        if live {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"access-2","refresh_token":"refresh-2","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        }
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        support::ask_before_acting(&mut app);
        if live {
            wait("catalog", || provider.status()["models_ready"] == true);
        }
        let mut f = Self {
            app,
            provider,
            server,
            path,
            now_ms: 100,
        };
        for id in ["child", "grandchild", "other"] {
            f.post("/api/bots",json!({"id":id,"name":id,"color":"citron","description":"Worker","instructions":""}));
        }
        f
    }
    fn db(&self) -> Connection {
        let db = Connection::open(&self.path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
        db
    }
    fn post(&mut self, path: &str, value: Value) -> Value {
        self.app
            .api("POST", path, value.to_string().as_bytes())
            .unwrap()
    }
    fn conversation(&mut self) -> String {
        self.post("/api/conversations", json!({"title":"Delegation test"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn send(&mut self, c: &str, m: &str, text: &str) {
        self.post(
            "/api/send",
            json!({"conversation_id":c,"message_id":m,"body":text}),
        );
    }
    fn events(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn run(&self, c: &str, m: &str) -> Run {
        store::latest_run_for_trigger(
            &self.db(),
            &Trigger::UserMessage {
                message_id: format!("{c}/{m}"),
                attempt: 0,
            },
        )
        .unwrap()
        .unwrap()
    }
    fn root(&mut self, id: &str) -> Run {
        let c = self.conversation();
        self.run_in(id, "grokbot", &c, 1)
    }
    fn run_in(&self, id: &str, bot: &str, c: &str, started_at: i64) -> Run {
        let db = self.db();
        let tx = features::write_transaction(&db).unwrap();
        let r = model::start(
            &tx,
            &Run {
                run_id: id.into(),
                bot_id: bot.into(),
                conversation_id: c.into(),
                trigger: Trigger::UserMessage {
                    message_id: format!("manual/{id}"),
                    attempt: 0,
                },
                kind: RunKind::Chat,
                status: RunStatus::Queued,
                task_status: None,
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at,
            },
        )
        .unwrap();
        let r = model::set_running_at(&tx, &r.run_id, 1).unwrap();
        tx.commit().unwrap();
        r
    }
    fn context<'a>(&'a self, db: &'a Connection, run: &'a Run) -> Context<'a> {
        Context {
            connection: db,
            provider: &self.provider,
            db_path: &self.path,
            now_ms: self.now_ms,
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
        }
    }
    fn tool(&self, run: &Run, id: &str, name: &str, args: Value) -> ToolOutcome {
        let db = self.db();
        let tx = features::write_transaction(&db).unwrap();
        let outcome = delegation::call_tool(
            &self.context(&tx, run),
            &ToolCall {
                call_id: id.into(),
                name: name.into(),
                arguments: args,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        outcome
    }
    fn children(&self, run: &Run) -> Vec<Delegation> {
        graph::descendants(&self.db(), &run.run_id).unwrap()
    }
    fn tick(&mut self) {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.app.tick(ms);
    }
    fn wait_parent(&mut self, c: &str, m: &str, status: RunStatus) {
        let mut delivered = false;
        wait("parent state", || {
            // A background launch pauses on its own card. Existing engine tests
            // that wait for the child (or for the parent to finish) allow it once.
            if self.run(c, m).status == RunStatus::AwaitingApproval {
                if self.allow_pending_background() || !delivered {
                    self.tick_suspended();
                    delivered = true;
                }
            } else {
                delivered = false;
            }
            self.run(c, m).status == status && self.provider.status()["titling"] == false
        });
    }
    fn allow_pending_background(&mut self) -> bool {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        let mut pending = Vec::new();
        for conversation in state["conversations"].as_array().into_iter().flatten() {
            for message in conversation["messages"].as_array().into_iter().flatten() {
                if message["kind"] == "approval"
                    && message["payload"]["status"]["status"] == "pending"
                    && message["payload"]["policy"]["scope"] == "action"
                {
                    pending.push((
                        message["payload"]["entry_id"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                        message["payload"]["route"]["target_conversation_id"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned(),
                    ));
                }
            }
        }
        if pending.is_empty() {
            return false;
        }
        for (entry, conversation) in pending {
            self.post(
                "/api/approvals/resolve",
                json!({
                    "entry_id": entry,
                    "submission_id": format!("bg-{entry}"),
                    "conversation_id": conversation,
                    "decision": "allow_once"
                }),
            );
        }
        true
    }
    fn tick_suspended(&mut self) {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        self.app.tick_suspended(ms);
    }
    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }
    fn begin(&mut self, c: &str, m: &str, args: Value) -> (Run, Delegation) {
        self.events(vec![call("delegate-call", "delegate", args), completed()]);
        self.send(c, m, "Perform this task, without copying hidden history.");
        self.wait_parent(c, m, RunStatus::AwaitingChildren);
        let run = self.run(c, m);
        let edge = self.children(&run).remove(0);
        (run, edge)
    }
    fn child_done(&mut self, edge: &Delegation) {
        self.child_result(edge, "Verified child summary");
    }
    fn child_result(&mut self, edge: &Delegation, summary: &str) {
        self.events(vec![
            json!({"type":"response.output_text.delta","delta":summary}),
            call("finish-child", "complete_task", json!({"summary":summary})),
            completed(),
        ]);
        self.tick();
        wait("child done", || {
            matches!(
                graph::get(&self.db(), &edge.id).unwrap().unwrap().status,
                DelegationStatus::Done { .. }
            )
        });
        wait("child owner released", || {
            self.provider.status()["titling"] == false
        });
    }
    /// The parent resume is a spawned model call. Settle it before queueing
    /// another scripted response, and drop an acknowledgement it no longer needs.
    fn settle(&mut self, c: &str, m: &str) {
        let run_id = self.run(c, m).run_id;
        let terminal = |status: RunStatus| {
            matches!(
                status,
                RunStatus::Finished | RunStatus::Failed | RunStatus::Cancelled
            )
        };
        if terminal(store::get(&self.db(), &run_id).unwrap().unwrap().status)
            && self.provider.status()["titling"] == false
        {
            return;
        }
        let before = self.server.seen.lock().unwrap().responses.len();
        self.events(vec![
            json!({"type":"response.output_text.delta","delta":"Parent acknowledged"}),
            completed(),
        ]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            self.tick();
            let status = store::get(&self.db(), &run_id).unwrap().unwrap().status;
            if terminal(status) && self.provider.status()["titling"] == false {
                let mut seen = self.server.seen.lock().unwrap();
                if seen.responses.len() > before {
                    seen.responses.pop_back();
                }
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "parent {status:?} not settled"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    fn resume(&mut self, c: &str, m: &str) {
        self.events(vec![
            json!({"type":"response.output_text.delta","delta":"Parent acknowledged"}),
            completed(),
        ]);
        self.tick();
        self.wait_parent(c, m, RunStatus::Finished);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn completed() -> Value {
    json!({"type":"response.completed"})
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}})
}
fn args(bot: &str) -> Value {
    json!({"child_bot":bot,"objective":"Verify selected material","selected_context":{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}})
}
fn form_request(state: &Value) -> &Value {
    state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["kind"]["kind"] == "form")
        .expect("formulaire")
}

fn suspended(outcome: ToolOutcome) -> String {
    match outcome {
        ToolOutcome::Suspend {
            cause: SuspendCause::Children { ids },
        } => ids[0].clone(),
        other => panic!("expected child suspension, got {other:?}"),
    }
}

fn context_request(run: &Run) -> ContextRequest {
    ContextRequest {
        run_id: run.run_id.clone(),
        trigger: run.trigger.clone(),
        bot_id: run.bot_id.clone(),
        conversation_id: run.conversation_id.clone(),
        current_message: HistoryItem {
            entry_id: "current".into(),
            role: Role::User,
            text: "Verify selected material".into(),
            images: vec![],
            tool_items: vec![],
        },
    }
}

#[test]
fn failed_last_delegate_waits_for_its_admitted_sibling() {
    let mut invalid = Vec::new();
    invalid.push(args("ghost-bot"));
    let mut empty = args("child");
    empty["objective"] = json!("");
    invalid.push(empty);
    let mut context = args("child");
    context["selected_context"]["entry_ids"] = json!(["unreadable-entry"]);
    invalid.push(context);
    let mut schema = args("child");
    schema["unknown"] = json!(true);
    invalid.push(schema);
    for bad in invalid {
        let mut f = Fixture::new(true);
        let c = f.conversation();
        f.events(vec![
            call("d1", "delegate", args("child")),
            call("d2", "delegate", bad),
            completed(),
        ]);
        f.send(&c, "failed-sibling", "Delegate independent work");
        f.wait_parent(&c, "failed-sibling", RunStatus::AwaitingChildren);
        let parent = f.run(&c, "failed-sibling");
        let children = f.children(&parent);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].status, DelegationStatus::Running);
        let failures:i64=f.db().query_row("SELECT COUNT(*) FROM wrapper_delegation_calls WHERE parent_run_id=?1 AND refusal IS NOT NULL",[&parent.run_id],|r|r.get(0)).unwrap();
        assert_eq!(failures, 1);
        f.child_done(&children[0]);
        f.resume(&c, "failed-sibling");
        let body = f
            .server
            .seen
            .lock()
            .unwrap()
            .bodies
            .last()
            .unwrap()
            .to_string();
        assert!(body.contains("Verified child summary"));
        assert!(body.contains("Le contexte de délégation est invalide ou indisponible ; aucune sous-tâche n’a été créée."));
        assert!(matches!(
            graph::get(&f.db(), &children[0].id)
                .unwrap()
                .unwrap()
                .status,
            DelegationStatus::Done { .. }
        ));
    }
}

#[test]
fn non_object_last_delegate_cannot_own_the_batch_wait() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("d1", "delegate", args("child")),
        call("d2", "delegate", json!([1])),
        completed(),
    ]);
    f.send(&c, "invalid-args", "Delegate work");
    f.wait_parent(&c, "invalid-args", RunStatus::AwaitingChildren);
    let parent = f.run(&c, "invalid-args");
    let edge = f.children(&parent).remove(0);
    assert_eq!(edge.status, DelegationStatus::Running);
    let events = store::events(&f.db(), &parent.run_id, 0).unwrap();
    assert!(events
        .iter()
        .any(|e| e.kind == RunEventKind::Suspended && e.payload["call_id"] == "d1"));
    f.child_done(&edge);
    f.resume(&c, "invalid-args");
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(body.contains("Verified child summary") && body.contains("invalid arguments"));
    assert!(matches!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Done { .. }
    ));
}

#[test]
fn tool_budget_cannot_skip_the_final_batch_wait() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let db = f.db();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let tx = features::write_transaction(&db).unwrap();
    let root = model::start(
        &tx,
        &Run {
            run_id: "budget-batch-root".into(),
            bot_id: "grokbot".into(),
            conversation_id: c,
            trigger: Trigger::UserMessage {
                message_id: "manual/budget".into(),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Queued,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets {
                tool_calls: 1,
                ..RunBudgets::default()
            },
            started_at: now,
        },
    )
    .unwrap();
    let root = model::set_running_at(&tx, &root.run_id, now).unwrap();
    tx.execute(
        "INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,1,'gpt-test')",
        rusqlite::params![
            root.run_id,
            serde_json::to_string(&context_request(&root).current_message).unwrap()
        ],
    )
    .unwrap();
    for (id, bot) in [("d1", "child"), ("d2", "other")] {
        store::append_event(
            &tx,
            &root.run_id,
            RunEventKind::ToolCall,
            &json!({"call_id":id,"name":"delegate","arguments":args(bot),"response":1}),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    f.tick();
    let mut budget_delivered = false;
    wait("budget batch waits", || {
        if store::required(&f.db(), &root.run_id).unwrap().status == RunStatus::AwaitingApproval
            && (f.allow_pending_background() || !budget_delivered)
        {
            f.tick_suspended();
            budget_delivered = true;
        }
        if store::required(&f.db(), &root.run_id).unwrap().status != RunStatus::AwaitingApproval {
            budget_delivered = false;
        }
        store::required(&f.db(), &root.run_id).unwrap().status == RunStatus::AwaitingChildren
    });
    let edge = f.children(&root).remove(0);
    assert_eq!(edge.status, DelegationStatus::Running);
    f.child_done(&edge);
    f.tick();
    wait("budget ends only after child", || {
        store::required(&f.db(), &root.run_id).unwrap().status == RunStatus::Finished
    });
    assert_eq!(
        store::required(&f.db(), &root.run_id).unwrap().turn_end,
        Some(TurnEnd::BudgetExhausted)
    );
    assert!(matches!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Done { .. }
    ));
    assert!(!store::pending_wakes(&f.db())
        .unwrap()
        .iter()
        .any(|w| w.run_id == root.run_id));
}

#[test]
fn completed_before_batch_suspension_wake_is_delivered_with_the_group() {
    let mut f = Fixture::new(false);
    let root = f.root("early-completion-root");
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    for (id, bot) in [("d1", "child"), ("d2", "other")] {
        store::append_event(
            &tx,
            &root.run_id,
            RunEventKind::ToolCall,
            &json!({"call_id":id,"name":"delegate","arguments":args(bot),"response":1}),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let first = f.tool(&root, "d1", "delegate", args("child"));
    assert!(matches!(first, ToolOutcome::Done { .. }));
    let edge = f.children(&root).remove(0);
    let tx = features::write_transaction(&db).unwrap();
    store::append_event(
        &tx,
        &root.run_id,
        RunEventKind::ToolResult,
        &json!({"call_id":"d1","outcome":first}),
    )
    .unwrap();
    let mut child = model::set_running_at(&tx, edge.child_run_id.as_ref().unwrap(), 100).unwrap();
    child.task_status = Some(TaskStatus::Done {
        summary: "early summary".into(),
    });
    store::save(&tx, &child).unwrap();
    let event = store::append_event(
        &tx,
        &child.run_id,
        RunEventKind::TaskChanged,
        &json!({"task_status":child.task_status}),
    )
    .unwrap();
    delegation::on_run_event(&f.context(&tx, &child), &event);
    tx.commit().unwrap();
    let second = f.tool(&root, "d2", "delegate", args("other"));
    let cause = match &second {
        ToolOutcome::Suspend {
            cause: SuspendCause::Children { ids },
        } => {
            assert_eq!(ids.len(), 2);
            SuspendCause::Children { ids: ids.clone() }
        }
        other => panic!("{other:?}"),
    };
    let later = f
        .children(&root)
        .into_iter()
        .find(|d| d.status == DelegationStatus::Running)
        .unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut parent = store::required(&tx, &root.run_id).unwrap();
    parent.status = RunStatus::AwaitingChildren;
    store::save(&tx, &parent).unwrap();
    store::append_event(
        &tx,
        &root.run_id,
        RunEventKind::ToolResult,
        &json!({"call_id":"d2","outcome":second}),
    )
    .unwrap();
    store::append_event(
        &tx,
        &root.run_id,
        RunEventKind::Suspended,
        &json!({"call_id":"d2","cause":cause}),
    )
    .unwrap();
    let mut child = model::set_running_at(&tx, later.child_run_id.as_ref().unwrap(), 100).unwrap();
    child.task_status = Some(TaskStatus::Done {
        summary: "later summary".into(),
    });
    store::save(&tx, &child).unwrap();
    let event = store::append_event(
        &tx,
        &child.run_id,
        RunEventKind::TaskChanged,
        &json!({"task_status":child.task_status}),
    )
    .unwrap();
    delegation::on_run_event(&f.context(&tx, &child), &event);
    model::resume(&tx, &root.run_id).unwrap();
    tx.commit().unwrap();
    assert!(!store::pending_wakes(&db)
        .unwrap()
        .iter()
        .any(|w| w.run_id == root.run_id));
    let events = store::events(&db, &root.run_id, 0).unwrap();
    let resumed = events
        .iter()
        .find(|e| e.kind == RunEventKind::Resumed)
        .unwrap();
    assert_eq!(resumed.payload["wake_ids"].as_array().unwrap().len(), 2);
}

#[test]
fn routines_and_all_delegated_result_tails_survive_assembly() {
    let mut f = Fixture::new(false);
    let root = f.root("routine-budget-root");
    f.post("/api/routines",json!({"bot_id":"grokbot","name":"Routine index","prompt":"Daily check","schedule":"@daily","time_zone":"UTC","enabled":false}));
    for n in 0..3 {
        let id = suspended(f.tool(&root, &format!("result-{n}"), "delegate", args("child")));
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        graph::set_status(
            &tx,
            &id,
            &DelegationStatus::Done {
                result: format!("CHILD_{n}_BEGIN{}CHILD_{n}_END", "x".repeat(7_000)),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let db = f.db();
    let request = context_request(&root);
    let blocks = features::context(&f.context(&db, &root), &request)
        .into_iter()
        .filter(|b| b.source != ContextSource::CurrentMessage)
        .collect::<Vec<_>>();
    let assembled = grokbot_engine::context::assemble_input(
        &request,
        &blocks,
        &[],
        None,
        None,
        &ContextBudget::default(),
    )
    .unwrap();
    let body = assembled
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(body.contains("routines_total"));
    for n in 0..3 {
        assert!(body.contains(&format!("CHILD_{n}_END")));
    }
}

#[test]
fn child_routine_does_not_reduce_the_selected_context_window() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.post("/api/routines",json!({"bot_id":"child","name":"Child routine","prompt":"Daily check","schedule":"@daily","time_zone":"UTC","enabled":false}));
    let mut a = args("child");
    a["selected_context"]["texts"] =
        json!([format!("{}SELECTED_CONTEXT_TAIL", "x".repeat(20_000))]);
    let (_, edge) = f.begin(&c, "selected-window", a);
    f.child_done(&edge);
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(body.contains("SELECTED_CONTEXT_TAIL") && body.contains("routines_total"));
    f.resume(&c, "selected-window");
}

#[test]
fn active_task_send_is_rejected_without_a_second_run() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "busy-thread", args("child"));
    let task = edge.conversation_id.as_ref().unwrap();
    let error = f
        .app
        .api(
            "POST",
            "/api/send",
            json!({"conversation_id":task,"message_id":"steer-child","body":"Change direction"})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(
        error.message,
        "Utilisez le fil de tâche pour envoyer un suivi à cette sous-tâche."
    );
    assert_eq!(
        store::runs_for_conversation(&f.db(), task).unwrap().len(),
        1
    );
    f.child_done(&edge);
    f.resume(&c, "busy-thread");
}

#[test]
fn deleting_waiting_task_thread_cancels_child_and_delivers_cancelled_to_parent() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "delete-child", args("child"));
    f.events(vec![call("wait","request_form",json!({"fields":[{"name":"answer","label":"Answer","field_type":{"kind":"text"},"required":true}]})),completed()]);
    f.tick();
    wait("child input wait", || {
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status
            == RunStatus::AwaitingHumanInput
    });
    wait("child owner released", || {
        f.provider.status()["titling"] == false
    });
    f.post(
        "/api/conversations/delete",
        json!({"id":edge.conversation_id}),
    );
    f.tick();
    assert_eq!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
    assert_eq!(
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status,
        RunStatus::Cancelled
    );
    assert!(store::pending_wakes(&f.db())
        .unwrap()
        .iter()
        .any(|w| w.run_id == parent.run_id && w.reference == edge.id));
    f.resume(&c, "delete-child");
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string()
        .contains("cancelled"));
}

#[test]
fn checked_context_is_reused_once_and_cache_is_scoped_to_assembly() {
    let mut f = Fixture::new(false);
    let root = f.root("context-cache-root");
    let mut a = args("child");
    a["selected_context"]["texts"] = json!(["SCOPED_SELECTED_DATA"]);
    let id = suspended(f.tool(&root, "cache", "delegate", a));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    let db = f.db();
    let child = store::required(&db, edge.child_run_id.as_ref().unwrap()).unwrap();
    let request = context_request(&child);
    let ctx = f.context(&db, &child);
    let blocks = delegation::with_context(&ctx, &request, || {
        // If the hook recomputed context_checked, this malformed vault would
        // erase/fail it. It must reuse the single already-checked snapshot.
        std::fs::write(f.path.with_extension("secrets.json"), b"not-json").unwrap();
        delegation::context(&ctx, &request)
    })
    .unwrap();
    assert!(blocks
        .iter()
        .any(|b| b.text.contains("SCOPED_SELECTED_DATA")));
    assert!(delegation::context(&ctx, &request).is_empty());
    assert!(delegation::with_context(&ctx, &request, || ()).is_err());
    std::fs::remove_file(f.path.with_extension("secrets.json")).unwrap();
    let indexed:i64=db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='wrapper_delegation_calls_parent'",[],|r|r.get(0)).unwrap();
    assert_eq!(indexed, 1);
}

#[test]
fn denied_last_delegate_still_waits_for_approved_sibling() {
    rejected_last_delegate_still_waits_for_approved_sibling(false);
}

#[test]
fn expired_last_delegate_still_waits_for_approved_sibling() {
    rejected_last_delegate_still_waits_for_approved_sibling(true);
}

fn rejected_last_delegate_still_waits_for_approved_sibling(expired: bool) {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("approved", "delegate", args("child")),
        call("denied", "delegate", args("other")),
        completed(),
    ]);
    f.send(&c, "mixed", "Run two independent tasks");
    wait("first approval", || {
        f.run(&c, "mixed").status == RunStatus::AwaitingApproval
            && f.provider.status()["titling"] == false
    });
    assert!(f.allow_pending_background());
    f.tick_suspended();
    wait("second approval", || {
        f.run(&c, "mixed").status == RunStatus::AwaitingApproval
            && f.children(&f.run(&c, "mixed")).len() == 1
            && f.provider.status()["titling"] == false
    });
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let card = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|conversation| conversation["messages"].as_array().unwrap())
        .find(|message| {
            message["kind"] == "approval" && message["payload"]["status"]["status"] == "pending"
        })
        .expect("pending sibling approval");
    let request = &card["payload"];
    if expired {
        let db = f.db();
        let parent = f.run(&c, "mixed");
        let ctx = Context {
            now_ms: request["kind"]["expires_at"].as_u64().unwrap() + 1,
            ..f.context(&db, &parent)
        };
        features::forms::tick(&ctx);
    } else {
        f.post(
            "/api/approvals/resolve",
            json!({
                "entry_id": request["entry_id"],
                "submission_id": "deny-sibling",
                "conversation_id": c,
                "decision": "deny"
            }),
        );
    }
    f.events(vec![completed()]);
    f.tick_suspended();
    wait("approval settled", || {
        f.provider.status()["titling"] == false
    });
    let parent = f.run(&c, "mixed");
    assert_eq!(parent.status, RunStatus::AwaitingChildren);
    let children = f.children(&parent);
    assert_eq!(children.len(), 1, "the denied task must never launch");
    assert!(matches!(
        children[0].status,
        DelegationStatus::Queued | DelegationStatus::Running
    ));
    // The unused parent response must still be queued while children run.
    f.server.seen.lock().unwrap().responses.pop_front().unwrap();
    f.child_done(&children[0]);
    f.resume(&c, "mixed");
    let seen = f.server.seen.lock().unwrap();
    let resumed = seen.bodies.last().unwrap().to_string();
    assert!(resumed.contains("Verified child summary"));
    assert!(resumed.contains(if expired { "écoulé" } else { "refusée" }));
}

#[test]
fn parallel_delegates_in_one_response_run_concurrently() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("d1", "delegate", args("child")),
        call("d2", "delegate", args("other")),
        call("refused", "delegate", args("grokbot")),
        completed(),
    ]);
    f.send(&c, "parallel", "Run independent tasks in parallel");
    f.wait_parent(&c, "parallel", RunStatus::AwaitingChildren);
    let parent = f.run(&c, "parallel");
    let children = f.children(&parent);
    assert_eq!(children.len(), 2);
    assert!(children
        .iter()
        .all(|d| d.status == DelegationStatus::Running));
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["delegation"]["active_by_conversation"][&c], 2);
    let events = store::events(&f.db(), &parent.run_id, 0).unwrap();
    let children_wait = events
        .iter()
        .find(|e| e.kind == RunEventKind::Suspended && e.payload["cause"]["kind"] == "children")
        .expect("attente des sous-tâches");
    assert!(events
        .iter()
        .any(|e| { e.kind == RunEventKind::Suspended && e.payload["approval"] == true }));
    assert_eq!(
        events
            .iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "d1")
            .unwrap()
            .payload["outcome"]["status"],
        "done"
    );
    assert_eq!(
        children_wait.payload["cause"]["ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.events(vec![
        call(
            "first-done",
            "complete_task",
            json!({"summary":"PARALLEL_FIRST_RESULT"}),
        ),
        completed(),
    ]);
    f.events(vec![call("need-input","request_form",json!({"fields":[{"name":"answer","label":"Continue","field_type":{"kind":"text"},"required":true}]})),completed()]);
    f.tick();
    wait("one child complete, sibling still working", || {
        let edges = f.children(&parent);
        edges
            .iter()
            .filter(|d| matches!(d.status, DelegationStatus::Done { .. }))
            .count()
            == 1
            && edges.iter().any(|d| {
                store::required(&f.db(), d.child_run_id.as_ref().unwrap())
                    .unwrap()
                    .status
                    == RunStatus::AwaitingHumanInput
            })
    });
    wait("child owners released", || {
        f.provider.status()["titling"] == false
    });
    assert!(!model::ready_to_resume(&f.db(), &parent.run_id).unwrap());
    assert_eq!(f.run(&c, "parallel").status, RunStatus::AwaitingChildren);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let entry = form_request(&state)["entry_id"].clone();
    f.events(vec![
        call(
            "second-done",
            "complete_task",
            json!({"summary":"PARALLEL_SECOND_RESULT"}),
        ),
        completed(),
    ]);
    f.post("/api/forms/submit",json!({"entry_id":entry,"submission_id":"continue","conversation_id":c,"values":{"answer":"yes"}}));
    f.tick();
    wait("both child results", || {
        f.children(&parent)
            .iter()
            .all(|d| matches!(d.status, DelegationStatus::Done { .. }))
    });
    wait("child owners released", || {
        f.provider.status()["titling"] == false
    });
    f.resume(&c, "parallel");
    let events = store::events(&f.db(), &parent.run_id, 0).unwrap();
    let approvals = events
        .iter()
        .filter(|e| e.kind == RunEventKind::Suspended && e.payload["approval"] == true)
        .count();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == RunEventKind::Resumed)
            .count(),
        approvals + 1,
        "une reprise pour chaque carte, puis une seule pour le lot de sous-tâches"
    );
    let seen = f.server.seen.lock().unwrap();
    let body = seen.bodies.last().unwrap().to_string();
    assert!(body.contains("PARALLEL_FIRST_RESULT") && body.contains("PARALLEL_SECOND_RESULT"));
    assert!(
        body.contains("cycle"),
        "a refused last delegate must not hide its reason"
    );
}

#[test]
fn plain_child_answer_is_preserved_and_completion_contract_is_host_owned() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    // A one-character private instruction must not cap the host contract.
    f.post("/api/bots/update", json!({"id":"child","instructions":"X"}));
    let (_, edge) = f.begin(&c, "plain", args("child"));
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"CHILD_PLAIN_ANSWER_42"}),
        completed(),
    ]);
    f.tick();
    wait("plain child ended", || {
        matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Failed { .. }
        )
    });
    wait("child owner released", || {
        f.provider.status()["titling"] == false
    });
    let seen = f.server.seen.lock().unwrap();
    let instructions = seen.bodies.last().unwrap()["instructions"]
        .as_str()
        .unwrap();
    assert!(instructions.contains("You are executing a delegated Task from bot grokbot"));
    assert!(instructions.contains("call complete_task(summary) with the complete result"));
    drop(seen);
    f.resume(&c, "plain");
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string()
        .contains("CHILD_PLAIN_ANSWER_42"));
}

#[test]
fn newest_long_results_and_all_wakees_fit_the_rendered_context_budget() {
    let mut f = Fixture::new(false);
    let root = f.root("long-results-root");
    let mut ids = Vec::new();
    for n in 0..8 {
        let id = suspended(f.tool(&root, &format!("long-{n}"), "delegate", args("child")));
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        graph::set_status(
            &tx,
            &id,
            &DelegationStatus::Done {
                result: format!("SUMMARY_{n}_BEGIN{}", "\n\"\\".repeat(10_000)),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        ids.push(id);
    }
    // Model resumed for the older group too: none of its required wakees may be
    // omitted merely because five newer historical results exist.
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let mut run = store::required(&tx, &root.run_id).unwrap();
    run.status = RunStatus::AwaitingChildren;
    store::save(&tx, &run).unwrap();
    store::append_event(
        &tx,
        &root.run_id,
        RunEventKind::Suspended,
        &json!({"cause":{"kind":"children","ids":ids[..3]}}),
    )
    .unwrap();
    for id in &ids[..3] {
        store::enqueue_wake(&tx, &root.run_id, WakeCause::Child, id).unwrap();
    }
    model::resume(&tx, &root.run_id).unwrap();
    tx.commit().unwrap();
    let request = ContextRequest {
        run_id: root.run_id.clone(),
        trigger: root.trigger.clone(),
        bot_id: root.bot_id.clone(),
        conversation_id: root.conversation_id.clone(),
        current_message: HistoryItem {
            entry_id: "resume".into(),
            role: Role::User,
            text: "resume".into(),
            images: vec![],
            tool_items: vec![],
        },
    };
    let blocks = features::context(&f.context(&db, &root), &request)
        .into_iter()
        .filter(|b| b.source != ContextSource::CurrentMessage)
        .collect::<Vec<_>>();
    let assembled = grokbot_engine::context::assemble_input(
        &request,
        &blocks,
        &[],
        None,
        None,
        &ContextBudget::default(),
    )
    .unwrap();
    let body = assembled
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for n in 0..8 {
        assert!(
            body.contains(&format!("SUMMARY_{n}_BEGIN")),
            "missing result {n}"
        );
    }
    assert!(body.contains("tronqué"));
    let result = blocks
        .iter()
        .find(|b| b.source == ContextSource::DelegatedContext)
        .unwrap();
    assert!(serde_json::to_string(&result.text).unwrap().chars().count() + 512 <= 32_000);
    assert!(result.text.starts_with("Child results (untrusted data"));
}

#[test]
fn task_namespace_is_rejected_for_bot_and_message_ids() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    let err=f.app.api("POST","/api/bots",json!({"id":"task:a:b","name":"bad","color":"citron","description":"","instructions":""}).to_string().as_bytes()).unwrap_err();
    assert_eq!(err.status, 400);
    let err = f
        .app
        .api(
            "POST",
            "/api/send",
            json!({"conversation_id":c,"message_id":"task:a:b","body":"bad"})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(err.status, 400);
    let root = f.root("conversation-scope-root");
    let id = suspended(f.tool(&root, "task", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    f.post(
        "/api/conversations/update",
        json!({"id":edge.conversation_id,"title":"Allowed task thread"}),
    );
    f.post(
        "/api/conversations/delete",
        json!({"id":edge.conversation_id}),
    );
}

#[test]
fn revoking_one_selected_fact_keeps_other_context_and_reports_revocation() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let fact = match f
        .app
        .invoke_tool(
            "grokbot",
            "remember",
            json!({"text":"REVOKED_DELEGATED_FACT"}),
        )
        .unwrap()
    {
        ToolOutcome::Done { output } => output["fact"]["fact_id"].clone(),
        _ => panic!(),
    };
    let mut a = args("child");
    a["selected_context"]["texts"] = json!(["STILL_VALID_INLINE_CONTEXT"]);
    a["selected_context"]["fact_ids"] = json!([fact]);
    let (_, edge) = f.begin(&c, "revoke", a);
    f.events(vec![call("pause","request_form",json!({"fields":[{"name":"answer","label":"Continue","field_type":{"kind":"text"},"required":true}]})),completed()]);
    f.tick();
    wait("child paused", || {
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status
            == RunStatus::AwaitingHumanInput
    });
    wait("child released", || f.provider.status()["titling"] == false);
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":fact}),
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let entry = form_request(&state)["entry_id"].clone();
    f.events(vec![
        call(
            "done",
            "complete_task",
            json!({"summary":"Used only remaining context"}),
        ),
        completed(),
    ]);
    f.post("/api/forms/submit",json!({"entry_id":entry,"submission_id":"after-forget","conversation_id":c,"values":{"answer":"continue"}}));
    f.tick();
    wait("child finished", || {
        matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Done { .. }
        )
    });
    wait("child released", || f.provider.status()["titling"] == false);
    let seen = f.server.seen.lock().unwrap();
    let body = seen.bodies.last().unwrap().to_string();
    assert!(body.contains("STILL_VALID_INLINE_CONTEXT"));
    assert!(body.contains("revoked/unavailable"));
    assert!(body.contains(fact.as_str().unwrap()));
    assert!(!body.contains("REVOKED_DELEGATED_FACT"));
    drop(seen);
    f.resume(&c, "revoke");
}

#[test]
fn unreadable_vault_prevents_child_model_calls_instead_of_dropping_context() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "bad-vault", args("child"));
    let before = f.server.seen.lock().unwrap().bodies.len();
    std::fs::write(f.path.with_extension("secrets.json"), b"not-json").unwrap();
    f.tick();
    wait("host rejects unreadable vault", || {
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status
            == RunStatus::Failed
    });
    wait("child released", || f.provider.status()["titling"] == false);
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
    std::fs::remove_file(f.path.with_extension("secrets.json")).unwrap();
    f.tick();
    f.resume(&c, "bad-vault");
}

#[test]
fn deleting_parent_conversation_prevents_any_first_child_model_call() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "delete-parent", args("child"));
    let before = f.server.seen.lock().unwrap().bodies.len();
    f.post("/api/conversations/delete", json!({"id":c}));
    f.tick();
    assert_eq!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
    assert_eq!(
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status,
        RunStatus::Cancelled
    );
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
}

#[test]
fn cascade_settles_every_unexecuted_child_call() {
    let mut f = Fixture::new(false);
    let root = f.root("settle-root");
    let id = suspended(f.tool(&root, "child", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let child = model::set_running_at(&tx, edge.child_run_id.as_ref().unwrap(), 100).unwrap();
    for call in ["unexecuted-1", "unexecuted-2"] {
        store::append_event(&tx,&child.run_id,RunEventKind::ToolCall,&json!({"call_id":call,"name":"remember","arguments":{"text":"not executed"},"response":0})).unwrap();
    }
    tx.commit().unwrap();
    f.post(
        "/api/stop",
        json!({"conversation_id":root.conversation_id,"message_id":root.run_id}),
    );
    let events = store::events(&db, &child.run_id, 0).unwrap();
    for id in ["unexecuted-1", "unexecuted-2"] {
        let result = events
            .iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == id)
            .unwrap();
        assert_eq!(result.payload["outcome"]["status"], "failed");
        assert_eq!(result.payload["outcome"]["reason"], "not executed: stopped");
    }
}

#[test]
fn state_history_is_bounded_and_idle_tick_takes_no_writer_lock() {
    let mut f = Fixture::new(false);
    let root = f.root("bounded-root");
    let active = suspended(f.tool(&root, "old-active", "delegate", args("child")));
    for n in 0..60 {
        let id = suspended(f.tool(&root, &format!("history-{n}"), "delegate", args("child")));
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        graph::set_status(
            &tx,
            &id,
            &DelegationStatus::Done {
                result: "historical".into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let db = f.db();
    let value = delegation::state(&f.context(&db, &root));
    assert_eq!(value["delegations"].as_array().unwrap().len(), 51);
    assert!(value["delegations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["id"] == active));
    assert_eq!(value["active_by_conversation"][&root.conversation_id], 1);
    let tx = features::write_transaction(&db).unwrap();
    graph::set_status(
        &tx,
        &active,
        &DelegationStatus::Done {
            result: "done".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let writer = features::write_transaction(&db).unwrap();
    let reader = f.db();
    let started = std::time::Instant::now();
    delegation::tick(&f.context(&reader, &root));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "idle tick waited for the writer lock"
    );
    writer.rollback().unwrap();
}

#[test]
fn child_receives_only_selected_context() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.send(&c, "selected-entry", "SELECTED_ENTRY_TEXT");
    f.wait_parent(&c, "selected-entry", RunStatus::Finished);
    f.send(&c, "hidden-entry", "UNSELECTED_PARENT_HISTORY");
    f.wait_parent(&c, "hidden-entry", RunStatus::Finished);
    let selected = f
        .app
        .invoke_tool("grokbot", "remember", json!({"text":"SELECTED_FACT_TEXT"}))
        .unwrap();
    f.app
        .invoke_tool(
            "grokbot",
            "remember",
            json!({"text":"UNSELECTED_PRIVATE_FACT"}),
        )
        .unwrap();
    let fact = match selected {
        ToolOutcome::Done { output } => output["fact"]["fact_id"].as_str().unwrap().to_owned(),
        _ => panic!(),
    };
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"selected","body":"SELECTED_SKILL_TEXT"}),
    );
    f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"hidden","body":"UNSELECTED_PRIVATE_SKILL"}),
    );
    f.post(
        "/api/bots/update",
        json!({"id":"grokbot","instructions":"UNSELECTED_PARENT_INSTRUCTIONS"}),
    );
    let mut a = args("child");
    a["selected_context"] = json!({"texts":["SELECTED_INLINE_TEXT"],"entry_ids":["selected-entry"],"fact_ids":[fact],"skill_ids":[skill["skill"]["skill_id"]]});
    let (_, edge) = f.begin(&c, "delegate", a);
    f.child_done(&edge);
    let seen = f.server.seen.lock().unwrap();
    let body = seen
        .bodies
        .iter()
        .find(|b| b["input"].to_string().contains("Verify selected material"))
        .expect("child request")
        .to_string();
    for text in [
        "SELECTED_ENTRY_TEXT",
        "SELECTED_INLINE_TEXT",
        "SELECTED_FACT_TEXT",
        "SELECTED_SKILL_TEXT",
    ] {
        assert!(body.contains(text), "missing {text}: {body}");
    }
    for text in [
        "UNSELECTED_PARENT_HISTORY",
        "UNSELECTED_PRIVATE_FACT",
        "UNSELECTED_PRIVATE_SKILL",
        "UNSELECTED_PARENT_INSTRUCTIONS",
        "Perform this task",
    ] {
        assert!(!body.contains(text), "leaked {text}");
    }
    assert!(body.contains("untrusted"));
    drop(seen);
    f.resume(&c, "delegate");
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let child = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"].as_str() == edge.conversation_id.as_deref())
        .unwrap();
    assert_eq!(child["kind"], "task");
    assert_eq!(child["parent_id"], c);
    assert_eq!(
        edge.conversation_id.as_ref().unwrap(),
        &format!("task:{}:{c}", edge.id)
    );
}

#[test]
fn child_done_wakes_parent_with_summary() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "wake", args("child"));
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["delegation"]["active_by_conversation"][&c], 1);
    assert!(state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == c)
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "delegation"));
    f.child_done(&edge);
    let wakes = store::pending_wakes(&f.db()).unwrap();
    let wake = wakes.iter().find(|w| w.run_id == parent.run_id).unwrap();
    assert_eq!(wake.cause, WakeCause::Child);
    assert_eq!(wake.reference, edge.id);
    assert_eq!(wake.status, WakeStatus::Pending);
    let wake_id = wake.wake_id.clone();
    assert_eq!(f.run(&c, "wake").status, RunStatus::AwaitingChildren);
    f.resume(&c, "wake");
    assert_eq!(f.run(&c, "wake").run_id, parent.run_id);
    assert!(
        matches!(store::wake_status(&f.db(),&wake_id).unwrap(),WakeStatus::Delivered{run_id,..} if run_id==parent.run_id)
    );
    let seen = f.server.seen.lock().unwrap();
    assert!(seen
        .bodies
        .last()
        .unwrap()
        .to_string()
        .contains("Verified child summary"));
    drop(seen);
    assert_eq!(
        graph::descendants(&f.db(), &parent.run_id).unwrap().len(),
        1
    );
    let out = f.tool(
        &Run {
            status: RunStatus::Running,
            ..parent.clone()
        },
        "check",
        "check_delegations",
        json!({}),
    );
    // A stale context is not authority: the actual persisted run has finished.
    assert!(matches!(out, ToolOutcome::Failed { .. }));
}

#[test]
fn cycle_is_refused_as_tool_result() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("cycle", "delegate", args("grokbot")),
        completed(),
    ]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Doing it inline"}),
        completed(),
    ]);
    f.send(&c, "cycle", "Work");
    f.wait_parent(&c, "cycle", RunStatus::Finished);
    let parent = f.run(&c, "cycle");
    assert!(f.children(&parent).is_empty());
    let events = store::events(&f.db(), &parent.run_id, 0).unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult)
        .unwrap();
    assert_eq!(result.payload["outcome"]["status"], "done");
    assert_eq!(result.payload["outcome"]["output"]["status"], "refused");
    assert!(result.payload.to_string().contains("cycle"));
}

#[test]
fn admission_queues_then_refuses() {
    let mut f = Fixture::new(false);
    let root = f.root("root");
    for n in 0..19 {
        suspended(f.tool(&root, &format!("call-{n}"), "delegate", args("child")));
    }
    let all = f.children(&root);
    assert_eq!(
        all.iter()
            .filter(|d| d.status == DelegationStatus::Running)
            .count(),
        3
    );
    assert_eq!(
        all.iter()
            .filter(|d| d.status == DelegationStatus::Queued)
            .count(),
        16
    );
    let refused = f.tool(&root, "overflow", "delegate", args("child"));
    assert!(
        matches!(refused,ToolOutcome::Done{output} if output["status"]=="refused" && output["reason"].as_str().unwrap().contains("backlog"))
    );
    assert_eq!(f.children(&root).len(), 19);
    let first = all
        .iter()
        .find(|d| d.status == DelegationStatus::Running)
        .unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let child = model::set_running_at(&tx, first.child_run_id.as_ref().unwrap(), 100).unwrap();
    tx.commit().unwrap();
    // Nested requests cannot occupy a queued slot and deadlock their parent.
    let nested = f.tool(&child, "nested", "delegate", args("grandchild"));
    assert!(
        matches!(nested,ToolOutcome::Done{output} if output["reason"].as_str().unwrap().contains("do this subtask in the current task"))
    );
}

#[test]
fn interrupting_parent_cancels_children_including_queued() {
    let mut f = Fixture::new(false);
    let root = f.root("cancel-root");
    let first = suspended(f.tool(&root, "first", "delegate", args("child")));
    let first = graph::get(&f.db(), &first).unwrap().unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let child = model::set_running_at(&tx, first.child_run_id.as_ref().unwrap(), 100).unwrap();
    tx.commit().unwrap();
    suspended(f.tool(&child, "grandchild", "delegate", args("grandchild")));
    for n in 0..3 {
        suspended(f.tool(&root, &format!("sibling-{n}"), "delegate", args("other")));
    }
    assert!(f
        .children(&root)
        .iter()
        .any(|d| d.status == DelegationStatus::Queued));
    f.post(
        "/api/stop",
        json!({"conversation_id":root.conversation_id,"message_id":root.run_id}),
    );
    for edge in f.children(&root) {
        assert_eq!(edge.status, DelegationStatus::Cancelled);
        if let Some(run) = store::get(&f.db(), edge.child_run_id.as_ref().unwrap()).unwrap() {
            assert_eq!(run.status, RunStatus::Cancelled);
        }
    }
    let total = f.children(&root).len();
    f.tick();
    assert_eq!(f.children(&root).len(), total);
    assert!(f
        .children(&root)
        .iter()
        .all(|d| d.status == DelegationStatus::Cancelled));
}

#[test]
fn failed_child_reports_failure_to_parent() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "failure", args("child"));
    f.events(vec![
        json!({"type":"response.failed","response":{"error":{"message":"worker broke"}}}),
    ]);
    f.tick();
    wait("child failure", || {
        matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Failed { .. }
        )
    });
    wait("owner released", || f.provider.status()["titling"] == false);
    f.resume(&c, "failure");
    let seen = f.server.seen.lock().unwrap();
    let body = seen.bodies.last().unwrap().to_string();
    assert!(body.contains("failed"));
    assert!(body.contains("La sous-tâche s’est arrêtée sans terminer son travail"));
}

#[test]
fn queued_child_starts_when_slot_frees() {
    let mut f = Fixture::new(false);
    let root = f.root("fifo-root");
    let mut ids = Vec::new();
    for n in 0..4 {
        ids.push(suspended(f.tool(
            &root,
            &format!("fifo-{n}"),
            "delegate",
            args("child"),
        )));
    }
    let queued = graph::get(&f.db(), &ids[3]).unwrap().unwrap();
    assert_eq!(queued.status, DelegationStatus::Queued);
    assert!(store::get(&f.db(), queued.child_run_id.as_ref().unwrap())
        .unwrap()
        .is_none());
    let first = graph::get(&f.db(), &ids[0]).unwrap().unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let mut child = model::set_running_at(&tx, first.child_run_id.as_ref().unwrap(), 100).unwrap();
    child.task_status = Some(TaskStatus::Done {
        summary: "first complete".into(),
    });
    store::save(&tx, &child).unwrap();
    let event = store::append_event(
        &tx,
        &child.run_id,
        RunEventKind::TaskChanged,
        &json!({"task_status":child.task_status}),
    )
    .unwrap();
    delegation::on_run_event(&f.context(&tx, &child), &event);
    assert_eq!(
        graph::get(&tx, &queued.id).unwrap().unwrap().status,
        DelegationStatus::Running
    );
    assert_eq!(
        store::required(&tx, queued.child_run_id.as_ref().unwrap())
            .unwrap()
            .status,
        RunStatus::Queued
    );
    tx.commit().unwrap();
    f.tick();
    assert!(f
        .db()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM wrapper_engine_runs WHERE run_id=?1)",
            [queued.child_run_id.unwrap()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
}

#[test]
fn crash_before_child_launch_reconciles_without_duplicate_runs() {
    let mut f = Fixture::new(false);
    let root = f.root("recovery-root");
    let id = suspended(f.tool(&root, "crash", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    let run_id = edge.child_run_id.unwrap();
    assert_eq!(
        store::required(&f.db(), &run_id).unwrap().status,
        RunStatus::Queued
    );
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    f.tick();
    f.tick();
    let count: i64 = f
        .db()
        .query_row(
            "SELECT COUNT(*) FROM engine_runs WHERE trigger_ref=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let host: i64 = f
        .db()
        .query_row(
            "SELECT COUNT(*) FROM wrapper_engine_runs WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host, 1);
}

#[test]
fn selected_resources_are_parent_scoped_and_failed_writes_are_atomic() {
    let mut f = Fixture::new(false);
    let root = f.root("scope-root");
    let elsewhere = f.conversation();
    f.send(&elsewhere, "foreign", "FOREIGN_ENTRY");
    let private = f
        .app
        .invoke_tool("other", "remember", json!({"text":"OTHER_PRIVATE_FACT"}))
        .unwrap();
    let fact = match private {
        ToolOutcome::Done { output } => output["fact"]["fact_id"].clone(),
        _ => panic!(),
    };
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"other","name":"private","body":"OTHER_PRIVATE_SKILL"}),
    );
    for (kind, value) in [
        ("entry_ids", json!(["foreign"])),
        ("fact_ids", json!([fact])),
        ("skill_ids", json!([skill["skill"]["skill_id"]])),
        ("texts", json!(["x".repeat(32_001)])),
    ] {
        let mut a = args("child");
        a["selected_context"][kind] = value;
        assert!(matches!(
            f.tool(&root, kind, "delegate", a),
            ToolOutcome::Failed { .. }
        ));
    }
    assert!(f.children(&root).is_empty());
    let n: i64 = f
        .db()
        .query_row("SELECT COUNT(*) FROM wrapper_delegation_calls", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn nested_cycle_uses_reserved_ancestor_lineage() {
    let mut f = Fixture::new(false);
    let root = f.root("nested-root");
    let id = suspended(f.tool(&root, "child", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let child = model::set_running_at(&tx, edge.child_run_id.as_ref().unwrap(), 100).unwrap();
    tx.commit().unwrap();
    assert!(
        matches!(f.tool(&child, "cycle", "delegate", args("grokbot")),
        ToolOutcome::Done { output } if output["reason"].as_str().unwrap().contains("cycle"))
    );
    assert_eq!(f.children(&root).len(), 1);
}

#[test]
fn refused_legacy_queue_retry_is_persisted_and_never_promoted() {
    let mut f = Fixture::new(false);
    let root = f.root("legacy-root");
    let id = suspended(f.tool(&root, "child", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let child = model::set_running_at(&tx, edge.child_run_id.as_ref().unwrap(), 100).unwrap();
    tx.commit().unwrap();
    let nested = suspended(f.tool(&child, "legacy", "delegate", args("grandchild")));
    let a = suspended(f.tool(&root, "a", "delegate", args("other")));
    let b = suspended(f.tool(&root, "b", "delegate", args("other")));
    // Simulate a pre-fix nested queue while all three root slots are occupied.
    // New admission never queues a nested request or skips its FIFO slot.
    let tx = features::write_transaction(&db).unwrap();
    tx.execute(
        "UPDATE delegation_graph SET status=?1 WHERE id=?2",
        rusqlite::params![
            serde_json::to_string(&DelegationStatus::Queued).unwrap(),
            nested
        ],
    )
    .unwrap();
    let edge_b = graph::get(&tx, &b).unwrap().unwrap();
    graph::promote(
        &tx,
        &b,
        edge_b.child_run_id.as_ref().unwrap(),
        edge_b.conversation_id.as_ref().unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(
        matches!(f.tool(&child, "legacy", "delegate", args("grandchild")),
        ToolOutcome::Done { output } if output["status"]["status"]=="refused")
    );
    let edge = graph::get(&db, &nested).unwrap().unwrap();
    assert!(matches!(edge.status, DelegationStatus::Refused { .. }));
    assert_eq!(
        store::required(&db, edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status,
        RunStatus::Cancelled
    );
    let tx = features::write_transaction(&db).unwrap();
    graph::set_status(&tx, &a, &DelegationStatus::Cancelled).unwrap();
    assert!(
        grokbot_delegation::admission::promotable(&tx, &Default::default())
            .unwrap()
            .is_empty()
    );
    tx.commit().unwrap();
}

#[test]
fn hook_storage_failure_is_reconciled_on_tick() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "retry-wake", args("child"));
    let db = f.db();
    db.execute_batch("CREATE TRIGGER reject_child_wake BEFORE INSERT ON engine_run_wakes BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    f.events(vec![
        call(
            "done",
            "complete_task",
            json!({"summary":"Recovered summary"}),
        ),
        completed(),
    ]);
    f.tick();
    wait("completed engine run", || {
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status
            == RunStatus::Finished
    });
    wait("released child", || f.provider.status()["titling"] == false);
    assert_eq!(
        graph::get(&db, &edge.id).unwrap().unwrap().status,
        DelegationStatus::Running
    );
    assert!(store::pending_wakes(&db).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER reject_child_wake").unwrap();
    f.tick();
    assert!(
        matches!(graph::get(&db, &edge.id).unwrap().unwrap().status, DelegationStatus::Done { result } if result=="Recovered summary")
    );
    assert_eq!(
        store::pending_wakes(&db)
            .unwrap()
            .iter()
            .filter(|w| w.run_id == parent.run_id)
            .count(),
        1
    );
    f.resume(&c, "retry-wake");
}

#[test]
fn child_human_input_routes_to_principal_and_is_cancelled_with_parent() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "human-child", args("child"));
    f.events(vec![call("form", "request_form", json!({"fields":[{"name":"answer","label":"Answer","field_type":{"kind":"text"},"required":true}]})), completed()]);
    f.tick();
    wait("child human gate", || {
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status
            == RunStatus::AwaitingHumanInput
    });
    wait("released child", || f.provider.status()["titling"] == false);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let request = form_request(&state);
    assert_eq!(request["route"]["target_conversation_id"], c);
    assert_eq!(request["route"]["target_bot_id"], "grokbot");
    assert_eq!(request["route"]["requesting_bot_id"], "child");
    let entry = request["entry_id"].as_str().unwrap().to_owned();
    f.post(
        "/api/stop",
        json!({"conversation_id":c,"message_id":"human-child"}),
    );
    assert_eq!(
        f.run(&parent.conversation_id, "human-child").status,
        RunStatus::Cancelled
    );
    assert!(matches!(
        grokbot_forms::requests::get(&f.db(), &entry)
            .unwrap()
            .unwrap()
            .status,
        grokbot_forms::requests::RequestStatus::Cancelled { .. }
    ));
}

#[test]
fn signout_can_cancel_a_child_before_its_first_tick() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "signout-child", args("child"));
    assert_eq!(
        store::required(&f.db(), edge.child_run_id.as_ref().unwrap())
            .unwrap()
            .status,
        RunStatus::Queued
    );
    f.post("/api/provider/signout", json!({}));
    assert_eq!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
    assert_eq!(f.provider.status()["connected"], false);
}

#[test]
fn malformed_task_conversation_keys_remain_rejected() {
    let mut f = Fixture::new(false);
    let long = "x".repeat(129);
    let huge = "x".repeat(513);
    for id in [
        "task:".to_owned(),
        "task::root".into(),
        "task:child:".into(),
        "other:child:root".into(),
        "task:child:root:extra".into(),
        "task:child:root/escape".into(),
        "task:child:root%3Aextra".into(),
        "task:child:røøt".into(),
        "task:child:root\n".into(),
        "task:a:task:b:task:c:task:d:root".into(),
        format!("task:{long}:root"),
        format!("task:child:{long}"),
        long,
        huge,
    ] {
        let error = f
            .app
            .api(
                "POST",
                "/api/stop",
                json!({"conversation_id":id,"message_id":"run"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap_err();
        assert_eq!(error.status, 400, "accepted malformed task key {id:?}");
        assert_eq!(error.message, "Identifiant invalide.");
    }
}

#[test]
fn nested_task_conversation_keys_can_address_the_deepest_child() {
    let mut f = Fixture::new(false);
    let root = f.root("nested-keys-root");
    let mut parent = root;
    let mut deepest = None;
    for (n, bot) in ["child", "grandchild", "other"].into_iter().enumerate() {
        let id = suspended(f.tool(&parent, &format!("key-{n}"), "delegate", args(bot)));
        let edge = graph::get(&f.db(), &id).unwrap().unwrap();
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        parent = model::set_running_at(&tx, edge.child_run_id.as_ref().unwrap(), 100).unwrap();
        tx.commit().unwrap();
        deepest = Some(edge);
    }
    let edge = deepest.unwrap();
    let key = edge.conversation_id.as_ref().unwrap();
    assert!(key.len() > 128 && key.len() <= 512);
    f.post(
        "/api/stop",
        json!({"conversation_id":key,"message_id":edge.child_run_id}),
    );
    assert_eq!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
}

#[test]
fn task_thread_accepts_stop_with_its_conversation_key() {
    let mut f = Fixture::new(false);
    let root = f.root("task-thread-root");
    let id = suspended(f.tool(&root, "task-thread", "delegate", args("child")));
    let edge = graph::get(&f.db(), &id).unwrap().unwrap();
    f.post(
        "/api/stop",
        json!({
            "conversation_id": edge.conversation_id,
            "message_id": edge.child_run_id,
        }),
    );
    assert_eq!(
        graph::get(&f.db(), &id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
}

#[test]
fn delegation_call_retries_keep_binding_and_terminal_result() {
    let mut f = Fixture::new(false);
    let root = f.root("retry-root");
    let id = suspended(f.tool(&root, "same", "delegate", args("child")));
    assert_eq!(
        suspended(f.tool(&root, "same", "delegate", args("child"))),
        id
    );
    let mut changed = args("child");
    changed["objective"] = json!("Changed");
    assert!(matches!(
        f.tool(&root, "same", "delegate", changed),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(f.children(&root).len(), 1);
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    graph::set_status(
        &tx,
        &id,
        &DelegationStatus::Done {
            result: "stable result".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(
        matches!(f.tool(&root,"same","delegate",args("child")),ToolOutcome::Done{output} if output["status"]["result"]=="stable result")
    );
    // Sortie sans argument figée sur 9cfe300b, avant la portée « fil » (#298).
    let ToolOutcome::Done { output } = f.tool(&root, "check", "check_delegations", json!({}))
    else {
        panic!("check_delegations must succeed");
    };
    assert_eq!(
        output,
        json!({"delegations":[{
            "id":id,"objective":"Verify selected material","parent_run_id":"retry-root",
            "root_run_id":"retry-root","parent_bot":"grokbot","child_bot":"child","depth":1,
            "child_run_id":format!("run-{id}"),
            "conversation_id":format!("task:{id}:{}",root.conversation_id),
            "parent_conversation_id":root.conversation_id,
            "status":{"status":"done","result":"stable result"}
        }]})
    );
}

#[test]
fn selected_image_reaches_the_child_request_and_not_another_thread() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
    const DESCRIPTION: &str = "Carré PNG d’un pixel, fond clair.";
    let png = STANDARD.decode(PNG).unwrap();
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let image = f.app.api("POST", "/api/attachments", &png).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut targeted = args("child");
    targeted["objective"] = json!("Décris cette photo");
    targeted["selected_context"]["texts"] = json!(["photo.png"]);
    targeted["selected_context"]["selected_images"] = json!([image.clone()]);
    // The send would otherwise start a caption call that consumes the scripted
    // child reply. Prepare the same draft the message will carry.
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Légende prête."}),
        completed(),
    ]);
    let caption = json!({
        "conversation_id": c,
        "attachment_id": image,
        "draft": "Délègue la description de cette photo."
    });
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        call("delegate-photo", "delegate", targeted),
        completed(),
    ]);
    f.post(
        "/api/send",
        json!({
            "conversation_id": c,
            "message_id": "describe-photo",
            "body": "Délègue la description de cette photo.",
            "attachments": [image]
        }),
    );
    f.wait_parent(&c, "describe-photo", RunStatus::AwaitingChildren);
    let edge = f.children(&f.run(&c, "describe-photo")).remove(0);
    f.child_result(&edge, DESCRIPTION);
    let seen = f.server.seen.lock().unwrap();
    let child = seen
        .bodies
        .iter()
        .rev()
        .find(|body| {
            body["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("executing a delegated Task")
                && body.to_string().contains("Décris cette photo")
        })
        .expect("requête de l'enfant")
        .clone();
    drop(seen);
    let child_json = child.to_string();
    assert!(
        child_json.contains(&format!("data:image/png;base64,{PNG}")),
        "la requête de l'enfant ne contient pas les octets de l'image"
    );
    // Delegated blocks are JSON-fenced, so the descriptor's quotes are escaped once.
    // Object keys are alphabetical: mime and path precede kind.
    let instructions = child["instructions"]
        .as_str()
        .unwrap_or("")
        .replace("\\\"", "\"");
    let marker = "\"kind\":\"selected_images\"";
    let at = instructions
        .find(marker)
        .expect("selected_images absent de la requête de l'enfant");
    let start = instructions[..at]
        .rfind('{')
        .expect("objet selected_images introuvable");
    let end = instructions[at..]
        .find('}')
        .expect("objet selected_images non fermé")
        + at
        + 1;
    let selected = &instructions[start..end];
    assert!(
        selected.contains("\"mime_type\":\"image/png\""),
        "selected_images n'a pas le type MIME"
    );
    let path_key = "\"path\":\"";
    let path_at = selected
        .find(path_key)
        .expect("selected_images n'a pas le chemin local");
    let path = &selected[path_at + path_key.len()..];
    let path = &path[..path.find('"').unwrap()];
    assert_eq!(
        std::fs::read(path).unwrap(),
        png,
        "le chemin de selected_images ne pointe pas vers les octets"
    );
    assert!(
        !selected
            .split("\"mime_type\"")
            .next()
            .unwrap_or("")
            .contains("photo.png"),
        "le nom du fichier ne doit pas tenir lieu d'image"
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let thread = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == json!(edge.conversation_id))
        .unwrap();
    let messages = thread["messages"].as_array().unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m["body"].as_str().unwrap_or("").contains(DESCRIPTION)),
        "le fil task: ne décrit pas l'image"
    );
    let brief = messages
        .iter()
        .find(|m| m["kind"] == "delegation_brief")
        .expect("consigne du fil task:");
    assert!(
        brief["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == image && a["mime_type"] == "image/png"),
        "le fil task: ne montre pas l'image jointe"
    );
    f.settle(&c, "describe-photo");

    let mut name_only = args("child");
    name_only["objective"] = json!("Décris photo.png");
    name_only["selected_context"]["texts"] = json!(["photo.png"]);
    let (_, name_edge) = f.begin(&c, "name-only", name_only);
    f.child_result(&name_edge, "Je n'ai que le nom photo.png.");
    let seen = f.server.seen.lock().unwrap();
    let named = seen
        .bodies
        .iter()
        .rev()
        .find(|body| {
            body["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("executing a delegated Task")
                && body.to_string().contains("Décris photo.png")
        })
        .expect("requête du fil qui n'a sélectionné que le nom")
        .clone();
    drop(seen);
    assert!(
        !named.to_string().contains(PNG)
            && !named["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("selected_images"),
        "le nom du fichier a suffi à transmettre l'image"
    );
    f.settle(&c, "name-only");

    let other = f.conversation();
    let mut elsewhere = args("child");
    elsewhere["objective"] = json!("Décris ce fil vide");
    let (_, other_edge) = f.begin(&other, "other-thread", elsewhere);
    f.child_result(&other_edge, "Ce fil n'a pas de photo.");
    let seen = f.server.seen.lock().unwrap();
    let foreign = seen
        .bodies
        .iter()
        .rev()
        .find(|body| {
            body["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("executing a delegated Task")
                && body.to_string().contains("Décris ce fil vide")
        })
        .expect("requête de l'autre fil")
        .clone();
    drop(seen);
    assert!(
        !foreign.to_string().contains(PNG) && !foreign.to_string().contains(&image),
        "un autre fil a placé l'image dans la requête de l'enfant"
    );
    f.settle(&other, "other-thread");

    let outsider = f.root("other-thread-root");
    let mut stolen = args("child");
    stolen["selected_context"]["selected_images"] = json!([image]);
    stolen["selected_context"]["texts"] = json!(["photo.png"]);
    assert!(
        matches!(
            f.tool(&outsider, "steal", "delegate", stolen),
            ToolOutcome::Failed { .. }
        ),
        "une image d'un autre fil a été acceptée dans selected_images"
    );
    assert!(f.children(&outsider).is_empty());
}

#[test]
fn selected_image_follows_when_the_task_input_is_forwarded() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
    let png = STANDARD.decode(PNG).unwrap();
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let image = f.app.api("POST", "/api/attachments", &png).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut targeted = args("child");
    targeted["objective"] = json!("Décris cette photo");
    targeted["selected_context"]["selected_images"] = json!([image.clone()]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Légende prête."}),
        completed(),
    ]);
    let caption = json!({
        "conversation_id": c,
        "attachment_id": image,
        "draft": "Transmets cette photo."
    });
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        call("delegate-photo", "delegate", targeted),
        completed(),
    ]);
    f.post(
        "/api/send",
        json!({
            "conversation_id": c,
            "message_id": "forward-photo",
            "body": "Transmets cette photo.",
            "attachments": [image]
        }),
    );
    f.wait_parent(&c, "forward-photo", RunStatus::AwaitingChildren);
    let edge = f.children(&f.run(&c, "forward-photo")).remove(0);
    let mut forwarded = args("grandchild");
    forwarded["objective"] = json!("Décris la photo un niveau plus bas");
    forwarded["selected_context"]["entry_ids"] = json!([edge.id]);
    f.events(vec![
        call("forward-task", "delegate", forwarded),
        completed(),
        call(
            "finish-grandchild",
            "complete_task",
            json!({"summary":"Photo reçue"}),
        ),
        completed(),
        call(
            "finish-child",
            "complete_task",
            json!({"summary":"Transmise"}),
        ),
        completed(),
    ]);
    wait("grandchild request", || {
        f.allow_pending_background();
        f.tick();
        f.server.seen.lock().unwrap().bodies.iter().any(|body| {
            body["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("executing a delegated Task")
                && body
                    .to_string()
                    .contains("Décris la photo un niveau plus bas")
        })
    });
    let seen = f.server.seen.lock().unwrap();
    let grandchild = seen
        .bodies
        .iter()
        .rev()
        .find(|body| {
            body["instructions"]
                .as_str()
                .unwrap_or("")
                .contains("executing a delegated Task")
                && body
                    .to_string()
                    .contains("Décris la photo un niveau plus bas")
        })
        .expect("requête du petit-enfant")
        .clone();
    drop(seen);
    let grandchild_json = grandchild.to_string();
    assert!(
        grandchild_json.contains(&format!("data:image/png;base64,{PNG}")),
        "la requête du petit-enfant ne contient pas les octets de l'image transmise avec l'entrée"
    );
    let instructions = grandchild["instructions"]
        .as_str()
        .unwrap_or("")
        .replace("\\\"", "\"");
    let marker = "\"kind\":\"selected_images\"";
    let at = instructions
        .find(marker)
        .expect("selected_images absent de la requête du petit-enfant");
    let start = instructions[..at]
        .rfind('{')
        .expect("objet selected_images introuvable");
    let end = instructions[at..]
        .find('}')
        .expect("objet selected_images non fermé")
        + at
        + 1;
    let selected = &instructions[start..end];
    let path_key = "\"path\":\"";
    let path_at = selected
        .find(path_key)
        .expect("selected_images n'a pas le chemin local");
    let path = &selected[path_at + path_key.len()..];
    let path = &path[..path.find('"').unwrap()];
    assert_eq!(
        std::fs::read(path).unwrap(),
        png,
        "le chemin transmis au petit-enfant ne pointe pas vers les octets"
    );
}

#[test]
fn selected_images_captions_and_documents_reach_child_after_restart_only() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let image = f.app.api("POST","/api/attachments",&STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=").unwrap()).unwrap()["id"].as_str().unwrap().to_owned();
    let doc = f.post("/api/documents",json!({"name":"preuve.txt","data":STANDARD.encode("DOCUMENT_SELECTIONNE_éè — contenu local") }))["id"].as_str().unwrap().to_owned();
    // Persist the caption through the existing preparation path.
    f.events(vec![json!({"type":"response.output_text.delta","delta":"LEGENDE_SELECTIONNEE : un carré clair."}),completed()]);
    let caption = json!({"conversation_id":c,"attachment_id":image,"draft":"Pièces à vérifier"});
    f.post("/api/attachments/precaption", caption.clone());
    wait("selected caption", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Pièces reçues."}),
        completed(),
    ]);
    f.post("/api/send",json!({"conversation_id":c,"message_id":"selected-files","body":"","attachments":[image,doc]}));
    f.wait_parent(&c, "selected-files", RunStatus::Finished);
    f.send(&c, "private-entry", "TEXTE_NON_SELECTIONNE");
    f.wait_parent(&c, "private-entry", RunStatus::Finished);
    let mut a = args("child");
    a["selected_context"]["entry_ids"] = json!(["selected-files"]);
    let (_, edge) = f.begin(&c, "delegate-files", a);
    // Housekeeping sees the parent's selected material even when the child is
    // still queued and has not copied anything into its own model snapshot.
    assert_eq!(f.post("/api/storage/preview", json!({}))["count"], 0);
    f.db()
        .execute("UPDATE attachments_staged SET staged_at=0", [])
        .unwrap();
    f.db()
        .execute("UPDATE wrapper_storage_staged SET staged_at=0", [])
        .unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(f.post("/api/storage/preview", json!({}))["count"], 0);
    f.child_done(&edge);
    let seen = f.server.seen.lock().unwrap();
    let outgoing = seen.bodies.last().unwrap().to_string();
    assert!(outgoing.contains("data:image/png;base64,"));
    assert!(outgoing.contains("LEGENDE_SELECTIONNEE"));
    assert!(outgoing.contains("DOCUMENT_SELECTIONNE"));
    assert!(!outgoing.contains("TEXTE_NON_SELECTIONNE"));
    assert!(!seen.bodies.last().unwrap()["instructions"]
        .to_string()
        .contains("data:image"));
    drop(seen);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let thread = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == json!(edge.conversation_id))
        .unwrap();
    let objective = thread["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == edge.id)
        .unwrap();
    assert_eq!(objective["attachments"].as_array().unwrap().len(), 2);
}

#[test]
fn followup_during_final_text_gets_another_model_call() {
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "text-followup", args("child"));
    let gate = Arc::new(AtomicBool::new(false));
    let at = f.server.seen.lock().unwrap().bodies.len() + 1;
    f.server.seen.lock().unwrap().response_gate = Some((at, gate.clone()));
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Ancienne réponse"}),
        completed(),
    ]);
    f.events(vec![
        call(
            "new-end",
            "complete_task",
            json!({"summary":"Nouvelle réponse"}),
        ),
        completed(),
    ]);
    f.tick();
    wait("child text request", || {
        f.server.seen.lock().unwrap().bodies.len() >= at
    });
    f.post(
        "/api/delegation/send",
        task_input(&parent, &edge, "text-note"),
    );
    gate.store(true, Ordering::SeqCst);
    wait(
        "follow-up after text",
        || matches!(graph::get(&f.db(),&edge.id).unwrap().unwrap().status,DelegationStatus::Done{ref result} if result == "Nouvelle réponse"),
    );
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(body.contains("Ancienne réponse") && body.contains("Ajoute une conclusion française."));
}

#[test]
fn individual_cancel_wins_over_late_running_completion() {
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "cancel-live", args("child"));
    let gate = Arc::new(AtomicBool::new(false));
    let at = f.server.seen.lock().unwrap().bodies.len() + 1;
    f.server.seen.lock().unwrap().response_gate = Some((at, gate.clone()));
    f.events(vec![
        call("too-late", "complete_task", json!({"summary":"Late"})),
        completed(),
    ]);
    f.tick();
    wait("cancel in flight", || {
        f.server.seen.lock().unwrap().bodies.len() >= at
    });
    f.post(
        "/api/delegation/cancel",
        json!({"parent_conversation_id":parent.conversation_id,"task_id":edge.id}),
    );
    gate.store(true, Ordering::SeqCst);
    wait("cancel releases owner", || {
        !f.provider.status()["titling"].as_bool().unwrap()
    });
    assert_eq!(
        graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
    assert_eq!(
        store::required(&f.db(), edge.child_run_id.as_deref().unwrap())
            .unwrap()
            .status,
        RunStatus::Cancelled
    );
    assert_eq!(
        store::required(&f.db(), &parent.run_id).unwrap().status,
        RunStatus::AwaitingChildren
    );
}

#[test]
fn selected_files_are_revoked_on_reset_and_missing_bytes_fail_honestly() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    for reset in [true, false] {
        let mut f = Fixture::new(true);
        let c = f.conversation();
        let file = f.app.api("POST","/api/attachments",&STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=").unwrap()).unwrap();
        f.events(vec![
            json!({"type":"response.output_text.delta","delta":"Image reçue"}),
            completed(),
        ]);
        f.post("/api/send",json!({"conversation_id":c,"message_id":"revoke-source","body":"Image sélectionnée","attachments":[file["id"]]}));
        f.wait_parent(&c, "revoke-source", RunStatus::Finished);
        let mut a = args("child");
        a["selected_context"]["entry_ids"] = json!(["revoke-source"]);
        let (_, edge) = f.begin(&c, "revoke-child", a);
        if reset {
            features::conversations::append_entry(&f.db(), &c, "reset", "reset-selection", 123)
                .unwrap();
            f.child_done(&edge);
            let body = f
                .server
                .seen
                .lock()
                .unwrap()
                .bodies
                .last()
                .unwrap()
                .to_string();
            assert!(!body.contains("data:image/"));
            assert!(body.contains("revoked/unavailable"));
        } else {
            let directory = PathBuf::from(format!("{}.attachments", f.path.display()));
            std::fs::remove_file(directory.join(file["sha256"].as_str().unwrap())).unwrap();
            let before = f.server.seen.lock().unwrap().bodies.len();
            f.tick();
            wait("missing selected blob fails", || {
                matches!(
                    graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
                    DelegationStatus::Failed { .. }
                )
            });
            assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
        }
    }
}

#[test]
fn followup_does_not_bypass_pending_human_gate() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "human-gate-followup", args("child"));
    f.events(vec![call("pending-form","request_form",json!({"fields":[{"name":"answer","label":"Autorisation","field_type":{"kind":"text"},"required":true}]})),completed()]);
    f.tick();
    wait("child awaits human", || {
        store::required(&f.db(), edge.child_run_id.as_deref().unwrap())
            .unwrap()
            .status
            == RunStatus::AwaitingHumanInput
    });
    let count = f.server.seen.lock().unwrap().bodies.len();
    f.post(
        "/api/delegation/send",
        task_input(&parent, &edge, "human-pending-note"),
    );
    f.tick();
    assert_eq!(
        store::required(&f.db(), edge.child_run_id.as_deref().unwrap())
            .unwrap()
            .status,
        RunStatus::AwaitingHumanInput
    );
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), count);
    assert_eq!(store::pending_wakes(&f.db()).unwrap().len(), 0);
}

#[test]
fn model_can_discover_real_recipient_and_selected_message_ids_without_private_history() {
    let mut f = Fixture::new(true);
    let hidden = f.conversation();
    f.send(&hidden, "hidden-catalogue", "NE_PAS_EXPOSER_AUTRE_FIL");
    f.wait_parent(&hidden, "hidden-catalogue", RunStatus::Finished);
    let c = f.conversation();
    f.events(vec![
        call("discover", "list_delegation_context", json!({})),
        completed(),
    ]);
    let mut a = args("child");
    a["selected_context"]["entry_ids"] = json!(["catalogue-entry"]);
    f.events(vec![
        call("delegate-discovered", "delegate", a),
        completed(),
    ]);
    f.send(&c, "catalogue-entry", "Sélection à confier au sous-bot");
    f.wait_parent(&c, "catalogue-entry", RunStatus::AwaitingChildren);
    let run = f.run(&c, "catalogue-entry");
    let events = store::events(&f.db(), &run.run_id, 0).unwrap();
    let output = &events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "discover")
        .unwrap()
        .payload["outcome"]["output"];
    assert!(output["bots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["id"] == "child"));
    assert!(!output["bots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["id"] == "grokbot"));
    assert_eq!(output["entries"][0]["entry_id"], "catalogue-entry");
    assert!(!output.to_string().contains("NE_PAS_EXPOSER_AUTRE_FIL"));
    assert!(
        f.server.seen.lock().unwrap().bodies.last().unwrap()["input"]
            .to_string()
            .contains("catalogue-entry")
    );
}

#[test]
fn group_member_can_delegate_and_receive_a_scoped_child_followup() {
    let mut f = Fixture::new(true);
    let c = f.post(
        "/api/conversations",
        json!({"bot_ids":["grokbot","other"],"title":"Groupe et sous-tâche"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.events(vec![
        call("from-group", "delegate", args("child")),
        completed(),
    ]);
    f.post("/api/send",json!({"conversation_id":c,"message_id":"group-root","body":"Analyse en sous-tâche","mentions":[{"id":"grokbot","label":"Ezerbot"}]}));
    f.tick();
    let mut group_delivered = false;
    wait("group delegates", || {
        let waiting = f
            .db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM engine_runs WHERE status='awaiting_approval')",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if waiting && (f.allow_pending_background() || !group_delivered) {
            // Group turns resume from the multibot driver, which a full tick runs
            // after the engine has already scanned queued children.
            f.tick();
            group_delivered = true;
        }
        if !waiting {
            group_delivered = false;
        }
        f.db()
            .query_row("SELECT COUNT(*) FROM delegation_graph", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            == 1
    });
    let edge_id: String = f
        .db()
        .query_row("SELECT id FROM delegation_graph", [], |r| r.get(0))
        .unwrap();
    let edge = graph::get(&f.db(), &edge_id).unwrap().unwrap();
    let parent = store::required(&f.db(), &edge.parent_run_id).unwrap();
    f.post(
        "/api/delegation/send",
        task_input(&parent, &edge, "group-child-note"),
    );
    f.child_done(&edge);
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string()
        .contains("Ajoute une conclusion française."));
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Résultat reçu dans le groupe."}),
        completed(),
    ]);
    wait("group parent resumes", || {
        f.tick();
        store::required(&f.db(), &parent.run_id).unwrap().status == RunStatus::Finished
    });
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let group = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == c)
        .unwrap();
    assert_eq!(group["kind"], "group");
    assert_eq!(group["member_ids"].as_array().unwrap().len(), 2);
    assert!(!group["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "group-child-note"));
    assert!(group["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "delegation" && m["payload"]["status"]["status"] == "done"));
}

#[test]
fn task_transcript_preserves_parent_provenance_followup_and_long_title_after_restart() {
    let mut f = Fixture::new(false);
    let root = f.root("transcript-provenance");
    let mut a = args("child");
    a["objective"] = json!("É".repeat(95));
    f.tool(&root, "provenance-child", "delegate", a);
    let edge = f.children(&root).remove(0);
    f.post(
        "/api/delegation/send",
        task_input(&root, &edge, "human-followup"),
    );
    for restart in [false, true] {
        if restart {
            f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
        }
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        let conversation = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == json!(edge.conversation_id))
            .unwrap();
        let title = conversation["title"].as_str().unwrap();
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with('…'));
        let messages = conversation["messages"].as_array().unwrap();
        let instruction = messages.iter().find(|m| m["id"] == edge.id).unwrap();
        assert_eq!(instruction["payload"]["source"], "delegation");
        assert_eq!(instruction["payload"]["from_bot"], root.bot_id);
        assert_eq!(instruction["payload"]["parent_bot"], root.bot_id);
        assert_eq!(instruction["role"], "assistant");
        assert_eq!(instruction["kind"], "delegation_brief");
        let followup = messages
            .iter()
            .find(|m| m["id"] == "human-followup")
            .unwrap();
        assert_eq!(followup["payload"]["source"], "follow_up");
        assert!(followup["payload"].get("from_bot").is_none());
        assert_eq!(followup["role"], "user");
    }
}

#[test]
fn followup_image_bytes_survive_tool_continuations_and_terminal_retry_is_idempotent() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "image-followup", args("child"));
    let image=f.app.api("POST","/api/attachments",&STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=").unwrap()).unwrap()["id"].clone();
    let gate = Arc::new(AtomicBool::new(false));
    let at = f.server.seen.lock().unwrap().bodies.len() + 1;
    f.server.seen.lock().unwrap().response_gate = Some((at, gate.clone()));
    f.events(vec![
        call("before-image", "complete_task", json!({"summary":"Old"})),
        completed(),
    ]);
    f.events(vec![
        call("image-continuation", "check_delegations", json!({})),
        completed(),
    ]);
    f.events(vec![
        call(
            "after-image",
            "complete_task",
            json!({"summary":"Image traitée"}),
        ),
        completed(),
    ]);
    f.tick();
    wait("image followup in flight", || {
        f.server.seen.lock().unwrap().bodies.len() >= at
    });
    let input = json!({"parent_conversation_id":parent.conversation_id,"task_id":edge.id,"message_id":"image-note","body":"Suivi avec image sans légende","attachments":[image]});
    f.post("/api/delegation/send", input.clone());
    // A direct child follow-up is not a parent submission, but is just as live.
    assert_eq!(f.post("/api/storage/preview", json!({}))["count"], 0);
    gate.store(true, Ordering::SeqCst);
    wait("image followup completed", || {
        matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Done { .. }
        )
    });
    let seen = f.server.seen.lock().unwrap();
    let requests: Vec<_> = seen
        .bodies
        .iter()
        .filter(|b| {
            b["input"]
                .to_string()
                .contains("Suivi avec image sans légende")
        })
        .collect();
    assert_eq!(requests.len(), 2, "first request and tool continuation");
    for body in requests {
        assert!(body["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["content"]
                .as_array()
                .is_some_and(
                    |content| content.iter().any(|item| item["type"] == "input_image"
                        && item["image_url"]
                            .as_str()
                            .is_some_and(|url| url.starts_with("data:image/png;base64,")))
                )));
    }
    drop(seen);
    let before = store::events(&f.db(), edge.child_run_id.as_deref().unwrap(), 0)
        .unwrap()
        .len();
    f.post("/api/delegation/send", input.clone());
    assert_eq!(
        store::events(&f.db(), edge.child_run_id.as_deref().unwrap(), 0)
            .unwrap()
            .len(),
        before
    );
    let mut fresh = input;
    fresh["message_id"] = json!("new-after-finish");
    assert_eq!(
        f.app
            .api("POST", "/api/delegation/send", fresh.to_string().as_bytes())
            .unwrap_err()
            .status,
        409
    );
}

fn task_input(parent: &Run, edge: &Delegation, id: &str) -> Value {
    json!({"parent_conversation_id":parent.conversation_id,"task_id":edge.id,"message_id":id,"body":"Ajoute une conclusion française."})
}

#[test]
fn queued_followup_persists_and_individual_cancel_preserves_siblings() {
    let mut f = Fixture::new(false);
    let root = f.root("queued-followup");
    for id in ["one", "two", "three", "four", "five"] {
        f.tool(&root, id, "delegate", args("child"));
    }
    let edges = f.children(&root);
    let queued = edges
        .iter()
        .find(|d| d.status == DelegationStatus::Queued)
        .unwrap()
        .clone();
    let input = task_input(&root, &queued, "queued-note");
    f.post("/api/delegation/send", input.clone());
    f.post("/api/delegation/send", input.clone());
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM wrapper_delegation_inputs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert!(store::get(&f.db(), queued.child_run_id.as_deref().unwrap())
        .unwrap()
        .is_none());
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(queued.conversation_id))
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "queued-note"));
    let victim = edges
        .iter()
        .find(|d| d.status == DelegationStatus::Running)
        .unwrap();
    f.post(
        "/api/delegation/cancel",
        json!({"parent_conversation_id":root.conversation_id,"task_id":victim.id}),
    );
    assert_eq!(
        graph::get(&f.db(), &victim.id).unwrap().unwrap().status,
        DelegationStatus::Cancelled
    );
    assert!(!store::required(&f.db(), &root.run_id)
        .unwrap()
        .status
        .is_terminal());
    for sibling in edges.iter().filter(|d| d.id != victim.id) {
        assert!(matches!(
            graph::get(&f.db(), &sibling.id).unwrap().unwrap().status,
            DelegationStatus::Running | DelegationStatus::Queued
        ));
    }
    // Promotion transfers the persisted follow-up exactly once to the reserved run.
    let promoted = edges
        .iter()
        .filter(|d| d.status == DelegationStatus::Queued)
        .find(|d| graph::get(&f.db(), &d.id).unwrap().unwrap().status == DelegationStatus::Running)
        .unwrap();
    f.post(
        "/api/delegation/send",
        task_input(&root, promoted, "promoted-note"),
    );
    let events = store::events(&f.db(), promoted.child_run_id.as_deref().unwrap(), 0).unwrap();
    assert!(events.iter().any(|e| e.kind == RunEventKind::InputReceived));
    let still_queued = edges
        .iter()
        .find(|d| graph::get(&f.db(), &d.id).unwrap().unwrap().status == DelegationStatus::Queued)
        .unwrap();
    f.post(
        "/api/delegation/cancel",
        json!({"parent_conversation_id":root.conversation_id,"task_id":still_queued.id}),
    );
    assert!(
        store::get(&f.db(), still_queued.child_run_id.as_deref().unwrap())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        graph::get(&f.db(), &still_queued.id)
            .unwrap()
            .unwrap()
            .status,
        DelegationStatus::Cancelled
    );
}

#[test]
fn followup_identity_denial_terminal_and_invalid_files_are_atomic() {
    let mut f = Fixture::new(false);
    let root = f.root("followup-denial");
    f.tool(&root, "child", "delegate", args("child"));
    let edge = f.children(&root).remove(0);
    let other = f.conversation();
    let mut input = task_input(&root, &edge, "denied-note");
    input["parent_conversation_id"] = json!(other);
    assert_eq!(
        f.app
            .api("POST", "/api/delegation/send", input.to_string().as_bytes())
            .unwrap_err()
            .status,
        404
    );
    let mut input = task_input(&root, &edge, "bad-file");
    input["attachments"] = json!(["nonexistent"]);
    assert_eq!(
        f.app
            .api("POST", "/api/delegation/send", input.to_string().as_bytes())
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM wrapper_delegation_inputs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    f.post(
        "/api/delegation/send",
        task_input(&root, &edge, "immutable"),
    );
    let mut changed_files = task_input(&root, &edge, "immutable");
    changed_files["attachments"] = json!(["nonexistent"]);
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/delegation/send",
                changed_files.to_string().as_bytes()
            )
            .unwrap_err()
            .status,
        409
    );
    f.tool(&root, "second-recipient", "delegate", args("other"));
    let sibling = f
        .children(&root)
        .into_iter()
        .find(|d| d.id != edge.id)
        .unwrap();
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/delegation/send",
                task_input(&root, &sibling, "immutable")
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        409
    );
    let mut changed = task_input(&root, &edge, "immutable");
    changed["body"] = json!("Différent");
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/delegation/send",
                changed.to_string().as_bytes()
            )
            .unwrap_err()
            .status,
        409
    );
    f.post(
        "/api/delegation/cancel",
        json!({"parent_conversation_id":root.conversation_id,"task_id":edge.id}),
    );
    for (path, input) in [
        ("/api/delegation/send", task_input(&root, &edge, "finished")),
        (
            "/api/send",
            json!({"conversation_id":edge.conversation_id,"message_id":"finished","body":"Encore"}),
        ),
    ] {
        let error = f
            .app
            .api("POST", path, input.to_string().as_bytes())
            .unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(
            error.message,
            "Cette tâche est terminée ; poursuivez dans la conversation principale."
        );
    }
    assert_eq!(
        store::runs_for_conversation(&f.db(), edge.conversation_id.as_deref().unwrap())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn running_followup_beats_stale_completion_and_reaches_same_child_run() {
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "running-note", args("child"));
    let gate = Arc::new(AtomicBool::new(false));
    let at = f.server.seen.lock().unwrap().bodies.len() + 1;
    f.server.seen.lock().unwrap().response_gate = Some((at, gate.clone()));
    f.events(vec![
        call(
            "stale-completion",
            "complete_task",
            json!({"summary":"Old answer"}),
        ),
        completed(),
    ]);
    f.events(vec![
        call(
            "fresh-completion",
            "complete_task",
            json!({"summary":"Conclusion française ajoutée."}),
        ),
        completed(),
    ]);
    f.tick();
    wait("child request in flight", || {
        f.server.seen.lock().unwrap().bodies.len() >= at
    });
    let input = task_input(&parent, &edge, "live-note");
    f.post("/api/delegation/send", input.clone());
    f.post("/api/delegation/send", input);
    gate.store(true, Ordering::SeqCst);
    wait(
        "updated task completion",
        || matches!(graph::get(&f.db(),&edge.id).unwrap().unwrap().status,DelegationStatus::Done{ref result} if result.contains("française")),
    );
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string()
        .contains("Ajoute une conclusion française."));
    let events = store::events(&f.db(), edge.child_run_id.as_deref().unwrap(), 0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == RunEventKind::InputReceived)
            .count(),
        1
    );
    assert!(events.iter().any(|e| e.kind == RunEventKind::ToolResult
        && e.payload["call_id"] == "stale-completion"
        && e.payload["outcome"]["status"] == "failed"));
}

#[test]
fn background_delegate_card_saves_the_action_survives_restart_and_revoke_asks_again() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let mut first = args("child");
    first["objective"] = json!(format!("Préparer {}", "le dossier ".repeat(30)));
    f.events(vec![call("delegate-call", "delegate", first), completed()]);
    f.send(&c, "bg-task", "Lance la tâche de fond");
    wait("carte de tâche", || {
        f.run(&c, "bg-task").status == RunStatus::AwaitingApproval
    });
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let card = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["messages"].as_array().unwrap().clone())
        .find(|message| message["kind"] == "approval")
        .expect("carte");
    assert_eq!(card["payload"]["policy"]["scope"], "action");
    assert_eq!(card["payload"]["policy"]["remember"], true);
    assert_eq!(
        card["payload"]["background_task"]["kind"],
        "Exécuter une tâche sur l'ordinateur"
    );
    assert_eq!(card["payload"]["background_task"]["bot_name"], "Ezerbot");
    assert!(card["payload"]["background_task"]["summary"]
        .as_str()
        .unwrap()
        .ends_with("… (raccourci)"));
    f.post(
        "/api/approvals/resolve",
        json!({
            "entry_id": card["payload"]["entry_id"],
            "submission_id": "always-bg",
            "conversation_id": card["payload"]["route"]["target_conversation_id"],
            "decision": {"always_allow": {"scope": "global"}}
        }),
    );
    f.wait_parent(&c, "bg-task", RunStatus::AwaitingChildren);
    let settled = f.app.api("GET", "/api/state", b"").unwrap();
    let shown = settled["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["messages"].as_array().unwrap().clone())
        .find(|message| message["id"] == card["id"])
        .unwrap();
    assert!(shown["payload"]["decision"]["always_allow"].is_object());
    f.events(vec![
        call("delegate-call", "delegate", args("child")),
        completed(),
    ]);
    f.send(&c, "bg-next", "La même sorte de tâche");
    f.wait_parent(&c, "bg-next", RunStatus::AwaitingChildren);
    assert_ne!(f.run(&c, "bg-next").status, RunStatus::AwaitingApproval);
    f.reopen();
    f.events(vec![
        call("delegate-call", "delegate", args("other")),
        completed(),
    ]);
    f.send(&c, "bg-restart", "Après redémarrage");
    f.wait_parent(&c, "bg-restart", RunStatus::AwaitingChildren);
    f.post(
        "/api/approvals/rules/revoke",
        json!({
            "requesting_bot_id": "grokbot",
            "target_bot_id": "grokbot",
            "tool": "background_computer",
            "args_digest": "",
            "scope": "global"
        }),
    );
    f.events(vec![
        call("delegate-call", "delegate", args("child")),
        completed(),
    ]);
    f.send(&c, "bg-revoked", "La règle est retirée");
    wait("carte revenue", || {
        f.run(&c, "bg-revoked").status == RunStatus::AwaitingApproval
    });
}

#[test]
fn a_delegated_child_can_use_its_granted_image_as_a_generation_reference() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
    let png = STANDARD.decode(PNG).unwrap();
    let mut f = Fixture::new(true);
    image_session(&f);
    let c = f.conversation();
    let image = f.app.api("POST", "/api/attachments", &png).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut targeted = args("child");
    targeted["objective"] = json!("Dessine dans le style de cette photo");
    targeted["selected_context"]["selected_images"] = json!([image.clone()]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Légende prête."}),
        completed(),
    ]);
    let caption = json!({
        "conversation_id": c,
        "attachment_id": image,
        "draft": "Délègue le dessin dans ce style."
    });
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        call("delegate-style", "delegate", targeted),
        completed(),
    ]);
    f.post(
        "/api/send",
        json!({
            "conversation_id": c,
            "message_id": "style-photo",
            "body": "Délègue le dessin dans ce style.",
            "attachments": [image]
        }),
    );
    f.wait_parent(&c, "style-photo", RunStatus::AwaitingChildren);
    let edge = f.children(&f.run(&c, "style-photo")).remove(0);
    f.events(vec![
        call(
            "child-draw",
            "generate_image",
            json!({"description":"un phare dans ce style","aspect_ratio":"16:9","reference_image_paths":[image]}),
        ),
        completed(),
    ]);
    f.child_result(&edge, "Image générée.");
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(
        seen.image_calls.len(),
        1,
        "l'enfant n'a pas pu citer l'image qui lui a été confiée"
    );
    let content = seen.image_calls[0]["input"][0]["content"]
        .as_array()
        .expect("la référence doit partir avec la demande");
    assert_eq!(
        content[1]["image_url"],
        format!("data:image/png;base64,{PNG}")
    );
    assert_eq!(seen.image_calls[0]["tools"][0]["size"], "1536x1024");
}

#[test]
fn a_delegated_child_cannot_use_a_revoked_image_as_a_generation_reference() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
    let png = STANDARD.decode(PNG).unwrap();
    let mut f = Fixture::new(true);
    image_session(&f);
    let c = f.conversation();
    let image = f.app.api("POST", "/api/attachments", &png).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut targeted = args("child");
    targeted["objective"] = json!("Dessine dans le style de cette photo");
    targeted["selected_context"]["selected_images"] = json!([image.clone()]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Légende prête."}),
        completed(),
    ]);
    let caption = json!({
        "conversation_id": c,
        "attachment_id": image,
        "draft": "Délègue le dessin dans ce style."
    });
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        call("delegate-style", "delegate", targeted),
        completed(),
    ]);
    f.post(
        "/api/send",
        json!({
            "conversation_id": c,
            "message_id": "style-photo",
            "body": "Délègue le dessin dans ce style.",
            "attachments": [image]
        }),
    );
    f.wait_parent(&c, "style-photo", RunStatus::AwaitingChildren);
    let edge = f.children(&f.run(&c, "style-photo")).remove(0);
    features::conversations::append_entry(&f.db(), &c, "reset", "reset-selection", 123).unwrap();
    f.events(vec![
        call(
            "child-draw",
            "generate_image",
            json!({"description":"un phare dans ce style","aspect_ratio":"16:9","reference_image_paths":[image]}),
        ),
        completed(),
    ]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Référence refusée."}),
        call(
            "finish-child",
            "complete_task",
            json!({"summary":"Référence refusée."}),
        ),
        completed(),
    ]);
    f.tick();
    wait("child settled", || {
        !matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Running | DelegationStatus::Queued
        )
    });
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(
        seen.image_calls.len(),
        0,
        "une image révoquée ne doit plus partir"
    );
}

/// The delegation fixture's sign-in token is opaque; image generation needs one
/// that carries an account, as a real ChatGPT session does.
fn image_session(f: &Fixture) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let session = format!(
        "e30.{}.e30",
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":"account-child"}})
                .to_string()
        )
    );
    std::fs::write(f.path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"account_id":"account-child","access_token":session,"refresh_token":"refresh-2","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
}

#[test]
fn a_child_can_use_an_image_of_a_selected_entry_as_a_generation_reference() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
    let mut f = Fixture::new(true);
    image_session(&f);
    let c = f.conversation();
    let image = f
        .app
        .api("POST", "/api/attachments", &STANDARD.decode(PNG).unwrap())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Légende prête."}),
        completed(),
    ]);
    let caption = json!({"conversation_id":c,"attachment_id":image,"draft":"Photo de style"});
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Photo reçue."}),
        completed(),
    ]);
    f.post("/api/send",json!({"conversation_id":c,"message_id":"style-entry","body":"Photo de style","attachments":[image]}));
    f.wait_parent(&c, "style-entry", RunStatus::Finished);
    let mut a = args("child");
    a["objective"] = json!("Dessine dans le style de cette photo");
    a["selected_context"]["entry_ids"] = json!(["style-entry"]);
    let (_, edge) = f.begin(&c, "delegate-entry", a);
    f.events(vec![
        call(
            "child-draw",
            "generate_image",
            json!({"description":"un phare dans ce style","aspect_ratio":"16:9","reference_image_paths":[image]}),
        ),
        completed(),
    ]);
    f.child_result(&edge, "Image générée.");
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(
        seen.image_calls.len(),
        1,
        "l'enfant n'a pas pu citer l'image du message sélectionné"
    );
    let content = seen.image_calls[0]["input"][0]["content"]
        .as_array()
        .expect("la référence doit partir avec la demande");
    assert_eq!(
        content[1]["image_url"],
        format!("data:image/png;base64,{PNG}")
    );
}

#[test]
fn yolo_launches_a_delegated_task_without_the_background_card() {
    let mut f = Fixture::new(true);
    f.post("/api/yolo/mode", json!({"mode":"yolo"}));
    let c = f.conversation();
    let (parent, edge) = f.begin(&c, "yolo-delegate", args("child"));
    let cards: i64 = f
        .db()
        .query_row("SELECT count(*) FROM forms_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        cards, 0,
        "le lancement d'une sous-tâche ne demande plus de carte"
    );
    assert_eq!(f.children(&parent).len(), 1);
    f.child_done(&edge);
    f.resume(&c, "yolo-delegate");
}

const DAY: u64 = 86_400_000;
impl Fixture {
    fn fil(&self, run: &Run) -> Value {
        match self.tool(run, "fil", "check_delegations", json!({"portee":"fil"})) {
            ToolOutcome::Done { output } => output,
            other => panic!("portee fil refusée : {other:?}"),
        }
    }
    fn tasks_block(&self, run: &Run) -> Option<String> {
        let db = self.db();
        let tx = features::write_transaction(&db).unwrap();
        let blocks = delegation::context(&self.context(&tx, run), &context_request(run));
        let texts = blocks.into_iter().map(|b| b.text);
        texts
            .into_iter()
            .find(|t| t.starts_with("Tâches lancées depuis ce fil"))
    }
    fn delegate_as(&self, run: &Run, objective: &str) {
        let mut task = args("child");
        task["objective"] = json!(objective);
        self.tool(run, "d", "delegate", task);
    }
    fn shell_task(&self, run: &Run, call: &str, status: &str, code: Option<i64>, command: &str) {
        let outcome = json!({"status":"done","output":{"command":command}});
        let db = self.db();
        db.execute(
            "INSERT INTO engine_external_tools(run_id,call_id,outcome) VALUES (?1,?2,?3)",
            rusqlite::params![run.run_id, call, outcome.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO wrapper_shell_tasks(run_id,call_id,status,exit_code) VALUES (?1,?2,?3,?4)",
            rusqlite::params![run.run_id, call, status, code],
        )
        .unwrap();
    }
}
fn summaries(output: &Value) -> Vec<&str> {
    let tasks = output["tasks"].as_array().unwrap();
    tasks
        .iter()
        .map(|t| t["summary"].as_str().unwrap())
        .collect()
}

#[test]
fn thread_scope_lists_a_task_started_in_an_earlier_reply() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let (_, edge) = f.begin(&c, "r1", args("child"));
    f.child_result(&edge, "private-prefix\nprivate-suffix");
    f.resume(&c, "r1");
    // Only the complete value is in the vault, and only since after the result was
    // stored: it must be masked before the result is cut to a sentence.
    let vault = Vault::open(&f.path.with_extension("secrets.json")).unwrap();
    vault
        .set("late", "test", "private-prefix\nprivate-suffix")
        .unwrap();
    f.events(vec![
        call("fil", "check_delegations", json!({"portee":"fil"})),
        completed(),
    ]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Voilà."}),
        completed(),
    ]);
    let before = f.server.seen.lock().unwrap().bodies.len();
    f.send(&c, "r2", "Où en est ma tâche ?");
    f.wait_parent(&c, "r2", RunStatus::Finished);
    let second = f.run(&c, "r2");
    let result = store::events(&f.db(), &second.run_id, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "fil")
        .unwrap();
    let output = &result.payload["outcome"]["output"];
    assert_eq!(summaries(output), ["Verify selected material"]);
    let task = &output["tasks"][0];
    assert_eq!(task["bot"], "child");
    assert_eq!(task["status"], "terminée");
    assert_eq!(task["result"], grokbot_forms::redact::MASK);
    let seen = f.server.seen.lock().unwrap();
    assert!(seen.bodies[before]
        .to_string()
        .contains("Tâches lancées depuis ce fil : 1 terminée."));
    assert!(seen.bodies[before..]
        .iter()
        .all(|body| !body.to_string().contains("private-prefix")));
}

#[test]
fn thread_scope_keeps_to_its_conversation_and_bot_and_labels_each_task() {
    let mut f = Fixture::new(false);
    let (a, b) = (f.root("iso-a"), f.root("iso-b"));
    assert_eq!(f.tasks_block(&a), None);
    assert_eq!(f.fil(&a), json!({"portee":"fil","tasks":[]}));
    f.delegate_as(&a, "Tâche A");
    f.delegate_as(&b, "Tâche B");
    f.shell_task(&b, "ok", "terminé", Some(0), "echo fil B");
    f.shell_task(&b, "ko", "échec", Some(3), "exit 3");
    f.shell_task(&b, "run", "en cours", None, "sleep 60");
    assert_eq!(summaries(&f.fil(&a)), ["Tâche A"]);
    let listed = f.fil(&b);
    assert_eq!(
        summaries(&listed),
        ["Tâche B", "echo fil B", "exit 3", "sleep 60"]
    );
    let failed = &listed["tasks"][2];
    assert_eq!(
        (
            failed["kind"].as_str(),
            failed["status"].as_str(),
            failed["exit_code"].as_i64()
        ),
        (Some("command"), Some("échec"), Some(3))
    );
    assert_eq!(
        f.tasks_block(&b).as_deref(),
        Some("Tâches lancées depuis ce fil : 2 en cours, 1 terminée, 1 en échec.")
    );
    let group = f.post(
        "/api/conversations",
        json!({"bot_ids":["grokbot","other"],"title":"Groupe"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (mine, theirs) = (
        f.run_in("grp-me", "grokbot", &group, 1),
        f.run_in("grp-other", "other", &group, 1),
    );
    f.delegate_as(&mine, "Tâche du groupe");
    f.shell_task(&theirs, "sh", "en cours", None, "echo autre bot");
    assert_eq!(summaries(&f.fil(&mine)), ["Tâche du groupe"]);
    assert_eq!(summaries(&f.fil(&theirs)), ["echo autre bot"]);
}

#[test]
fn thread_scope_lists_ten_tasks_of_the_last_day_but_counts_them_all() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.now_ms = 3 * DAY;
    let reply = |name: &str, ago: u64| f.run_in(name, "grokbot", &c, (f.now_ms - ago) as i64);
    let (newest, older) = (reply("new", 0), reply("older", 3_600_000));
    let (edge, stale) = (reply("edge", DAY), reply("stale", DAY + 1));
    for i in 0..3 {
        f.shell_task(&newest, &format!("n{i}"), "terminé", Some(0), "cmd new");
    }
    for i in 0..7 {
        f.shell_task(&older, &format!("o{i}"), "terminé", Some(0), "cmd older");
    }
    f.shell_task(&older, "o7", "en cours", None, "cmd running");
    f.shell_task(&edge, "e", "terminé", Some(0), "cmd edge");
    f.shell_task(&stale, "s", "terminé", Some(0), "cmd stale");
    // A command carries the start of its reply, so a delegation launched later in
    // that reply comes before it.
    f.now_ms -= 1_800_000;
    f.delegate_as(&older, "Plus tard");
    f.now_ms += 1_800_000;
    assert_eq!(
        summaries(&f.fil(&newest)),
        [
            ["cmd new"; 3].as_slice(),
            ["Plus tard"].as_slice(),
            ["cmd older"; 6].as_slice()
        ]
        .concat()
    );
    assert_eq!(
        f.tasks_block(&newest).as_deref(),
        Some("Tâches lancées depuis ce fil : 2 en cours, 11 terminées.")
    );
}

#[test]
fn thread_scope_forgets_the_tasks_a_reset_left_behind() {
    for kind in ["reset", "engine_reset"] {
        let mut f = Fixture::new(false);
        let c = f.conversation();
        let (old, new) = (
            f.run_in("old", "grokbot", &c, 100),
            f.run_in("new", "grokbot", &c, 300),
        );
        f.delegate_as(&old, "Avant");
        f.shell_task(&old, "sh", "terminé", Some(0), "echo avant");
        f.now_ms = 300;
        assert_eq!(summaries(&f.fil(&new)), ["Avant", "echo avant"]);
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        features::conversations::append_entry(&tx, &c, kind, "marker", 200).unwrap();
        tx.commit().unwrap();
        assert_eq!(f.fil(&new), json!({"portee":"fil","tasks":[]}));
        assert_eq!(f.tasks_block(&new), None);
        f.shell_task(&new, "sh", "terminé", Some(0), "echo après");
        assert_eq!(summaries(&f.fil(&new)), ["echo après"]);
    }
}
