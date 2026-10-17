//! Routine adapter tests: injected host clock, real scheduler/engine uniqueness,
//! and conversation confirmation (Q5 / REA cron-detail.md CR19), not approval cards.
mod support;
use grokbot_engine::{contracts::*, model::Run, store};
use grokbot_scheduler::{
    occurrences::{self, CatchUpPolicy, OccurrenceStatus},
    routine::{self, Routine},
};
use grokbot_wrapper::{features, App, Provider};
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
    path: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new(live: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "grokbot-routines-{}-{}",
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
        Self {
            app,
            path,
            server,
            provider,
        }
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn post(&mut self, path: &str, v: Value) -> Value {
        self.app
            .api("POST", path, v.to_string().as_bytes())
            .unwrap()
    }
    fn create(&mut self, prompt: &str) -> Routine {
        serde_json::from_value(
            self.post(
                "/api/routines",
                json!({"bot_id":"grokbot","name":"Report","prompt":prompt,"schedule":"@every 1m"}),
            )["routine"]
                .clone(),
        )
        .unwrap()
    }
    fn tick_due(&mut self, r: &Routine) {
        self.app.tick((r.modified_at + 60_000) as u64);
    }
    fn history(&mut self, r: &Routine) -> Value {
        self.post(
            "/api/routines/history",
            json!({"bot_id":r.bot_id,"routine_id":r.routine_id}),
        )
    }
    fn run(&self, r: &Routine) -> Run {
        let record:String=self.db().query_row("SELECT record FROM engine_runs WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT 1",[&r.conversation_id],|row|row.get(0)).unwrap();
        serde_json::from_str(&record).unwrap()
    }
    fn count(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT count(*) FROM engine_runs WHERE trigger_kind='occurrence'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn events(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn reopen(&mut self) {
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
    }
    fn wait_run(&self, r: &Routine) {
        wait("routine finished", || self.run(r).status.is_terminal());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":args.to_string()}})
}
fn complete() -> Value {
    json!({"type":"response.completed"})
}
fn output(outcome: ToolOutcome) -> Value {
    match outcome {
        ToolOutcome::Done { output } => output,
        other => panic!("{other:?}"),
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn due_occurrence_starts_exactly_one_run() {
    let mut f = Fixture::new(false);
    let r = f.create("Write the daily report");
    f.app.tick((r.modified_at + 59_999) as u64);
    assert_eq!(f.count(), 0);
    f.tick_due(&r);
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    assert_eq!(f.run(&r).kind, RunKind::Task);
    let h = f.history(&r);
    assert_eq!(h["occurrences"].as_array().unwrap().len(), 1);
    // Simulated text ends a turn but is not an explicit successful task.
    assert_eq!(h["occurrences"][0]["status"]["status"], "failed");
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let c = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == r.conversation_id)
        .unwrap();
    assert_eq!(c["kind"], "routine");
    assert!(!c["messages"].as_array().unwrap().is_empty());
}

#[test]
fn restart_after_crash_does_not_duplicate_run() {
    let mut f = Fixture::new(false);
    let r = f.create("Crash-safe report");
    let db = f.db();
    let tx = db.unchecked_transaction().unwrap();
    let o = occurrences::materialize_due(&tx, r.modified_at + 60_000, &CatchUpPolicy::default())
        .unwrap()
        .remove(0);
    tx.commit().unwrap();
    f.reopen(); // crash after materializing the outbox, before creating the run
    f.tick_due(&r);
    let first = f.run(&r);
    assert_eq!(f.count(), 1);
    // Crash after engine creation but before scheduler attachment: reset only the
    // attachment to simulate the boundary, keeping the real durable run/events.
    db.execute("UPDATE scheduler_occurrences SET status='pending',run_id=NULL,reason=NULL WHERE occurrence_id=?1",[&o.occurrence_id]).unwrap();
    f.reopen();
    f.app.tick((r.modified_at + 61 * 60_000) as u64); // old attachment exceeds late window
    assert_eq!(
        store::latest_run_for_trigger(
            &db,
            &Trigger::Occurrence {
                occurrence_id: o.occurrence_id.clone()
            }
        )
        .unwrap()
        .unwrap()
        .run_id,
        first.run_id
    );
    let old = occurrences::get(&db, &o.occurrence_id).unwrap().unwrap();
    assert!(matches!(old.status, OccurrenceStatus::Failed { .. }));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM engine_runs WHERE trigger_ref=?1",
            [&o.occurrence_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn routine_without_skill_runs_inline_prompt() {
    let mut f = Fixture::new(true);
    let r = f.create("INLINE_ROUTINE_PAYLOAD");
    f.events(vec![
        call("done", "complete_task", json!({"summary":"Report ready"})),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.tick_due(&r);
    f.wait_run(&r);
    assert_eq!(
        f.run(&r).task_status,
        Some(TaskStatus::Done {
            summary: "Report ready".into()
        })
    );
    assert_eq!(f.history(&r)["occurrences"][0]["status"]["status"], "done");
    let seen = f.server.seen.lock().unwrap();
    assert!(seen
        .bodies
        .iter()
        .any(|b| b.to_string().contains("INLINE_ROUTINE_PAYLOAD")));
}

#[test]
fn skill_invocation_uses_pinned_version() {
    let mut f = Fixture::new(true);
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"Pinned","body":"PINNED_V1_DATA"}),
    )["skill"]
        .clone();
    let id = skill["skill_id"].as_str().unwrap();
    f.post(
        "/api/skills/update",
        json!({"bot_id":"grokbot","skill_id":id,"expected_version":1,"body":"NEW_V2_NOT_FOR_ROUTINE"}),
    );
    let r:Routine=serde_json::from_value(f.post("/api/routines",json!({"bot_id":"grokbot","name":"Pinned report","body":{"kind":"skill_invocation","skill_id":id,"version":1},"schedule":"@every 1m"}))["routine"].clone()).unwrap();
    f.events(vec![
        call(
            "done",
            "complete_task",
            json!({"summary":"Pinned report ready"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.tick_due(&r);
    f.wait_run(&r);
    let seen = f.server.seen.lock().unwrap();
    let text = seen.bodies.iter().map(Value::to_string).collect::<String>();
    assert!(text.contains("PINNED_V1_DATA"));
    // The normal skill index contains names/descriptions, never the V2 body.
    assert!(!text.contains("NEW_V2_NOT_FOR_ROUTINE"));
    assert!(text.contains("routine-skill-data"));
}

#[test]
fn model_enable_requires_approval() {
    // Historical plan test name; Q5 steering replaces a separate approval gate
    // with a real user confirmation in the principal conversation (CR19).
    let mut f = Fixture::new(false);
    let created = output(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({"name":"Model report","prompt":"Report","schedule":"@every 1m"}),
            )
            .unwrap(),
    );
    let r: Routine = serde_json::from_value(created["routine"].clone()).unwrap();
    assert!(!r.enabled);
    assert!(r.untrusted);
    assert_eq!(created["confirmation_required"], true);
    assert!(matches!(
        f.app
            .invoke_tool(
                "grokbot",
                "enable_routine",
                json!({"routine_id":r.routine_id,"confirmation_message_id":"invented"})
            )
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT count(*) FROM forms_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let chat = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert!(chat["messages"].to_string().contains("Confirmez-vous"));
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"confirm-1","body":"oui"}),
    );
    let user_trigger = Trigger::UserMessage {
        message_id: format!("{conv}/confirm-1"),
        attempt: 0,
    };
    let run = store::latest_run_for_trigger(&db, &user_trigger)
        .unwrap()
        .unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(conv);
    ctx.run_id = Some(&run.run_id);
    let tool = ToolCall {
        call_id: "enable".into(),
        name: "enable_routine".into(),
        arguments: json!({"routine_id":r.routine_id,"confirmation_message_id":"confirm-1"}),
    };
    assert!(
        output(features::routines::call_tool(&ctx, &tool).unwrap())["routine"]["enabled"] == true
    );
    tx.commit().unwrap();
    assert!(routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    assert!(
        !features::routines::tools()
            .iter()
            .find(|t| t.name == "enable_routine")
            .unwrap()
            .requires_approval
    );
}

#[test]
fn conversational_confirmation_enables_through_real_model_tool_loop() {
    let mut f = Fixture::new(true);
    let conv = f.post("/api/conversations", json!({"title":"Principal"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.events(vec![
        call(
            "create",
            "create_routine",
            json!({"name":"Confirmed live","prompt":"Report","schedule":"@every 1m"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"request","body":"Create a daily report"}),
    );
    let db = f.db();
    wait("created chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/request"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    let r = routine::list(&db, "grokbot").unwrap().remove(0);
    assert!(!r.enabled);
    f.events(vec![
        call(
            "activate",
            "enable_routine",
            json!({"routine_id":r.routine_id,"confirmation_message_id":"denied"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"denied","body":"non"}),
    );
    wait("denied chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/denied"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    assert!(!routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    f.events(vec![
        call(
            "activate",
            "enable_routine",
            json!({"routine_id":r.routine_id,"confirmation_message_id":"accepted"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"accepted","body":"oui"}),
    );
    wait("accepted chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/accepted"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    assert!(routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    assert_eq!(
        db.query_row("SELECT count(*) FROM forms_requests", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn pause_resume_manual_nonce_edit_delete_and_history() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    let id = json!({"bot_id":"grokbot","routine_id":r.routine_id});
    f.post("/api/routines/pause", id.clone());
    f.tick_due(&r);
    assert_eq!(f.count(), 0);
    let manual = json!({"bot_id":"grokbot","routine_id":r.routine_id,"nonce":"click-1"});
    let a = f.post("/api/routines/run-now", manual.clone());
    let b = f.post("/api/routines/run-now", manual);
    assert_eq!(a, b);
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    let enabled = f.post("/api/routines/resume", id.clone())["routine"].clone();
    assert_eq!(enabled["enabled"], true);
    let edited=f.post("/api/routines/update",json!({"bot_id":"grokbot","routine_id":r.routine_id,"modified_at":enabled["modified_at"],"schedule_version":enabled["schedule_version"],"name":"Renamed","schedule":"@every 2m"}))["routine"].clone();
    assert_eq!(edited["schedule_version"], 2);
    let listing = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    assert_eq!(listing["routines"][0]["name"], "Renamed");
    let history = f
        .app
        .api(
            "GET",
            &format!(
                "/api/routines/history?bot_id=grokbot&routine_id={}",
                r.routine_id
            ),
            b"",
        )
        .unwrap();
    assert!(history["missed_ranges"].is_array());
    f.post("/api/routines/delete", id);
    f.app.tick(now() + 86_400_000);
    assert_eq!(f.count(), 1);
}

#[test]
fn ownership_and_forged_fields_are_rejected() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    f.post(
        "/api/bots",
        json!({"id":"other","name":"Other","color":"citron","description":"","instructions":""}),
    );
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/routines/pause",
                json!({"bot_id":"other","routine_id":r.routine_id})
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        404
    );
    assert!(f.app.api("POST","/api/routines",json!({"bot_id":"grokbot","name":"Forged","prompt":"X","schedule":"@every 1m","modified_at":0,"origin":"user"}).to_string().as_bytes()).is_err());
    assert!(matches!(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":1,"enabled":true})).unwrap(),ToolOutcome::Failed{..}));
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"other","name":"Private","body":"private"}),
    )["skill"]
        .clone();
    assert_eq!(f.app.api("POST","/api/routines",json!({"bot_id":"grokbot","name":"Leak","schedule":"@every 1m","body":{"kind":"skill_invocation","skill_id":skill["skill_id"],"version":1}}).to_string().as_bytes()).unwrap_err().status,404);
}

#[test]
fn completion_hook_is_idempotent_and_failure_never_revives() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    f.tick_due(&r);
    let db = f.db();
    let run = f.run(&r);
    let events = store::events(&db, &run.run_id, 0).unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    for e in events.iter().rev() {
        features::routines::on_run_event(&ctx, e);
    }
    tx.commit().unwrap();
    assert_eq!(
        f.history(&r)["occurrences"][0]["status"]["status"],
        "failed"
    );
}

#[test]
fn model_update_pauses_and_invalidates_confirmation() {
    let mut f = Fixture::new(false);
    let r = f.create("Original");
    assert!(r.enabled);
    let edit=output(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"prompt":"Changed by model"})).unwrap());
    assert_eq!(edit["routine"]["enabled"], false);
    assert_eq!(edit["confirmation_required"], true);
    f.tick_due(&r);
    assert_eq!(f.count(), 0);
}

#[test]
fn paused_user_routine_can_request_fresh_conversational_confirmation() {
    let mut f = Fixture::new(false);
    let r = f.create("User report");
    f.post(
        "/api/routines/pause",
        json!({"bot_id":"grokbot","routine_id":r.routine_id}),
    );
    let requested = output(
        f.app
            .invoke_tool(
                "grokbot",
                "enable_routine",
                json!({"routine_id":r.routine_id}),
            )
            .unwrap(),
    );
    assert_eq!(requested["confirmation_required"], true);
    assert_eq!(requested["routine"]["enabled"], false);
    let before: i64 = f
        .db()
        .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |r| {
            r.get(0)
        })
        .unwrap();
    // Repeated requests reuse the same pending question instead of spamming chat.
    f.app
        .invoke_tool(
            "grokbot",
            "enable_routine",
            json!({"routine_id":r.routine_id}),
        )
        .unwrap();
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn subbot_confirmation_is_visible_only_in_principal_chat_and_context() {
    let mut f = Fixture::new(false);
    f.post(
        "/api/bots",
        json!({"id":"child","name":"Child","color":"citron","description":"","instructions":""}),
    );
    let requested = output(
        f.app
            .invoke_tool(
                "child",
                "create_routine",
                json!({"name":"Child report","prompt":"Child task","schedule":"@every 1m"}),
            )
            .unwrap(),
    );
    let conv = requested["confirmation_conversation_id"].as_str().unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let c = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert_eq!(c["bot_id"], "grokbot");
    assert_eq!(c["kind"], "chat");
    assert!(c["messages"].to_string().contains("Child task"));
    assert!(f
        .app
        .context_preview(conv)
        .unwrap()
        .iter()
        .any(|b| b.text.contains("Child report")));
    assert!(matches!(f.app.invoke_tool("child","enable_routine",json!({"routine_id":requested["routine"]["routine_id"],"confirmation_message_id":"yes"})).unwrap(),ToolOutcome::Failed{..}));
}

#[test]
fn failing_tool_rolls_back_partial_effects_inside_engine_transaction() {
    let f = Fixture::new(false);
    let db = f.db();
    db.execute_batch("CREATE TRIGGER reject_confirmation BEFORE INSERT ON wrapper_routine_notices BEGIN SELECT RAISE(ABORT,'test storage failure'); END").unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let result = features::routines::call_tool(
        &ctx,
        &ToolCall {
            call_id: "create".into(),
            name: "create_routine".into(),
            arguments: json!({"name":"Atomic","prompt":"Test","schedule":"@every 1m"}),
        },
    )
    .unwrap();
    assert!(matches!(result, ToolOutcome::Failed { .. }));
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM scheduler_routines", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM wrapper_conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn past_once_resume_is_refused_and_creation_time_is_server_owned() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    assert!(r.modified_at > 0);
    let db = f.db();
    db.execute(
        "UPDATE scheduler_routines SET enabled=0,trigger=?2 WHERE routine_id=?1",
        params![r.routine_id, json!({"kind":"once","at":1}).to_string()],
    )
    .unwrap();
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/routines/resume",
                json!({"bot_id":"grokbot","routine_id":r.routine_id})
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        400
    );
}
