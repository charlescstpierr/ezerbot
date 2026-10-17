//! Forgetting is a local privacy policy (CM12), distinct from reset (CM11).
//! Exercise the actual Responses boundary, not only memory/context previews.
mod support;

use grokbot_engine::{
    compaction,
    contracts::{with_tx, HistoryItem, Role, RunEventKind, Summary, ToolOutcome},
    model::Run,
    run_loop, store,
};
use grokbot_memory::facts::{self, FactStatus};
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
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
    server: FakeOpenAi,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "grokbot-memory-forget-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ))
            .join("app.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"), json!({
            "host_id":"test-host", "client_id":CLIENT, "access_token":"access-2",
            "refresh_token":"refresh-2", "expires_at":now+3600, "scopes":"chatgpt.tokens.use.direct"
        }).to_string()).unwrap();
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("catalog", || provider.status()["models_ready"] == true);
        Self { app, server, path }
    }
    fn db(&self) -> Connection {
        let conn = Connection::open(&self.path).unwrap();
        conn.busy_timeout(Duration::from_secs(3)).unwrap();
        conn
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn conversation(&mut self, bot: &str) -> String {
        self.post(
            "/api/conversations",
            json!({"title":"Memory test", "bot_id":bot}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn send(&mut self, c: &str, message: &str, text: &str) {
        self.post(
            "/api/send",
            json!({"conversation_id":c, "message_id":message, "body":text}),
        );
        wait("completed reply", || {
            self.state()["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|conv| conv["id"] == c)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == format!("reply-{message}") && item["state"] == "echoed")
        });
    }
    fn run(&self, message: &str) -> Run {
        let record: String = self.db().query_row(
            "SELECT r.record FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1 ORDER BY r.rowid DESC LIMIT 1",
            [message], |row| row.get(0)
        ).unwrap();
        serde_json::from_str(&record).unwrap()
    }
    fn events(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn learn(&mut self, c: &str, message: &str, text: &str) -> String {
        self.events(vec![
            call(
                "learn",
                "remember",
                json!({"text":text, "sensitive":false, "source":"model_inferred"}),
            ),
            complete(),
        ]);
        self.send(c, message, "Retain the relevant inference.");
        facts::list_active(&self.db(), &self.run(message).bot_id)
            .unwrap()
            .into_iter()
            .find(|fact| fact.text == text)
            .unwrap()
            .fact_id
    }
    fn forget(&mut self, bot: &str, id: &str) -> Value {
        self.post("/api/memory/forget", json!({"bot_id":bot,"fact_id":id}))
    }
    fn bodies(&self) -> Vec<Value> {
        self.server.seen.lock().unwrap().bodies.clone()
    }
    fn add_bot(&mut self) {
        self.post("/api/bots", json!({"id":"other", "name":"Other", "color":"cyan", "description":"", "instructions":""}));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn call(id: &str, name: &str, arguments: Value) -> Value {
    json!({"type":"response.output_item.done", "item":{"id":format!("fc-{id}"), "type":"function_call", "status":"completed", "call_id":id, "name":name, "arguments":arguments.to_string()}})
}
fn complete() -> Value {
    json!({"type":"response.completed"})
}
fn text(delta: &str) -> Value {
    json!({"type":"response.output_text.delta", "delta":delta})
}
fn contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => {
            let escaped = serde_json::to_string(needle).unwrap();
            value.contains(needle) || value.contains(&escaped[1..escaped.len() - 1])
        }
        Value::Array(values) => values.iter().any(|value| contains(value, needle)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(needle) || contains(value, needle)),
        _ => false,
    }
}

#[test]
fn forgotten_text_absent_from_next_request_body() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Préférence confidentielle : \"abricot\"\n東京";
    let id = f.learn(&c, "learn", secret);
    assert!(f.bodies().iter().any(|body| contains(body, secret)));
    let fact = facts::get(&f.db(), &id).unwrap().unwrap();
    assert_eq!(f.forget("grokbot", &id)["forgotten"], json!([id]));
    let tombstone = facts::get(&f.db(), &id).unwrap().unwrap();
    assert_eq!(tombstone.status, FactStatus::Forgotten);
    assert!(tombstone.text.is_empty());
    assert_eq!(tombstone.learned_at, fact.learned_at);
    let revision = compaction::revision(&f.db(), &c).unwrap();
    f.forget("grokbot", &id);
    assert_eq!(compaction::revision(&f.db(), &c).unwrap(), revision);
    // Reopen, ensuring this is persisted redaction rather than in-memory masking.
    let provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("reopened catalog", || {
        provider.status()["models_ready"] == true
    });
    let before = f.bodies().len();
    f.send(&c, "next", "Continue without that memory.");
    let bodies = f.bodies();
    assert!(bodies.len() > before);
    for body in &bodies[before..] {
        assert!(!contains(body, secret));
    }
}

#[test]
fn forgotten_text_absent_from_tool_cards() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let original = "Private inference: lives near avenue Alpha.";
    let corrected = "Private inference: lives near avenue Bêta.";
    let old_id = f.learn(&c, "old", original);
    let other = f.conversation("grokbot");
    f.events(vec![
        call(
            "correct",
            "correct_fact",
            json!({"fact_id":old_id, "text":corrected, "source":"user_stated", "sensitive":null}),
        ),
        complete(),
    ]);
    f.send(&other, "correction", "Correct the previous inference.");
    let id = facts::list_active(&f.db(), "grokbot").unwrap()[0]
        .fact_id
        .clone();
    assert_ne!(id, old_id);
    assert!(contains(&f.state(), original));
    assert!(contains(&f.state(), corrected));
    // Cover the model-tool path too, which participates in the engine transaction.
    f.events(vec![
        call("forget", "forget_fact", json!({"fact_id":id})),
        complete(),
    ]);
    let before = f.bodies().len();
    f.send(&other, "forget", "Forget the previous inference.");
    let state = f.state();
    assert!(state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["kind"] == "tool")));
    for secret in [original, corrected] {
        assert!(!contains(&state, secret));
        // First request proposes forgetting, subsequent requests must be clean.
        for body in &f.bodies()[before + 1..] {
            assert!(!contains(body, secret));
        }
    }
    for id in [&id, &old_id] {
        let tombstone = facts::get(&f.db(), id).unwrap().unwrap();
        assert!(tombstone.text.is_empty());
        assert_eq!(tombstone.status, FactStatus::Forgotten);
    }
    assert!(compaction::revision(&f.db(), &c).unwrap() > 0);
    assert!(compaction::revision(&f.db(), &other).unwrap() > 0);
}

#[test]
fn covering_summary_is_regenerated() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "The remembered inference is velvet-lantern-927.";
    let id = f.learn(&c, "learn", secret);
    f.post("/api/engine/config", json!({"estimated_tokens":12000}));
    for n in 0..2 {
        f.send(&c, &format!("long-{n}"), &"x".repeat(12000));
    }
    f.events(vec![text(secret), complete()]); // The next call is compaction.
    f.send(&c, "long-2", &"x".repeat(12000));
    let old = compaction::latest(&f.db(), &c).unwrap().unwrap();
    assert_eq!(old.text, secret);
    f.forget("grokbot", &id);
    assert!(compaction::latest(&f.db(), &c).unwrap().is_none());
    assert!(!contains(&f.state(), secret));
    let before = f.bodies().len();
    f.send(
        &c,
        "regenerate-summary",
        "Continue using sanitized history.",
    );
    let fresh = compaction::latest(&f.db(), &c).unwrap().unwrap();
    assert_ne!(fresh.summary_id, old.summary_id);
    assert!(fresh.previous_summary_id.is_none());
    let bodies = f.bodies();
    assert!(bodies[before..].iter().any(|body| body["instructions"]
        .as_str()
        .unwrap()
        .starts_with("Summarize")));
    for body in &bodies[before..] {
        assert!(!contains(body, secret));
    }
    assert!(bodies.last().unwrap()["instructions"]
        .as_str()
        .unwrap()
        .contains(&fresh.text));
    let record: Option<String> = f
        .db()
        .query_row(
            "SELECT record FROM engine_summaries WHERE summary_id=?1",
            [&old.summary_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(record.is_none());
}

// Save exact known coverage to distinguish invalidation by provenance from
// accidental substring matching of the summary's prose.
fn summary_for_run(f: &Fixture, message: &str, summary_id: &str) -> Summary {
    let run = f.run(message);
    let mut history =
        run_loop::replay_items(&store::events(&f.db(), &run.run_id, 0).unwrap()).unwrap();
    let tail = HistoryItem {
        entry_id: format!("tail-{message}"),
        role: Role::User,
        text: "Retained tail".into(),
        images: vec![],
        tool_items: vec![],
    };
    let summary = Summary {
        summary_id: summary_id.into(),
        conversation_id: run.conversation_id,
        text: "A paraphrased durable inference without the literal forgotten text.".into(),
        truncation_last_entry_inclusive: history.last().unwrap().entry_id.clone(),
        resume_from_entry_inclusive: tail.entry_id.clone(),
        previous_summary_id: None,
        includes_tool_results: true,
        strategy: "test".into(),
    };
    history.push(tail);
    with_tx(&f.db(), |conn| {
        compaction::save_with_history(conn, &summary, &history)
    })
    .unwrap();
    summary
}

#[test]
fn every_correction_provenance_and_descendant_is_invalidated() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let old = f.learn(&c, "learn", "First version of a durable inference.");
    let original = summary_for_run(&f, "learn", "original-summary");
    let other = f.conversation("grokbot");
    f.events(vec![
        call(
            "correct",
            "correct_fact",
            json!({"fact_id":old,"text":"Second version of a durable inference.","sensitive":null}),
        ),
        complete(),
    ]);
    f.send(&other, "correction", "Correct it.");
    summary_for_run(&f, "correction", "correction-summary");
    let descendant = Summary {
        summary_id: "descendant-summary".into(),
        previous_summary_id: Some(original.summary_id),
        truncation_last_entry_inclusive: "tail-learn".into(),
        resume_from_entry_inclusive: "tail-next".into(),
        ..original
    };
    with_tx(&f.db(), |conn| compaction::save(conn, &descendant)).unwrap();
    let id = facts::list_active(&f.db(), "grokbot").unwrap()[0]
        .fact_id
        .clone();
    f.forget("grokbot", &id);
    let remaining: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM engine_summaries WHERE record IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn team_forget_redacts_other_bots_and_split_deltas_but_private_forget_does_not() {
    let mut f = Fixture::new();
    f.add_bot();
    let owner = f.conversation("other");
    let other = f.conversation("grokbot");
    let secret = "Shared inference: allergic to mauve kumquats.";
    let id = f.learn(&owner, "learn", secret);
    // An independent bot's journal must not be changed by a private forget.
    f.events(vec![
        text("Shared inference: allergic "),
        text("to mauve kumquats."),
        complete(),
    ]);
    f.send(&other, "echo", "Give your own inference.");
    summary_for_run(&f, "echo", "other-summary");
    f.forget("other", &id);
    assert!(compaction::latest(&f.db(), &other).unwrap().is_some());
    assert!(contains(&f.state(), secret));
    let id = f.learn(&owner, "team-learn", secret);
    f.post(
        "/api/memory/promote",
        json!({"bot_id":"other","fact_ids":[id]}),
    );
    f.post("/api/bots/delete", json!({"id":"other"}));
    f.forget("other", &id);
    assert!(compaction::latest(&f.db(), &other).unwrap().is_none());
    assert!(!contains(&f.state(), secret));
    let before = f.bodies().len();
    f.send(&other, "after-team-forget", "Continue.");
    for body in &f.bodies()[before..] {
        assert!(!contains(body, secret));
    }
}

#[test]
fn short_facts_preserve_protocol_keys_ids_and_replay() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    for (n, secret) in [
        "name", "id", "text", "status", "remember", "a", "learn", "fact", "scope", "output",
    ]
    .iter()
    .enumerate()
    {
        let message = format!("short-{n}");
        let id = f.learn(&c, &message, secret);
        let run = f.run(&message);
        let before = store::events(&f.db(), &run.run_id, 0).unwrap();
        f.forget("grokbot", &id);
        let after = store::events(&f.db(), &run.run_id, 0).unwrap();
        assert_eq!(before.len(), after.len());
        for (original, sanitized) in before.iter().zip(&after) {
            assert_eq!(original.kind, sanitized.kind);
            for key in ["name", "call_id", "response"] {
                assert_eq!(
                    original.payload[key], sanitized.payload[key],
                    "{secret}: {key}"
                );
            }
        }
        let call = after
            .iter()
            .find(|event| event.kind == RunEventKind::ToolCall)
            .unwrap();
        assert_eq!(call.payload["arguments"]["text"], "[redacted]");
        let result = after
            .iter()
            .find(|event| event.kind == RunEventKind::ToolResult)
            .unwrap();
        assert_eq!(result.payload["outcome"]["status"], "done");
        assert_eq!(result.payload["outcome"]["output"]["fact"]["fact_id"], id);
        assert_eq!(
            result.payload["outcome"]["output"]["fact"]["text"],
            "[redacted]"
        );
        assert!(run_loop::replay_items(&after).is_ok());
        // An actual provider request must still be emitted and finish normally.
        let count = f.bodies().len();
        f.send(&c, &format!("after-short-{n}"), "Continue.");
        assert!(f.bodies().len() > count);
    }
}

#[test]
fn task_content_is_redacted_without_changing_task_protocol() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Private inference carried in task objective and completion.";
    f.events(vec![
        call("begin", "begin_task", json!({"objective":secret})),
        call(
            "remember-task",
            "remember",
            json!({"text":secret,"sensitive":true}),
        ),
        call("finish", "complete_task", json!({"summary":secret})),
        complete(),
    ]);
    f.send(&c, "task", "Retain and summarize the inference.");
    let id = facts::list_active(&f.db(), "grokbot").unwrap()[0]
        .fact_id
        .clone();
    f.forget("grokbot", &id);
    let run = f.run("task");
    assert_eq!(
        run.task_status,
        Some(grokbot_engine::contracts::TaskStatus::Done {
            summary: "[redacted]".into()
        })
    );
    assert!(!contains(&f.state(), secret));
    let events = store::events(&f.db(), &run.run_id, 0).unwrap();
    assert!(run_loop::replay_items(&events).is_ok());
    assert!(events.iter().all(|event| !contains(&event.payload, secret)));
    let before = f.bodies().len();
    f.send(&c, "after-task", "Continue.");
    for body in &f.bodies()[before..] {
        assert!(!contains(body, secret));
    }
}

#[test]
fn opaque_reasoning_is_discarded_with_forgotten_memory() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Private inference: allergic to peanuts.";
    let opaque = "ENC-REASONING-ABOUT-PEANUTS";
    f.events(vec![
        json!({"type":"response.output_item.done", "item":{"type":"reasoning", "id":"rs-private", "encrypted_content":opaque, "summary":[]}}),
        call("remember-private", "remember", json!({"text":secret,"sensitive":true})),
        complete(),
    ]);
    f.send(&c, "learn-opaque", "Retain the relevant inference.");
    assert!(f.bodies().iter().any(|body| contains(body, opaque)));
    let id = facts::list_active(&f.db(), "grokbot").unwrap()[0]
        .fact_id
        .clone();
    f.forget("grokbot", &id);
    assert!(store::events(&f.db(), &f.run("learn-opaque").run_id, 0)
        .unwrap()
        .iter()
        .filter(|event| event.kind == RunEventKind::ToolCall)
        .all(|event| event.payload.get("reasoning").is_none()));
    let before = f.bodies().len();
    f.send(&c, "after-opaque", "Continue without that memory.");
    for body in &f.bodies()[before..] {
        assert!(!contains(body, secret));
        assert!(!contains(body, opaque));
    }
}

#[test]
fn failed_normalized_alias_is_also_forgotten() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Durable inference: prefers green tea.";
    let alias = "Durable  inference:\n prefers green tea.";
    let id = f.learn(&c, "learn", secret);
    f.events(vec![
        call(
            "failed-duplicate",
            "remember",
            json!({"text":alias,"sensitive":true}),
        ),
        complete(),
    ]);
    f.send(&c, "failed-duplicate", "Retain that inference again.");
    assert!(store::events(&f.db(), &f.run("failed-duplicate").run_id, 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == RunEventKind::ToolResult
            && event.payload["outcome"]["status"] == "failed"));
    f.forget("grokbot", &id);
    let before = f.bodies().len();
    f.send(&c, "after-failed-duplicate", "Continue.");
    for body in &f.bodies()[before..] {
        assert!(!contains(body, secret));
        assert!(!contains(body, alias));
    }
}

#[test]
fn normalized_duplicate_arguments_are_also_forgotten() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Durable inference: prefers green tea.";
    let alias = "Durable inference:\n prefers  green\t tea.  ";
    let id = f.learn(&c, "learn", secret);
    f.events(vec![
        call(
            "duplicate",
            "remember",
            json!({"text":alias,"sensitive":false}),
        ),
        complete(),
    ]);
    f.send(&c, "duplicate", "Retain that inference again.");
    assert_eq!(facts::list_active(&f.db(), "grokbot").unwrap().len(), 1);
    assert!(contains(&f.state(), alias));
    f.forget("grokbot", &id);
    for text in [secret, alias] {
        assert!(!contains(&f.state(), text));
    }
    let before = f.bodies().len();
    f.send(&c, "after", "Continue.");
    for body in &f.bodies()[before..] {
        for text in [secret, alias] {
            assert!(!contains(body, text));
        }
    }
}

struct PausedStream {
    api: String,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    sent: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
struct PausedBody {
    first: std::io::Cursor<Vec<u8>>,
    last: std::io::Cursor<Vec<u8>>,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}
impl std::io::Read for PausedBody {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = std::io::Read::read(&mut self.first, out)?;
        if n > 0 {
            return Ok(n);
        }
        let (lock, signal) = &*self.gate;
        drop(
            signal
                .wait_while(lock.lock().unwrap(), |released| !*released)
                .unwrap(),
        );
        std::io::Read::read(&mut self.last, out)
    }
}
impl PausedStream {
    fn new(secret: &str) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let api = format!("http://{}/v1", server.server_addr());
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (gating, stopping, sending) = (gate.clone(), stop.clone(), sent.clone());
        let secret = secret.to_string();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                let mut request_body = String::new();
                request
                    .as_reader()
                    .read_to_string(&mut request_body)
                    .unwrap();
                if request.url() == "/v1/models" {
                    let _ = request.respond(
                        tiny_http::Response::from_string(
                            json!({"models":[{"slug":"gpt-test","visibility":"list"}]}).to_string(),
                        )
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        ),
                    );
                } else {
                    // Enough bytes to flush before the gate, proving a model
                    // response is already in flight at the instant of forget.
                    let first = (0..150)
                        .map(|_| format!("data: {}\n\n", text("warmup ")))
                        .collect::<String>();
                    let last = format!("data: {}\n\ndata: {}\n\n", text(&secret), complete());
                    let body = PausedBody {
                        first: std::io::Cursor::new(first.into_bytes()),
                        last: std::io::Cursor::new(last.into_bytes()),
                        gate: gating.clone(),
                    };
                    let _ = request.respond(tiny_http::Response::new(
                        tiny_http::StatusCode(200),
                        vec![
                            tiny_http::Header::from_bytes("Content-Type", "text/event-stream")
                                .unwrap(),
                        ],
                        body,
                        None,
                        None,
                    ));
                    sending.store(true, Ordering::SeqCst);
                }
            }
        });
        Self {
            api,
            gate,
            stop,
            sent,
            thread: Some(thread),
        }
    }
    fn release(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}
impl Drop for PausedStream {
    fn drop(&mut self) {
        self.release();
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn forgetting_stops_concurrent_stream_before_late_plaintext_can_commit() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "A forgotten inference emitted after the forget commit.";
    let id = f.learn(&c, "learn", secret);
    let gated = PausedStream::new(secret);
    let mut endpoints = f.server.endpoints();
    endpoints.api = gated.api.clone();
    let provider = Provider::with_endpoints(&f.path, endpoints, false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("gated catalog", || {
        provider.status()["models_ready"] == true
    });
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"streaming","body":"Continue in the background."}),
    );
    let run = f.run("streaming");
    wait("first committed streaming delta", || {
        store::events(&f.db(), &run.run_id, 0)
            .unwrap()
            .iter()
            .any(|event| event.kind == RunEventKind::TextDelta)
    });
    f.forget("grokbot", &id);
    assert_eq!(
        store::required(&f.db(), &run.run_id).unwrap().status,
        grokbot_engine::model::RunStatus::Cancelled
    );
    gated.release();
    wait("late model bytes sent", || {
        gated.sent.load(Ordering::SeqCst)
    });
    let provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("restored catalog", || {
        provider.status()["models_ready"] == true
    });
    let before = f.bodies().len();
    f.send(&c, "after-stream", "Continue with sanitized context.");
    for body in &f.bodies()[before..] {
        assert!(!contains(body, secret));
    }
    assert!(!contains(&f.state(), secret));
    assert!(store::events(&f.db(), &run.run_id, 0)
        .unwrap()
        .iter()
        .all(|event| !contains(&event.payload, secret)));
}

#[test]
fn redaction_failure_rolls_back_wipe_and_summary_invalidation() {
    let mut f = Fixture::new();
    let c = f.conversation("grokbot");
    let secret = "Sensitive inference before transactional erasure.";
    let id = f.learn(&c, "learn", secret);
    summary_for_run(&f, "learn", "kept-on-rollback");
    // Fail on the second invalidation, after facts, run payloads AND the
    // covering summary have already been rewritten inside the transaction.
    f.db().execute_batch("CREATE TRIGGER reject_invalidation BEFORE INSERT ON engine_compaction_revisions WHEN EXISTS(SELECT 1 FROM engine_compaction_revisions WHERE conversation_id=NEW.conversation_id) BEGIN SELECT RAISE(FAIL,'revision unavailable'); END;").unwrap();
    let input = json!({"bot_id":"grokbot","fact_id":id}).to_string();
    assert!(f
        .app
        .api("POST", "/api/memory/forget", input.as_bytes())
        .is_err());
    assert!(matches!(
        f.app
            .invoke_tool("grokbot", "forget_fact", json!({"fact_id":id}))
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
    let fact = facts::get(&f.db(), &id).unwrap().unwrap();
    assert_eq!(fact.status, FactStatus::Active);
    assert_eq!(fact.text, secret);
    assert!(compaction::latest(&f.db(), &c).unwrap().is_some());
    assert_eq!(compaction::revision(&f.db(), &c).unwrap(), 0);
    assert!(store::events(&f.db(), &f.run("learn").run_id, 0)
        .unwrap()
        .iter()
        .any(|event| event.kind == RunEventKind::ToolCall && contains(&event.payload, secret)));
}
