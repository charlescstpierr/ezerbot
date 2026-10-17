//! Wrapper/engine integration, never another worker's target/ or database.
mod support;
use grokbot_delegation::graph::{self, Delegation, DelegationStatus};
use grokbot_engine::{
    contracts::*,
    model::{self, Run, RunStatus},
    store,
};
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
        let app = App::open_with(&path, provider.clone()).unwrap();
        if live {
            wait("catalog", || provider.status()["models_ready"] == true);
        }
        let mut f = Self {
            app,
            provider,
            server,
            path,
        };
        for id in ["child", "grandchild", "other"] {
            f.post("/api/bots",json!({"id":id,"name":id,"color":"citron","description":"Worker","instructions":"Complete delegated work using complete_task."}));
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
        let db = self.db();
        let tx = features::write_transaction(&db).unwrap();
        let r = model::start(
            &tx,
            &Run {
                run_id: id.into(),
                bot_id: "grokbot".into(),
                conversation_id: c,
                trigger: Trigger::UserMessage {
                    message_id: format!("manual/{id}"),
                    attempt: 0,
                },
                kind: RunKind::Chat,
                status: RunStatus::Queued,
                task_status: None,
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at: 1,
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
            now_ms: 100,
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
    fn wait_parent(&self, c: &str, m: &str, status: RunStatus) {
        wait("parent state", || {
            self.run(c, m).status == status && self.provider.status()["titling"] == false
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
        self.events(vec![
            call(
                "finish-child",
                "complete_task",
                json!({"summary":"Verified child summary"}),
            ),
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
fn suspended(outcome: ToolOutcome) -> String {
    match outcome {
        ToolOutcome::Suspend {
            cause: SuspendCause::Children { ids },
        } => ids[0].clone(),
        other => panic!("expected child suspension, got {other:?}"),
    }
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
    assert!(body.contains("Child run ended without completing its task"));
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
    let request = &state["forms"]["requests"][0];
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
    assert!(
        matches!(f.tool(&root,"check","check_delegations",json!({})),ToolOutcome::Done{output} if output["delegations"][0]["status"]["result"]=="stable result")
    );
}
