//! Cutover regression tests against disposable SQLite and scripted Responses.
mod support;
use grokbot_engine::{
    contracts::*,
    model::{self, Run, RunStatus},
    store,
};
use grokbot_wrapper::{App, Provider};
use rusqlite::{params, Connection};
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
                "grokbot-engine-{}-{}",
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
            provider,
            server,
            path,
        }
    }
    fn post(&mut self, path: &str, value: Value) -> Value {
        self.app
            .api("POST", path, value.to_string().as_bytes())
            .unwrap()
    }
    fn conversation(&mut self) -> String {
        self.post("/api/conversations", json!({"title":"Test"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn send(&mut self, c: &str, m: &str, text: &str) -> Value {
        self.post(
            "/api/send",
            json!({"conversation_id":c,"message_id":m,"body":text}),
        )
    }
    fn reply(&mut self, c: &str, m: &str) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == c)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == format!("reply-{m}"))
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn done(&mut self, c: &str, m: &str) -> Value {
        wait("reply", || {
            let r = self.reply(c, m);
            !r.is_null() && r["state"] != "streaming"
        });
        let reply = self.reply(c, m);
        if reply["state"] == "failed" {
            eprintln!(
                "FAILED {reply} events={:?}",
                store::events(&self.db(), &self.run(m).run_id, 0).unwrap()
            );
        }
        reply
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
        c
    }
    fn events(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn run(&self, m: &str) -> Run {
        store::latest_run_for_trigger(
            &self.db(),
            &Trigger::UserMessage {
                message_id: m.into(),
                attempt: 0,
            },
        )
        .unwrap()
        .unwrap()
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
#[test]
fn simulated_mode_still_replies() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    let reply = f.send(&c, "sim", "bonjour");
    assert!(reply["conversation"]["messages"][1]["body"]
        .as_str()
        .unwrap()
        .starts_with("Bien reçu"));
    assert_eq!(f.run("sim").status, RunStatus::Finished);
    f.send(&c, "sim", "bonjour");
    assert_eq!(store::runs_for_conversation(&f.db(), &c).unwrap().len(), 1);
}
#[test]
fn user_message_reply_still_streams_as_reply_id() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let mut events = (0..100)
        .map(|_| json!({"type":"response.output_text.delta","delta":"x"}))
        .collect::<Vec<_>>();
    events.push(complete());
    f.events(events);
    let sent = f.send(&c, "stream", "bonjour");
    let reply = &sent["conversation"]["messages"][1];
    assert_eq!(reply["id"], "reply-stream");
    assert_eq!(reply["state"], "streaming");
    assert_eq!(f.done(&c, "stream")["body"], "x".repeat(100));
    assert_eq!(f.run("stream").task_status, None);
}
#[test]
fn retitle_after_first_and_third_turn() {
    let mut f = Fixture::new(true);
    let c = f.post("/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for n in 1..=3 {
        let m = format!("title-{n}");
        f.send(&c, &m, "Bonjour");
        f.done(&c, &m);
        wait("title job", || {
            f.db()
                .query_row(
                    "SELECT title_state FROM wrapper_engine_runs WHERE run_id=?1",
                    [f.run(&m).run_id],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
                == if n == 2 { "none" } else { "done" }
        });
    }
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(
        seen.bodies
            .iter()
            .filter(|b| b["instructions"].as_str().unwrap().contains("titre"))
            .count(),
        2
    );
    assert_eq!(f.provider.status()["titling"], false);
}
#[test]
fn tool_loop_runs_until_complete_task() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("a", "begin_task", json!({"objective":"Learn"})),
        complete(),
    ]);
    f.events(vec![
        call("b", "remember", json!({"text":"User likes Rust"})),
        complete(),
    ]);
    f.events(vec![
        call("c", "complete_task", json!({"summary":"Learned"})),
        complete(),
    ]);
    f.send(&c, "task", "Learn this");
    f.done(&c, "task");
    let run = f.run("task");
    assert_eq!(
        run.task_status,
        Some(TaskStatus::Done {
            summary: "Learned".into()
        })
    );
    let events = store::events(&f.db(), &run.run_id, 0).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == RunEventKind::ToolResult)
            .count(),
        3
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "tool"));
    let seen = f.server.seen.lock().unwrap();
    let input = seen.bodies[2]["input"].as_array().unwrap();
    assert_eq!(
        input
            .iter()
            .filter(|i| i["type"] == "function_call_output")
            .count(),
        2
    );
}
#[test]
fn task_turn_end_without_complete_task_shows_working() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![
        call("a", "begin_task", json!({"objective":"Work"})),
        complete(),
    ]);
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"Still working"}),
        complete(),
    ]);
    f.send(&c, "working", "Work");
    assert_eq!(f.done(&c, "working")["body"], "Still working");
    assert_eq!(f.run("working").task_status, Some(TaskStatus::Working));
}
#[test]
fn reset_keeps_memory_but_hides_history() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.app
        .invoke_tool("grokbot", "remember", json!({"text":"Durable preference"}))
        .unwrap();
    f.send(&c, "before", "Hidden before A");
    f.done(&c, "before");
    f.post("/api/conversations/reset", json!({"conversation_id":c}));
    let preview = serde_json::to_string(&f.app.context_preview(&c).unwrap()).unwrap();
    assert!(!preview.contains("Hidden before A"));
    assert!(preview.contains("Durable preference"));
    f.send(&c, "middle", "Visible after A");
    f.done(&c, "middle");
    f.post("/api/conversations/reset", json!({"conversation_id":c}));
    f.send(&c, "after", "Visible after B");
    f.done(&c, "after");
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(!body.contains("Hidden before A") && !body.contains("Visible after A"));
    assert!(body.contains("Durable preference"));
    f.post(
        "/api/regenerate",
        json!({"conversation_id":c,"message_id":"middle"}),
    );
    f.done(&c, "middle");
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(!body.contains("Hidden before A") && !body.contains("Visible after B"));
    assert!(body.contains("Visible after A"));
    assert_eq!(
        f.run("middle").trigger,
        Trigger::UserMessage {
            message_id: "middle".into(),
            attempt: 1
        }
    );
}
#[test]
fn long_conversation_compacts_and_keeps_transcript() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.post("/api/engine/config", json!({"estimated_tokens":12000}));
    for n in 0..8 {
        f.send(
            &c,
            &format!("long-{n}"),
            &format!("{}{}", n, "x".repeat(12000)),
        );
    }
    let db = f.db();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM engine_summaries WHERE record IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count > 0);
    let users: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM wrapper_messages WHERE kind='submission'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(users, 8);
    assert_eq!(
        db.query_row(
            "SELECT length(body) FROM wrapper_remote WHERE message_id='long-0'",
            [],
            |r| r.get::<_, usize>(0)
        )
        .unwrap(),
        12001
    );
}
#[test]
fn legacy_replies_migrate_once() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.send(&c, "legacy", "Original question");
    let db = f.db();
    db.execute_batch("DELETE FROM engine_run_events; DELETE FROM engine_runs; DELETE FROM wrapper_engine_runs; DELETE FROM component_versions WHERE component='wrapper.engine';").unwrap();
    db.execute(
        "UPDATE wrapper_remote SET reply='Original answer' WHERE message_id='legacy'",
        [],
    )
    .unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(f.reply(&c, "legacy")["body"], "Original answer");
    let before = store::events(&db, &f.run("legacy").run_id, 0).unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(
        store::events(&db, &f.run("legacy").run_id, 0).unwrap(),
        before
    );
}
#[test]
fn restart_mid_run_resumes_without_rerunning_committed_tool() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.send(&c, "recover", "Question");
    let db = f.db();
    let old = f.run("recover");
    let mut run = Run {
        run_id: "recover-run".into(),
        trigger: Trigger::UserMessage {
            message_id: "recover".into(),
            attempt: 1,
        },
        status: RunStatus::Queued,
        turn_end: None,
        ..old
    };
    let tx = db.unchecked_transaction().unwrap();
    run = model::start(&tx, &run).unwrap();
    model::set_running_at(&tx, &run.run_id, 1).unwrap();
    tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,0,'simulated')",params![run.run_id,serde_json::to_string(&HistoryItem{entry_id:"recover".into(),role:Role::User,text:"Question".into(),images:vec![],tool_items:vec![]}).unwrap()]).unwrap();
    store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::ToolCall,
        &json!({"call_id":"kept","name":"remember","arguments":{"text":"Only once"},"response":1}),
    )
    .unwrap();
    store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::ToolResult,
        &json!({"call_id":"kept","outcome":{"status":"done","output":"Committed"}}),
    )
    .unwrap();
    store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::TextDelta,
        &json!({"response":2,"text":"Abandoned partial"}),
    )
    .unwrap();
    tx.commit().unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(f.run("recover").status, RunStatus::Interrupted);
    f.app.tick(1000);
    assert_eq!(f.run("recover").status, RunStatus::Finished);
    assert!(!f.reply(&c, "recover")["body"]
        .as_str()
        .unwrap()
        .contains("Abandoned"));
    assert_eq!(
        store::events(&db, &run.run_id, 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == RunEventKind::ToolResult)
            .count(),
        1
    );
}
#[test]
fn stop_keeps_partial_text_and_regenerate_increments_attempt() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.send(&c, "stop", "Question");
    let db = f.db();
    let old = f.run("stop");
    let run = Run {
        run_id: "stop-run".into(),
        trigger: Trigger::UserMessage {
            message_id: "stop".into(),
            attempt: 1,
        },
        status: RunStatus::Queued,
        turn_end: None,
        ..old
    };
    let tx = db.unchecked_transaction().unwrap();
    model::start(&tx, &run).unwrap();
    model::set_running_at(&tx, &run.run_id, 1).unwrap();
    tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) SELECT ?1,input_json,live,model FROM wrapper_engine_runs WHERE run_id<>?1 LIMIT 1",[&run.run_id]).unwrap();
    store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::TextDelta,
        &json!({"text":"Visible partial","response":1}),
    )
    .unwrap();
    tx.commit().unwrap();
    f.post(
        "/api/stop",
        json!({"conversation_id":c,"message_id":"stop"}),
    );
    let reply = f.reply(&c, "stop");
    assert_eq!(reply["state"], "stopped");
    assert_eq!(reply["body"], "Visible partial");
    f.post(
        "/api/regenerate",
        json!({"conversation_id":c,"message_id":"stop"}),
    );
    assert_eq!(
        f.run("stop").trigger,
        Trigger::UserMessage {
            message_id: "stop".into(),
            attempt: 2
        }
    );
}

#[test]
fn pending_wake_is_delivered_by_tick_only_after_committed_resume() {
    use grokbot_forms::requests::{
        self, Field, FieldType, HumanInputRequest, InputKind, RequestStatus,
    };
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.send(&c, "wake", "Question");
    let db = f.db();
    let old = f.run("wake");
    let run = Run {
        run_id: "wake-run".into(),
        trigger: Trigger::UserMessage {
            message_id: "wake".into(),
            attempt: 1,
        },
        status: RunStatus::Queued,
        turn_end: None,
        ..old
    };
    let tx = db.unchecked_transaction().unwrap();
    model::start(&tx, &run).unwrap();
    model::set_running_at(&tx, &run.run_id, 1).unwrap();
    tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) SELECT ?1,input_json,0,'simulated' FROM wrapper_engine_runs WHERE run_id<>?1 LIMIT 1",[&run.run_id]).unwrap();
    requests::create(
        &tx,
        &HumanInputRequest {
            id: "form-request".into(),
            run_id: run.run_id.clone(),
            entry_id: "form-entry".into(),
            route: ApprovalRoute {
                requesting_bot_id: "grokbot".into(),
                target_bot_id: "grokbot".into(),
                target_conversation_id: c.clone(),
            },
            kind: InputKind::Form {
                fields: vec![Field {
                    name: "answer".into(),
                    label: "Answer".into(),
                    field_type: FieldType::Text,
                    required: true,
                }],
            },
            status: RequestStatus::Pending,
        },
    )
    .unwrap();
    store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::ToolCall,
        &json!({"call_id":"form-call","name":"request_form","arguments":{},"response":1}),
    )
    .unwrap();
    store::append_event(&tx,&run.run_id,RunEventKind::ToolResult,&json!({"call_id":"form-call","outcome":{"status":"suspend","cause":{"kind":"human_input","id":"form-request"}}})).unwrap();
    let mut suspended = store::get(&tx, &run.run_id).unwrap().unwrap();
    suspended.status = RunStatus::AwaitingHumanInput;
    store::save(&tx, &suspended).unwrap();
    store::append_event(&tx,&run.run_id,RunEventKind::Suspended,&json!({"cause":{"kind":"human_input","id":"form-request"},"call_id":"form-call","approval":false})).unwrap();
    tx.commit().unwrap();
    f.app.tick(1000);
    assert_eq!(f.run("wake").status, RunStatus::AwaitingHumanInput);
    let tx = db.unchecked_transaction().unwrap();
    let receipt = requests::submit(
        &tx,
        "form-entry",
        "submission",
        &json!({"answer":"A validated answer"}),
    )
    .unwrap();
    let wake =
        store::enqueue_wake(&tx, &run.run_id, WakeCause::Form, &receipt.resolution_id).unwrap();
    requests::attach_wake(&tx, &receipt.resolution_id, &wake).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        store::wake_status(&db, &wake.wake_id).unwrap(),
        WakeStatus::Pending
    );
    // A crash between submit and delivery does not create a new run.
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    f.app.tick(1001);
    let WakeStatus::Delivered {
        run_id,
        first_event_seq,
    } = store::wake_status(&db, &wake.wake_id).unwrap()
    else {
        panic!("wake not delivered")
    };
    assert_eq!(run_id, run.run_id);
    let resumed = store::events(&db, &run.run_id, first_event_seq - 1)
        .unwrap()
        .remove(0);
    assert_eq!(resumed.kind, RunEventKind::Resumed);
    assert_eq!(f.run("wake").status, RunStatus::Finished);
    f.app.tick(1002);
    assert_eq!(
        store::events(&db, &run.run_id, 0)
            .unwrap()
            .iter()
            .filter(|e| e.kind == RunEventKind::Resumed)
            .count(),
        1
    );
}

#[test]
fn legacy_streaming_reply_is_interrupted_not_failed_or_discarded() {
    let mut f = Fixture::new(false);
    let c = f.conversation();
    f.send(&c, "old-stream", "Question");
    let db = f.db();
    db.execute_batch("DELETE FROM engine_run_events; DELETE FROM engine_runs; DELETE FROM wrapper_engine_runs; DELETE FROM component_versions WHERE component='wrapper.engine';").unwrap();
    db.execute(
        "UPDATE wrapper_remote SET reply='Visible old partial' WHERE message_id='old-stream'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO wrapper_generation(account_id,conversation_id,message_id,status,updated_at) VALUES ('local-workspace',?1,'old-stream','streaming',1)",[&c]).unwrap();
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(f.run("old-stream").status, RunStatus::Interrupted);
    assert_eq!(f.reply(&c, "old-stream")["body"], "Visible old partial");
}

#[test]
fn typed_input_sends_image_once_then_replays_caption_only() {
    use grokbot_wrapper::features::engine;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let image = ImageInput {
        attachment_id: "image-1".into(),
        url: Some("data:image/png;base64,iVBORw==".into()),
        caption: Some("A blue square".into()),
    };
    let block = engine::image_input_block(std::slice::from_ref(&image)).unwrap();
    assert_eq!(block.source, ContextSource::ImageCaption);
    let mut ctx = f.app.feature_context();
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(&c);
    let run = engine::start_run(
        &ctx,
        Trigger::Occurrence {
            occurrence_id: "image-occurrence".into(),
        },
        HistoryItem {
            entry_id: "image-origin".into(),
            role: Role::User,
            text: "Look".into(),
            images: vec![image],
            tool_items: vec![],
        },
    )
    .unwrap();
    wait("image run", || {
        store::get(&f.db(), &run.run_id)
            .unwrap()
            .unwrap()
            .status
            .is_terminal()
    });
    let first = f.server.seen.lock().unwrap().bodies[0].to_string();
    assert!(first.contains("input_image") && first.contains("A blue square"));
    f.send(&c, "later-image", "What color?");
    f.done(&c, "later-image");
    let last = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(!last.contains("input_image") && !last.contains("base64"));
    assert!(last.contains("A blue square"));
}

#[test]
fn model_form_suspends_and_resumes_through_wrapper_receipt() {
    use grokbot_forms::requests;
    let mut f = Fixture::new(true);
    let c = f.conversation();
    f.events(vec![call("form-model","request_form",json!({"fields":[{"name":"answer","label":"Answer","field_type":{"kind":"text"},"required":true}]})),complete()]);
    f.send(&c, "actual-form", "Ask me");
    f.done(&c, "actual-form");
    let run = f.run("actual-form");
    assert_eq!(run.status, RunStatus::AwaitingHumanInput);
    let request = requests::list_for_run(&f.db(), &run.run_id)
        .unwrap()
        .remove(0);
    let receipt=f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"answer-1","conversation_id":c,"values":{"answer":"Ocean blue"}}));
    assert_eq!(receipt["receipt"]["wake"]["status"], "pending");
    f.app.tick(1000);
    wait("real form resumed", || {
        f.run("actual-form").status.is_terminal()
    });
    assert_eq!(f.run("actual-form").run_id, run.run_id);
    let replayed=f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"answer-1","conversation_id":c,"values":{}}));
    assert_eq!(replayed["receipt"]["wake"]["status"], "delivered");
    let body = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(body.contains("Ocean blue") && body.contains("function_call_output"));
}

#[test]
fn linked_upload_uses_typed_image_and_reset_hides_its_caption() {
    let mut f = Fixture::new(true);
    let c = f.conversation();
    let uploaded = f
        .app
        .api(
            "POST",
            "/api/attachments",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01image payload",
        )
        .unwrap();
    let id = uploaded["id"].as_str().unwrap().to_owned();
    f.events(vec![
        json!({"type":"response.output_text.delta","delta":"PRIVATE_BLUE_SQUARE_CAPTION"}),
        complete(),
    ]);
    let caption = json!({"conversation_id":c,"attachment_id":id,"draft":"Look"});
    f.post("/api/attachments/precaption", caption.clone());
    wait("caption ready", || {
        f.post("/api/attachments/precaption", caption.clone())["caption"]["status"] == "ready"
    });
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"linked-image","body":"Look","attachments":[id]}),
    );
    f.done(&c, "linked-image");
    let sent = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(sent.contains("input_image") && sent.contains("PRIVATE_BLUE_SQUARE_CAPTION"));
    let stored: String = f
        .db()
        .query_row(
            "SELECT input_json FROM wrapper_engine_runs WHERE run_id=?1",
            [f.run("linked-image").run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!stored.contains("base64"));
    f.send(&c, "linked-later", "What color?");
    f.done(&c, "linked-later");
    let later = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(!later.contains("input_image"));
    assert!(later.contains("PRIVATE_BLUE_SQUARE_CAPTION"));
    f.post("/api/conversations/reset", json!({"conversation_id":c}));
    f.send(&c, "linked-reset", "Fresh start");
    f.done(&c, "linked-reset");
    let reset = f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .last()
        .unwrap()
        .to_string();
    assert!(!reset.contains("PRIVATE_BLUE_SQUARE_CAPTION") && !reset.contains("input_image"));
}
