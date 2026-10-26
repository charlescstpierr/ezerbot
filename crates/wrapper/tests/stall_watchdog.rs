use grokbot_engine::{
    contracts::{
        with_tx, ApprovalCheck, ModelEvent, ModelGateway, ModelRequest, Result, RunBudgets,
        RunEvent, RunEventKind, RunKind, SuspendCause, TaskStatus, ToolCall, ToolOutcome, ToolSpec,
        Trigger, TurnEnd,
    },
    model::{self, Run, RunStatus},
    run_loop::{self, RunActivity, RunProgress, RunServices},
    store,
};
use grokbot_wrapper::{
    features::{
        self,
        engine::watchdog::{self, LATEST_SEQ_SQL, STALLED_ENTRY_SQL},
        Context,
    },
    App, Provider,
};
use rusqlite::{params, Connection, StatementStatus};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Fixture {
    app: App,
    conn: Connection,
    conversation: String,
    dir: grokbot_test_support::TestDir,
    start: u64,
}

impl Fixture {
    fn new() -> Self {
        let dir = grokbot_test_support::TestDir::new("stall-watchdog");
        let path = dir.join("workspace.sqlite");
        let mut app = App::open(&path).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conn = Connection::open(path).unwrap();
        conn.busy_timeout(Duration::from_secs(3)).unwrap();
        let start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        app.feature_context().provider.watchdog_test_clock(start);
        Self {
            app,
            conn,
            conversation,
            dir,
            start,
        }
    }

    fn provider(&self) -> Provider {
        self.app.feature_context().provider.clone()
    }

    fn run(&self, id: &str, status: RunStatus) {
        self.insert_run(
            id,
            &self.conversation,
            Trigger::UserMessage {
                message_id: format!("{}/{id}", self.conversation),
                attempt: 0,
            },
            status,
        );
    }

    fn insert_run(&self, id: &str, conversation: &str, trigger: Trigger, status: RunStatus) {
        let kind = if matches!(trigger, Trigger::UserMessage { .. }) {
            RunKind::Chat
        } else {
            RunKind::Task
        };
        with_tx(&self.conn, |conn| {
            model::start(
                conn,
                &Run {
                    run_id: id.into(),
                    bot_id: "grokbot".into(),
                    conversation_id: conversation.into(),
                    trigger: trigger.clone(),
                    kind,
                    status: RunStatus::Queued,
                    task_status: (kind == RunKind::Task).then_some(TaskStatus::Working),
                    turn_end: None,
                    budgets: RunBudgets {
                        duration_ms: u64::MAX,
                        ..RunBudgets::default()
                    },
                    started_at: self.start as i64,
                },
            )?;
            let mut run = model::set_running_at(conn, id, self.start as i64)?;
            store::append_event(
                conn,
                id,
                RunEventKind::ToolCall,
                &json!({"call_id":"call", "name":"slow", "arguments":{}, "response":1}),
            )?;
            run.status = status;
            store::save(conn, &run)?;
            Ok::<(), grokbot_engine::contracts::Error>(())
        })
        .unwrap();
    }

    fn child(&mut self, parent: &str, id: &str, status: RunStatus) {
        let edge = format!("edge-{id}");
        let conversation = self.post("/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.insert_run(
            id,
            &conversation,
            Trigger::Delegation {
                delegation_id: edge.clone(),
            },
            status,
        );
        self.conn
            .execute(
                "INSERT INTO delegation_graph VALUES (?1,?2,?2,'grokbot','grokbot',1,?3,?4,?5,?6)",
                params![
                    edge,
                    parent,
                    id,
                    conversation,
                    r#"{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}"#,
                    r#"{"status":"running"}"#
                ],
            )
            .unwrap();
    }

    fn clock(&self, elapsed: u64) {
        self.provider().watchdog_test_clock(self.start + elapsed);
    }

    fn tick(&mut self, elapsed: u64) {
        self.clock(elapsed);
        self.app.tick(self.start + elapsed);
    }

    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }

    fn ping(&mut self) -> Value {
        self.app.api("GET", "/api/ping", b"").unwrap()["revision"].clone()
    }

    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }

    fn messages(&mut self, kind: &str) -> Vec<Value> {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|conversation| conversation["messages"].as_array().unwrap())
            .filter(|message| message["kind"] == kind)
            .cloned()
            .collect()
    }

    fn event_count(&self, id: &str, kind: RunEventKind) -> usize {
        store::events(&self.conn, id, 0)
            .unwrap()
            .iter()
            .filter(|event| event.kind == kind)
            .count()
    }

    fn notification_count(&self, id: &str) -> u32 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM wrapper_notifications WHERE id=?1",
                [format!("stalled:{id}")],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn restart(&mut self, elapsed: u64) {
        self.app = App::open(&self.dir.join("workspace.sqlite")).unwrap();
        self.clock(elapsed);
    }
}

struct CompletedGateway;

impl ModelGateway for CompletedGateway {
    fn stream(
        &self,
        _: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()> {
        emit(ModelEvent::Completed)
    }
}

struct BlockedServices {
    provider: Provider,
    path: PathBuf,
    internal: bool,
    cancel: Arc<AtomicBool>,
    started: Sender<()>,
    stopped: Sender<()>,
    release: Receiver<()>,
    calls: Arc<AtomicUsize>,
}

impl BlockedServices {
    fn blocked(&mut self) -> ToolOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        while !self.cancel.load(Ordering::SeqCst) {
            match self.release.recv_timeout(Duration::from_millis(10)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return ToolOutcome::Done {
                        output: json!({"late-output":true}),
                    };
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        self.stopped.send(()).unwrap();
        if !self.internal {
            let _ = self.release.recv();
        }
        ToolOutcome::Done {
            output: json!({"late-output":true}),
        }
    }
}

impl RunServices for BlockedServices {
    fn context(&mut self, _: &Run) -> Result<ModelRequest> {
        Ok(ModelRequest {
            model: "watchdog-test".into(),
            instructions: String::new(),
            input: vec![],
            tools: vec![],
            effort: None,
            service_tier: None,
            prompt_cache_key: None,
        })
    }

    fn tools(&self) -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "slow".into(),
            description: String::new(),
            schema: json!({"type":"object"}),
            requires_approval: false,
        }]
    }

    fn call_tool(&mut self, conn: &Connection, _: &Run, _: &ToolCall) -> Result<ToolOutcome> {
        assert!(self.internal);
        assert!(!conn.is_autocommit());
        conn.execute("INSERT INTO watchdog_test_effects VALUES ('rollback')", [])?;
        Ok(self.blocked())
    }

    fn is_external_tool(&self, _: &ToolCall) -> bool {
        !self.internal
    }

    fn call_external_tool(
        &mut self,
        conn: &Connection,
        _: &Run,
        _: &ToolCall,
    ) -> Result<ToolOutcome> {
        assert!(conn.is_autocommit());
        assert!(!self.internal);
        Ok(self.blocked())
    }

    fn check_approval(&mut self, _: &Connection, _: &Run, _: &ToolCall) -> Result<ApprovalCheck> {
        panic!("the test tool is not protected")
    }

    fn request_approval(&mut self, _: &Connection, _: &Run, _: &ToolCall) -> Result<SuspendCause> {
        panic!("the test tool does not request human input")
    }

    fn is_cancelled(&self, _: &str) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn on_run_event(&mut self, conn: &Connection, event: &RunEvent) -> Result<()> {
        let run = store::required(conn, &event.run_id)?;
        let ctx = Context {
            connection: conn,
            provider: &self.provider,
            db_path: &self.path,
            now_ms: 0,
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
        };
        watchdog::record(&ctx, event, &run)
            .map_err(|error| grokbot_engine::contracts::Error::InvalidInput(error.to_string()))?;
        features::on_run_event(&ctx, event);
        Ok(())
    }

    fn on_activity(&mut self, id: &str, activity: RunActivity<'_>) {
        self.provider.watchdog_test_activity(id, activity);
    }
}

struct BlockedWorker {
    release: Option<Sender<()>>,
    stopped: Receiver<()>,
    join: Option<JoinHandle<Result<RunProgress>>>,
    calls: Arc<AtomicUsize>,
}

impl BlockedWorker {
    fn start(fixture: &Fixture, id: &str) -> Self {
        Self::start_tool(fixture, id, false)
    }

    fn start_tool(fixture: &Fixture, id: &str, internal: bool) -> Self {
        let provider = fixture.provider();
        let lease = provider.watchdog_test_claim_run(id);
        let path = fixture.dir.join("workspace.sqlite");
        let (started_tx, started_rx) = mpsc::channel();
        let (stopped_tx, stopped_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let id = id.to_owned();
        let join = thread::spawn(move || {
            let conn = Connection::open(path).unwrap();
            conn.busy_timeout(Duration::from_secs(3)).unwrap();
            let path = PathBuf::from(conn.path().unwrap());
            let mut services = BlockedServices {
                provider,
                path,
                internal,
                cancel: lease.cancel.clone(),
                started: started_tx,
                stopped: stopped_tx,
                release: release_rx,
                calls: worker_calls,
            };
            let progress = run_loop::run(&conn, &CompletedGateway, &mut services, &id);
            drop(lease);
            progress
        });
        let worker = Self {
            release: Some(release_tx),
            stopped: stopped_rx,
            join: Some(join),
            calls,
        };
        started_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("the real engine dispatch must enter the blocked tool");
        worker
    }

    fn finish(mut self) -> RunProgress {
        let _ = self.release.take().unwrap().send(());
        self.join.take().unwrap().join().unwrap().unwrap()
    }
}

impl Drop for BlockedWorker {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[test]
fn stall_soft_flag_after_three_minutes() {
    let mut f = Fixture::new();
    f.run("slow-run", RunStatus::Running);
    f.tick(0);
    let before = f.state();
    let before_ping = f.ping();
    assert_eq!(before["silence"], json!({}));
    assert_eq!(before["state_revision"], before_ping);
    f.tick(240_000);
    let silent = f.state();
    assert_eq!(silent["silence"]["slow-run"], f.start);
    assert_ne!(silent["state_revision"], before["state_revision"]);
    assert_ne!(f.ping(), before_ping);
    assert_eq!(
        store::required(&f.conn, "slow-run").unwrap().status,
        RunStatus::Running
    );
    f.clock(241_000);
    f.provider().pulse_run("slow-run");
    let active = f.state();
    assert_eq!(active["silence"], json!({}));
    assert_ne!(active["state_revision"], silent["state_revision"]);
    assert_eq!(f.ping(), active["state_revision"]);
    assert_eq!(f.notification_count("slow-run"), 0);
}

#[test]
fn stall_hard_stop_after_eight_minutes() {
    let mut f = Fixture::new();
    f.run("slow-run", RunStatus::Running);
    f.tick(0);
    let worker = BlockedWorker::start(&f, "slow-run");
    f.tick(240_000);
    assert_eq!(f.state()["silence"]["slow-run"], f.start);
    f.tick(540_000);
    worker
        .stopped
        .recv_timeout(Duration::from_secs(3))
        .expect("the watchdog must signal the blocked tool through its run cancellation token");
    f.tick(555_000);
    let cancelled = store::required(&f.conn, "slow-run").unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert_eq!(cancelled.turn_end, Some(TurnEnd::Stopped));
    let stalled = f.messages("stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(
        stalled[0]["body"],
        "Arrêté : rien ne bougeait depuis 8 min (dernière étape : slow)."
    );
    assert_eq!(stalled[0]["payload"]["run_id"], "slow-run");
    assert_eq!(stalled[0]["payload"]["conversation_id"], f.conversation);
    assert_eq!(stalled[0]["payload"]["message_id"], "slow-run");
    assert_eq!(f.notification_count("slow-run"), 1);
    assert_eq!(f.event_count("slow-run", RunEventKind::Cancelled), 1);
    let before_late = store::events(&f.conn, "slow-run", 0).unwrap();
    let calls = worker.calls.clone();
    assert_eq!(worker.finish(), RunProgress::Ended(TurnEnd::Stopped));
    f.tick(600_000);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(store::events(&f.conn, "slow-run", 0).unwrap(), before_late);
    assert_eq!(f.messages("stalled"), stalled);
    assert_eq!(f.notification_count("slow-run"), 1);
    assert_eq!(f.state()["silence"], json!({}));
    f.restart(600_000);
    assert_eq!(f.notification_count("slow-run"), 1);
    assert_eq!(f.messages("stalled"), stalled);
}

#[test]
fn stall_child_running_silent_is_stopped() {
    let mut f = Fixture::new();
    f.run("parent", RunStatus::AwaitingChildren);
    f.child("parent", "child", RunStatus::Running);
    f.child("parent", "sibling", RunStatus::Running);
    f.run("unrelated", RunStatus::Running);
    f.tick(0);
    f.clock(540_000);
    f.provider().pulse_run("sibling");
    f.provider().pulse_run("unrelated");
    f.tick(540_000);
    assert_eq!(
        store::required(&f.conn, "child").unwrap().status,
        RunStatus::Cancelled
    );
    assert_ne!(
        store::required(&f.conn, "parent").unwrap().status,
        RunStatus::Cancelled
    );
    for id in ["sibling", "unrelated"] {
        assert_eq!(
            store::required(&f.conn, id).unwrap().status,
            RunStatus::Running
        );
        assert_eq!(f.notification_count(id), 0);
    }
    let edge: String = f
        .conn
        .query_row(
            "SELECT status FROM delegation_graph WHERE id='edge-child'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&edge).unwrap()["status"],
        "failed"
    );
    let wakes: u32 = f.conn.query_row(
        "SELECT COUNT(*) FROM engine_run_wakes WHERE run_id='parent' AND cause='child' AND reference='edge-child' AND state='pending'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(wakes, 1);
    assert_eq!(f.notification_count("child"), 1);
    f.tick(555_000);
    assert_eq!(f.event_count("child", RunEventKind::Cancelled), 1);
}

#[test]
fn internal_tool_holding_the_writer_is_cancelled_and_its_effect_rolls_back() {
    let mut f = Fixture::new();
    f.conn
        .execute(
            "CREATE TABLE watchdog_test_effects(id TEXT PRIMARY KEY)",
            [],
        )
        .unwrap();
    f.run("internal", RunStatus::Running);
    f.tick(0);
    let worker = BlockedWorker::start_tool(&f, "internal", true);
    f.conn.busy_timeout(Duration::ZERO).unwrap();
    let writer = f.conn.execute_batch("BEGIN IMMEDIATE");
    f.conn.busy_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        matches!(writer, Err(rusqlite::Error::SqliteFailure(error, _))
        if error.code == rusqlite::ErrorCode::DatabaseBusy)
    );
    let started = Instant::now();
    f.tick(540_000);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "watchdog waited for its own writer timeout"
    );
    worker
        .stopped
        .recv_timeout(Duration::from_secs(3))
        .expect("the cancellation token must release the cooperative internal tool");
    let calls = worker.calls.clone();
    assert_eq!(worker.finish(), RunProgress::Ended(TurnEnd::Stopped));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let effects: u32 = f
        .conn
        .query_row("SELECT COUNT(*) FROM watchdog_test_effects", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(effects, 0);
    assert_eq!(
        store::required(&f.conn, "internal").unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(f.event_count("internal", RunEventKind::Cancelled), 1);
    let stalled = f.messages("stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(
        stalled[0]["body"],
        "Arrêté : rien ne bougeait depuis 8 min (dernière étape : slow)."
    );
    assert_eq!(f.notification_count("internal"), 1);
}

#[test]
fn human_approval_and_interrupted_runs_remain_waiting_after_twenty_minutes() {
    let mut f = Fixture::new();
    let waiting = [
        ("approval", RunStatus::AwaitingApproval),
        ("human", RunStatus::AwaitingHumanInput),
        ("interrupted", RunStatus::Interrupted),
        ("children", RunStatus::AwaitingChildren),
    ];
    for (id, status) in waiting {
        f.run(id, status);
    }
    f.tick(0);
    f.tick(1_200_000);
    assert_eq!(f.state()["silence"], json!({}));
    assert!(f.messages("stalled").is_empty());
    for (id, status) in waiting {
        assert_eq!(store::required(&f.conn, id).unwrap().status, status);
        assert_eq!(f.event_count(id, RunEventKind::Cancelled), 0);
        assert_eq!(f.notification_count(id), 0);
    }
}

#[test]
fn parent_with_running_edge_to_human_child_is_excluded() {
    for status in [RunStatus::AwaitingApproval, RunStatus::AwaitingHumanInput] {
        let mut f = Fixture::new();
        f.run("parent", RunStatus::AwaitingChildren);
        f.child("parent", "human", status);
        f.tick(0);
        f.tick(1_200_000);
        assert_eq!(f.state()["silence"], json!({}));
        assert_eq!(
            store::required(&f.conn, "parent").unwrap().status,
            RunStatus::AwaitingChildren
        );
        assert_eq!(store::required(&f.conn, "human").unwrap().status, status);
        assert!(f.messages("stalled").is_empty());
    }
}

#[test]
fn human_branch_does_not_protect_a_silent_running_sibling() {
    let mut f = Fixture::new();
    f.run("parent", RunStatus::AwaitingChildren);
    f.child("parent", "human", RunStatus::AwaitingHumanInput);
    f.child("parent", "active", RunStatus::Running);
    f.tick(0);
    f.tick(240_000);
    assert!(f.state()["silence"]["parent"].is_null());
    assert_eq!(f.state()["silence"]["active"], f.start);
    f.tick(1_200_000);
    assert_eq!(
        store::required(&f.conn, "parent").unwrap().status,
        RunStatus::AwaitingChildren
    );
    assert_eq!(
        store::required(&f.conn, "human").unwrap().status,
        RunStatus::AwaitingHumanInput
    );
    assert_eq!(
        store::required(&f.conn, "active").unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(f.notification_count("human"), 0);
    assert_eq!(f.notification_count("active"), 1);
}

#[test]
fn pulses_every_ten_seconds_keep_a_run_alive_for_twelve_simulated_minutes() {
    let mut f = Fixture::new();
    f.run("pulsing", RunStatus::Running);
    f.tick(0);
    for elapsed in (10_000..=720_000).step_by(10_000) {
        f.clock(elapsed);
        f.provider().pulse_run("pulsing");
        f.tick(elapsed);
        assert_eq!(f.state()["silence"], json!({}));
        assert_eq!(
            store::required(&f.conn, "pulsing").unwrap().status,
            RunStatus::Running
        );
    }
    assert!(f.messages("stalled").is_empty());
    assert_eq!(f.notification_count("pulsing"), 0);
    assert_eq!(f.event_count("pulsing", RunEventKind::Cancelled), 0);
}

#[test]
fn each_live_activity_clears_silence_without_waiting_for_a_scan() {
    for activity in [
        RunActivity::Model,
        RunActivity::ToolStarted("slow"),
        RunActivity::ToolFinished("slow"),
    ] {
        let mut f = Fixture::new();
        f.run("active", RunStatus::Running);
        f.tick(0);
        f.tick(240_000);
        assert_eq!(f.state()["silence"]["active"], f.start);
        let before = f.ping();
        f.clock(241_000);
        f.provider().watchdog_test_activity("active", activity);
        assert_eq!(f.state()["silence"], json!({}));
        assert_ne!(f.ping(), before);
        f.tick(421_000);
        assert_eq!(f.state()["silence"]["active"], f.start + 241_000);
    }
}

#[test]
fn rolled_back_events_are_not_a_committed_sign_of_life() {
    let mut f = Fixture::new();
    f.run("active", RunStatus::Running);
    f.tick(0);
    {
        let tx = f.conn.unchecked_transaction().unwrap();
        store::append_event(
            &tx,
            "active",
            RunEventKind::TextDelta,
            &json!({"text":"rolled back"}),
        )
        .unwrap();
    }
    f.tick(240_000);
    assert_eq!(f.state()["silence"]["active"], f.start);
    with_tx(&f.conn, |conn| {
        store::append_event(
            conn,
            "active",
            RunEventKind::TextDelta,
            &json!({"text":"committed"}),
        )
    })
    .unwrap();
    f.tick(255_000);
    assert_eq!(f.state()["silence"], json!({}));
    f.tick(435_000);
    assert_eq!(f.state()["silence"]["active"], f.start + 255_000);
}

#[test]
fn fresh_registry_seeds_resumed_runs_and_evicts_terminal_runs() {
    let mut f = Fixture::new();
    f.run("resumed", RunStatus::Running);
    f.tick(0);
    f.tick(240_000);
    assert_eq!(f.state()["silence"]["resumed"], f.start);
    f.restart(1_200_000);
    assert_eq!(
        store::required(&f.conn, "resumed").unwrap().status,
        RunStatus::Interrupted
    );
    with_tx(&f.conn, |conn| {
        model::resume_at(conn, "resumed", (f.start + 1_200_000) as i64)
    })
    .unwrap();
    f.tick(1_200_000);
    assert_eq!(f.state()["silence"], json!({}));
    f.tick(1_440_000);
    assert_eq!(f.state()["silence"]["resumed"], f.start + 1_200_000);
    with_tx(&f.conn, |conn| {
        model::cancel(conn, "resumed", TurnEnd::Stopped)
    })
    .unwrap();
    f.tick(1_455_000);
    let after_terminal = f.ping();
    assert_eq!(f.state()["silence"], json!({}));
    f.clock(1_460_000);
    f.provider().pulse_run("resumed");
    assert_eq!(f.ping(), after_terminal);
    assert_eq!(f.state()["silence"], json!({}));
    assert_eq!(f.notification_count("resumed"), 0);
}

#[test]
fn activity_and_clocks_are_isolated_between_provider_instances() {
    let mut first = Fixture::new();
    let mut second = Fixture::new();
    first.run("same-id", RunStatus::Running);
    second.run("same-id", RunStatus::Running);
    first.tick(0);
    second.tick(0);
    first.tick(240_000);
    second.tick(240_000);
    assert_eq!(first.state()["silence"]["same-id"], first.start);
    assert_eq!(second.state()["silence"]["same-id"], second.start);
    first.clock(241_000);
    first.provider().pulse_run("same-id");
    assert_eq!(first.state()["silence"], json!({}));
    assert_eq!(second.state()["silence"]["same-id"], second.start);
    second.tick(540_000);
    assert_eq!(
        store::required(&second.conn, "same-id").unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(
        store::required(&first.conn, "same-id").unwrap().status,
        RunStatus::Running
    );
}

#[test]
fn stalled_notification_obeys_existing_preferences_dnd_and_focus() {
    for (enabled, dnd, reply, focused, visible, expected) in [
        (true, false, true, false, false, 1),
        (false, false, true, false, false, 0),
        (true, true, true, false, false, 0),
        (true, false, false, false, false, 0),
        (true, false, true, true, true, 0),
        (true, false, true, true, false, 1),
    ] {
        let mut f = Fixture::new();
        let settings = json!({"enabled":enabled,"dnd":dnd,"reply":reply,"approval":true,
            "form":true,"secret":true,"routine":true,"delegation":true});
        f.post("/api/notifications/settings", settings.clone());
        let preferences = f.state()["preferences"].clone();
        f.run("slow-run", RunStatus::Running);
        f.tick(0);
        f.tick(540_000);
        assert_eq!(f.notification_count("slow-run"), 1);
        let poll = f.post(
            "/api/notifications/poll",
            json!({
                "focused":focused,"visible":visible,"conversation_id":f.conversation,
            }),
        );
        assert_eq!(poll["settings"], settings);
        assert_eq!(poll["deliveries"].as_array().unwrap().len(), expected);
        if expected == 1 {
            assert_eq!(poll["deliveries"][0]["id"], "stalled:slow-run");
            assert_eq!(
                poll["deliveries"][0]["title"],
                "Tâche arrêtée après un silence prolongé"
            );
        }
        assert_eq!(f.state()["preferences"], preferences);
        let repeated = f.post(
            "/api/notifications/poll",
            json!({
                "focused":false,"visible":false,"conversation_id":f.conversation,
            }),
        );
        assert_eq!(repeated["deliveries"], json!([]));
        assert_eq!(f.notification_count("slow-run"), 1);
    }
}

#[test]
fn last_public_step_does_not_expose_untrusted_tool_content() {
    let mut f = Fixture::new();
    f.run("slow-run", RunStatus::Running);
    f.tick(0);
    f.provider().watchdog_test_activity(
        "slow-run",
        RunActivity::ToolStarted("slow/private?secret=NE_PAS_EXPOSER"),
    );
    f.tick(540_000);
    let stalled = f.messages("stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(
        stalled[0]["body"],
        "Arrêté : rien ne bougeait depuis 8 min (dernière étape : outil)."
    );
    assert!(!serde_json::to_string(&stalled)
        .unwrap()
        .contains("NE_PAS_EXPOSER"));
}

#[test]
fn last_event_lookup_is_indexed_and_bounded_with_large_event_history() {
    let mut f = Fixture::new();
    f.run("history", RunStatus::Running);
    let measure = |conn: &Connection| {
        let mut statement = conn.prepare(LATEST_SEQ_SQL).unwrap();
        let seq: u64 = statement.query_row(["history"], |row| row.get(0)).unwrap();
        (seq, statement.get_status(StatementStatus::VmStep))
    };
    let (before_seq, small) = measure(&f.conn);
    f.conn.execute(
        "WITH RECURSIVE n(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM n WHERE value<20000)
         INSERT INTO engine_run_events(run_id,seq,kind,payload)
         SELECT 'history',?1+value,'text_delta','{\"text\":\"historical\"}' FROM n",
        [before_seq],
    ).unwrap();
    let (after_seq, large) = measure(&f.conn);
    assert_eq!(after_seq, before_seq + 20_000);
    println!("latest-seq VmStep small={small} large={large} rows_added=20000");
    assert!(small > 0);
    assert!(
        large <= small + 4,
        "last-event lookup used {small} VM steps before and {large} after 20000 events"
    );
    assert!(
        large <= 32,
        "last-event lookup must visit the indexed tail, used {large} VM steps"
    );
    let mut explain = f
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {LATEST_SEQ_SQL}"))
        .unwrap();
    let plan = explain
        .query_map(["history"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    println!("latest-seq plan={plan}");
    assert!(plan.contains("SEARCH engine_run_events"), "{plan}");
    assert!(plan.contains("run_id=?"), "{plan}");
    assert!(!plan.contains("SCAN "), "{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    drop(explain);
    f.tick(0);
    let before = f.state();
    for _ in 0..3 {
        let state = f.state();
        assert_eq!(state["silence"], json!({}));
        assert_eq!(state["state_revision"], before["state_revision"]);
    }
}

#[test]
fn stalled_entry_lookup_is_bounded_with_ten_times_more_old_stalls() {
    let mut f = Fixture::new();
    f.run("latest", RunStatus::Running);
    f.tick(0);
    f.tick(540_000);
    let expected = f.messages("stalled");
    assert_eq!(expected.len(), 1);
    let measure = |conn: &Connection| {
        let mut statement = conn.prepare(STALLED_ENTRY_SQL).unwrap();
        let entry: (String, u64) = statement
            .query_row(["latest"], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        (entry, statement.get_status(StatementStatus::VmStep))
    };
    let seed = |conn: &Connection, first: u32, last: u32| {
        let tx = conn.unchecked_transaction().unwrap();
        for index in first..=last {
            let id = format!("old-{index}");
            tx.execute(
                "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record)
                 SELECT ?1,bot_id,conversation_id,trigger_kind,conversation_id||'/'||?1,0,status,
                    json_set(record,'$.run_id',?1,'$.trigger.message_id',conversation_id||'/'||?1)
                 FROM engine_runs WHERE run_id='latest'",
                [&id],
            ).unwrap();
            tx.execute(
                "INSERT INTO wrapper_run_stalls VALUES (?1,'outil',1)",
                [&id],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    };
    seed(&f.conn, 1, 100);
    let (small_entry, small) = measure(&f.conn);
    seed(&f.conn, 101, 1_000);
    let (large_entry, large) = measure(&f.conn);
    assert_eq!(small_entry, large_entry);
    println!("stalled-entry VmStep small={small} large={large} old_rows=100/1000");
    assert!(small > 0);
    assert!(
        large <= small + 4,
        "stalled-entry lookup used {small} VM steps with 100 old stalls and {large} with 1000"
    );
    assert!(large <= 32, "one stalled entry used {large} VM steps");
    let mut statement = f
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {STALLED_ENTRY_SQL}"))
        .unwrap();
    let plan = statement
        .query_map(["latest"], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    println!("stalled-entry plan={plan}");
    assert!(plan.contains("SEARCH wrapper_run_stalls"), "{plan}");
    assert!(plan.contains("run_id=?"), "{plan}");
    assert!(!plan.contains("SCAN "), "{plan}");
    drop(statement);
    assert_eq!(f.messages("stalled"), expected);
}

#[test]
fn stalled_chat_regenerates_the_real_source_message_through_the_existing_route() {
    let mut f = Fixture::new();
    f.post(
        "/api/send",
        json!({
            "conversation_id":f.conversation,"message_id":"source","body":"Question source",
        }),
    );
    let trigger = Trigger::UserMessage {
        message_id: format!("{}/source", f.conversation),
        attempt: 0,
    };
    let original = store::latest_run_for_trigger(&f.conn, &trigger)
        .unwrap()
        .unwrap();
    let Trigger::UserMessage { attempt, .. } = original.trigger else {
        unreachable!()
    };
    f.insert_run(
        "stalled-source",
        &f.conversation,
        Trigger::UserMessage {
            message_id: format!("{}/source", f.conversation),
            attempt: attempt + 1,
        },
        RunStatus::Running,
    );
    f.tick(0);
    f.tick(540_000);
    f.conn
        .execute(
            "INSERT INTO wrapper_engine_runs(run_id,input_json,live,model)
         SELECT 'stalled-source',input_json,0,model FROM wrapper_engine_runs WHERE run_id=?1",
            [&original.run_id],
        )
        .unwrap();
    let stalled = f.messages("stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(stalled[0]["payload"]["message_id"], "source");
    assert_eq!(stalled[0]["payload"]["can_regenerate"], true);
    f.post(
        "/api/regenerate",
        json!({
            "conversation_id":stalled[0]["payload"]["conversation_id"],
            "message_id":stalled[0]["payload"]["message_id"],
        }),
    );
    let regenerated = store::latest_run_for_trigger(&f.conn, &trigger)
        .unwrap()
        .unwrap();
    assert_ne!(regenerated.run_id, "stalled-source");
    assert_eq!(
        regenerated.trigger,
        Trigger::UserMessage {
            message_id: format!("{}/source", f.conversation),
            attempt: attempt + 2,
        }
    );
    let ends: Vec<_> = store::events(&f.conn, "stalled-source", 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == RunEventKind::Cancelled)
        .map(|event| event.payload["turn_end"].clone())
        .collect();
    assert_eq!(ends, vec![json!("stopped"), json!("superseded")]);
    assert_eq!(f.notification_count("stalled-source"), 1);
    assert_eq!(f.messages("stalled"), stalled);
}

#[test]
fn thresholds_are_inclusive_and_the_scan_does_not_refresh_activity() {
    let mut f = Fixture::new();
    f.provider().watchdog_test_policy(watchdog::Policy {
        soft_ms: 180_000,
        hard_ms: 480_000,
        scan_ms: 1,
    });
    f.run("boundary", RunStatus::Running);
    f.tick(0);
    f.tick(179_999);
    assert_eq!(f.state()["silence"], json!({}));
    f.tick(180_000);
    assert_eq!(f.state()["silence"]["boundary"], f.start);
    f.tick(479_999);
    assert_eq!(
        store::required(&f.conn, "boundary").unwrap().status,
        RunStatus::Running
    );
    f.tick(480_000);
    assert_eq!(
        store::required(&f.conn, "boundary").unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(f.event_count("boundary", RunEventKind::Cancelled), 1);
}

#[test]
fn a_background_command_keeps_its_own_card_without_a_run_stall() {
    let mut f = Fixture::new();
    f.run("background", RunStatus::Running);
    f.conn.execute("INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('background','shell','en cours')",[]).unwrap();
    f.tick(0);
    f.tick(1_200_000);
    assert_eq!(f.state()["silence"], json!({}));
    assert_eq!(
        store::required(&f.conn, "background").unwrap().status,
        RunStatus::Running
    );
    assert_eq!(f.notification_count("background"), 0);
    assert!(f.messages("stalled").is_empty());
}

#[test]
fn a_running_parent_cannot_cancel_a_human_branch_or_a_live_sibling() {
    let mut f = Fixture::new();
    f.run("parent", RunStatus::Running);
    f.child("parent", "human", RunStatus::AwaitingApproval);
    f.child("parent", "active", RunStatus::Running);
    f.tick(0);
    f.tick(540_000);
    assert_eq!(
        store::required(&f.conn, "parent").unwrap().status,
        RunStatus::Running
    );
    assert_eq!(
        store::required(&f.conn, "human").unwrap().status,
        RunStatus::AwaitingApproval
    );
    assert_eq!(
        store::required(&f.conn, "active").unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(f.notification_count("parent"), 0);
    assert_eq!(f.notification_count("human"), 0);
    assert_eq!(f.notification_count("active"), 1);
}

#[test]
fn a_child_pulse_removes_its_waiting_parents_signal_immediately() {
    let mut f = Fixture::new();
    f.run("parent", RunStatus::AwaitingChildren);
    f.child("parent", "child", RunStatus::Running);
    f.tick(0);
    f.tick(240_000);
    assert_eq!(f.state()["silence"]["parent"], f.start);
    assert_eq!(f.state()["silence"]["child"], f.start);
    f.clock(241_000);
    f.provider().pulse_run("child");
    assert_eq!(f.state()["silence"], json!({}));
}

#[test]
fn stalled_group_turns_do_not_offer_a_regeneration_the_route_refuses() {
    let mut f = Fixture::new();
    f.conn
        .execute(
            "UPDATE wrapper_conversations SET kind='group' WHERE id=?1",
            [&f.conversation],
        )
        .unwrap();
    f.run("group-source", RunStatus::Running);
    f.tick(0);
    f.tick(540_000);
    let stalled = f.messages("stalled");
    assert_eq!(stalled.len(), 1);
    assert_eq!(stalled[0]["payload"]["message_id"], "group-source");
    assert_eq!(stalled[0]["payload"]["can_regenerate"], false);
    let error = f
        .app
        .api(
            "POST",
            "/api/regenerate",
            json!({"conversation_id":f.conversation,"message_id":"group-source"})
                .to_string()
                .as_bytes(),
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
}

fn stalled_notice_settings(f: &mut Fixture, kind: &str, enabled: bool, reply: bool) {
    let mut settings = f.app.api("GET", "/api/notifications", b"").unwrap()["settings"].clone();
    settings["enabled"] = json!(true);
    settings["reply"] = json!(reply);
    settings[kind] = json!(enabled);
    f.post("/api/notifications/settings", settings);
}

fn assert_one_stalled_notice(f: &mut Fixture, id: &str, kind: &str, run: &str, enabled: bool) {
    let input = json!({"focused":false,"visible":false,"conversation_id":null});
    let polled = f.post("/api/notifications/poll", input.clone());
    assert_eq!(
        polled["deliveries"].as_array().unwrap().len(),
        usize::from(enabled)
    );
    if enabled {
        assert_eq!(polled["deliveries"][0]["id"], id);
        assert_eq!(polled["deliveries"][0]["kind"], kind);
        assert_eq!(polled["deliveries"][0]["card_id"], format!("stalled:{run}"));
    }
    let inbox = f.app.api("GET", "/api/notifications", b"").unwrap();
    assert_eq!(inbox["items"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["unread"], 1);
    assert_eq!(inbox["items"][0]["id"], id);
    assert_eq!(inbox["items"][0]["kind"], kind);
    assert_eq!(inbox["items"][0]["card_id"], format!("stalled:{run}"));
    let count: u32 = f
        .conn
        .query_row("SELECT COUNT(*) FROM wrapper_notifications", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.post("/api/notifications/poll", input.clone())["deliveries"],
        json!([])
    );
    f.restart(540_000);
    assert_eq!(
        f.post("/api/notifications/poll", input)["deliveries"],
        json!([])
    );
    let reopened = f.app.api("GET", "/api/notifications", b"").unwrap();
    assert_eq!(reopened["items"], inbox["items"]);
    assert_eq!(reopened["unread"], 1);
}

#[test]
fn stalled_occurrence_has_one_notice_and_obeys_routine_independently_of_reply() {
    use grokbot_scheduler::occurrences::{self, OccurrenceStatus};
    for routine_enabled in [false, true] {
        for reply in [false, true] {
            let mut f = Fixture::new();
            stalled_notice_settings(&mut f, "routine", routine_enabled, reply);
            let routine = f.post(
                "/api/routines",
                json!({
                    "bot_id":"grokbot","name":"Silent routine","prompt":"Wait",
                    "schedule":"@every 1h","time_zone":"UTC","enabled":false
                }),
            )["routine"]
                .clone();
            let occurrence = occurrences::run_now(
                &f.conn,
                routine["routine_id"].as_str().unwrap(),
                "watchdog",
                f.start as i64,
            )
            .unwrap();
            f.insert_run(
                "routine-run",
                routine["conversation_id"].as_str().unwrap(),
                Trigger::Occurrence {
                    occurrence_id: occurrence.occurrence_id.clone(),
                },
                RunStatus::Running,
            );
            occurrences::attach_run(&f.conn, &occurrence.occurrence_id, "routine-run").unwrap();
            assert!(matches!(
                occurrences::get(&f.conn, &occurrence.occurrence_id).unwrap().unwrap().status,
                OccurrenceStatus::Running { run_id } if run_id == "routine-run"
            ));
            assert_eq!(
                store::required(&f.conn, "routine-run").unwrap().status,
                RunStatus::Running
            );
            f.tick(0);
            f.tick(540_000);
            let run = store::required(&f.conn, "routine-run").unwrap();
            assert_eq!(run.status, RunStatus::Cancelled);
            assert_eq!(run.turn_end, Some(TurnEnd::Stopped));
            assert_eq!(f.event_count("routine-run", RunEventKind::Cancelled), 1);
            assert!(matches!(
                occurrences::get(&f.conn, &occurrence.occurrence_id)
                    .unwrap()
                    .unwrap()
                    .status,
                OccurrenceStatus::Failed { .. }
            ));
            assert_one_stalled_notice(
                &mut f,
                &format!("routine:{}", occurrence.occurrence_id),
                "routine",
                "routine-run",
                routine_enabled,
            );
        }
    }
}

#[test]
fn stalled_child_notice_obeys_delegation_independently_of_reply_and_preserves_failure_wake() {
    for delegation_enabled in [false, true] {
        for reply in [false, true] {
            let mut f = Fixture::new();
            stalled_notice_settings(&mut f, "delegation", delegation_enabled, reply);
            f.run("parent", RunStatus::AwaitingChildren);
            f.child("parent", "child", RunStatus::Running);
            f.tick(0);
            f.tick(540_000);
            let child = store::required(&f.conn, "child").unwrap();
            assert_eq!(child.status, RunStatus::Cancelled);
            assert_eq!(child.turn_end, Some(TurnEnd::Stopped));
            assert_eq!(f.event_count("child", RunEventKind::Cancelled), 1);
            assert_eq!(
                store::required(&f.conn, "parent").unwrap().status,
                RunStatus::AwaitingChildren
            );
            let edge: String = f.conn.query_row(
                "SELECT json_extract(status,'$.status') FROM delegation_graph WHERE id='edge-child'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(edge, "failed");
            let wakes: u32 = f.conn.query_row(
                "SELECT COUNT(*) FROM engine_run_wakes WHERE run_id='parent' AND cause='child' AND reference='edge-child' AND state='pending'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(wakes, 1);
            assert_one_stalled_notice(
                &mut f,
                "stalled:child",
                "delegation",
                "child",
                delegation_enabled,
            );
        }
    }
}
