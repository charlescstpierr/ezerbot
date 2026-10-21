//! End-to-end fault injection against fake Codex only. No OAuth/account/network outside loopback.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use grokbot_wrapper::{App, Endpoints, Provider};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
enum Fault {
    Happy,
    Headers,
    Silent,
    Partial,
    Keepalive,
    EndlessEvents,
    Drop,
    ToolStall,
    ToolApproval,
    Invalid,
    Refusal,
    Http(u16),
    RefreshHang,
    RefreshExpired,
    Incomplete,
}
struct Fake {
    base: String,
    stop: Arc<AtomicBool>,
    queue: Arc<Mutex<VecDeque<Fault>>>,
    seen: Arc<Mutex<Vec<Value>>>,
    refresh: Arc<Mutex<Option<Fault>>>,
    side_hang: Arc<AtomicBool>,
    slow_summary: Arc<AtomicBool>,
    summaries: Arc<AtomicUsize>,
    partial_released: Arc<AtomicBool>,
    partial_finished: Arc<AtomicBool>,
    holds_finished: Arc<AtomicUsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}
fn event(stream: &mut TcpStream, value: Value) {
    let _ = write!(stream, "data: {value}\n\n");
    let _ = stream.flush();
}
fn hold(stream: &mut TcpStream, stop: &AtomicBool, keepalive: bool) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut buf = [0u8; 1];
    while !stop.load(Ordering::SeqCst) {
        if keepalive && (stream.write_all(b": ping\n\n").is_err() || stream.flush().is_err()) {
            break;
        }
        match stream.peek(&mut buf) {
            Ok(0) => break,
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
            Ok(_) => std::thread::sleep(Duration::from_millis(30)),
        }
    }
}
fn token() -> String {
    format!(
        "e30.{}.fake",
        URL_SAFE_NO_PAD.encode(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":"fake-only"}}).to_string()
        )
    )
}
impl Fake {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let queue = Arc::new(Mutex::new(VecDeque::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let refresh = Arc::new(Mutex::new(None));
        let side_hang = Arc::new(AtomicBool::new(false));
        let slow_summary = Arc::new(AtomicBool::new(false));
        let summaries = Arc::new(AtomicUsize::new(0));
        let (summary_slow, summary_count) = (slow_summary.clone(), summaries.clone());
        let partial_released = Arc::new(AtomicBool::new(false));
        let partial_finished = Arc::new(AtomicBool::new(false));
        let holds_finished = Arc::new(AtomicUsize::new(0));
        let (s, q, b, r, side, late, finished, holds) = (
            stop.clone(),
            queue.clone(),
            seen.clone(),
            refresh.clone(),
            side_hang.clone(),
            partial_released.clone(),
            partial_finished.clone(),
            holds_finished.clone(),
        );
        let thread = std::thread::spawn(move || {
            let mut workers = vec![];
            while !s.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let (s, q, b, r, side, late, finished, holds) = (
                    s.clone(),
                    q.clone(),
                    b.clone(),
                    r.clone(),
                    side.clone(),
                    late.clone(),
                    finished.clone(),
                    holds.clone(),
                );
                let (summary_slow, summary_count) = (summary_slow.clone(), summary_count.clone());
                workers.push(std::thread::spawn(move || {
                    // Accepted sockets can inherit O_NONBLOCK on BSD/macOS.
                    // Only the listener is polled; request parsing must block.
                    socket.set_nonblocking(false).expect("fake accepted socket blocking mode");
                    socket.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                    let mut reader = BufReader::new(socket.try_clone().unwrap()); let mut line = String::new();
                    if reader.read_line(&mut line).expect("fake request-line read failed") == 0 { return; } let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                    let mut length=0; loop { line.clear(); if reader.read_line(&mut line).expect("fake request-header read failed")==0 || line=="\r\n" { break; }
                        if let Some((name,v))=line.split_once(':') { if name.eq_ignore_ascii_case("content-length") { length=v.trim().parse().unwrap(); } } }
                    let mut bytes=vec![0;length]; if reader.read_exact(&mut bytes).is_err() { return; }
                    if path.ends_with("/models") { let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n{{\"models\":[]}}"); return; }
                    if path=="/token" {
                        let refresh = *r.lock().unwrap();
                        match refresh {
                            Some(Fault::RefreshHang) => {
                                while r.lock().unwrap().is_some() && !s.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(10)); }
                                if s.load(Ordering::SeqCst) { return; }
                                let body=json!({"access_token":token(),"refresh_token":"fake-rotated","expires_in":3600}).to_string();
                                let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body); return;
                            },
                            Some(Fault::RefreshExpired) => { let body=json!({"error":{"code":"invalid_grant"}}).to_string(); let _=write!(socket,"HTTP/1.1 400 Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body); return; },
                            _ => { let body=json!({"access_token":token(),"refresh_token":"fake-rotated","expires_in":3600}).to_string(); let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body); return; }
                        }
                    }
                    // /api/state polls Codex quota with an empty GET. It is not a chat body.
                    if path.ends_with("/wham/usage") {
                        let body = json!({"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":35,"limit_window_seconds":18000,"reset_at":4102444800u64},"secondary_window":{"used_percent":82,"limit_window_seconds":604800,"reset_at":4102531200u64}}}).to_string();
                        let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
                        return;
                    }
                    let body: Value=serde_json::from_slice(&bytes).unwrap(); b.lock().unwrap().push(body.clone());
                    let instructions=body["instructions"].as_str().unwrap_or("");
                    let internal=instructions.contains("titre") || instructions.starts_with("Extraction") || instructions.contains("description") || body["input"].to_string().contains("input_image");
                    let fault=if internal { if side.load(Ordering::SeqCst) { Fault::Headers } else { Fault::Happy } } else { q.lock().unwrap().pop_front().unwrap_or(Fault::Happy) };
                    if matches!(fault,Fault::Headers) { hold(&mut socket,&s,false); holds.fetch_add(1, Ordering::SeqCst); return; }
                    if let Fault::Http(code)=fault { let body=json!({"error":{"code":"fake_error","message":"Erreur factice précise"}}).to_string(); let _=write!(socket,"HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body); return; }
                    let _=write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"); let _=socket.flush();
                    if instructions.starts_with("Summarize the supplied conversation") {
                        summary_count.fetch_add(1, Ordering::SeqCst);
                        if summary_slow.swap(false, Ordering::SeqCst) {
                            for _ in 0..13 {
                                if s.load(Ordering::SeqCst) { return; }
                                event(&mut socket,json!({"type":"response.output_text.delta","delta":"Résumé. "}));
                                std::thread::sleep(Duration::from_secs(1));
                            }
                            event(&mut socket,json!({"type":"response.completed"}));
                            return;
                        }
                    }
                    match fault {
                        Fault::Silent => { hold(&mut socket,&s,false); holds.fetch_add(1, Ordering::SeqCst); }
                        Fault::Partial | Fault::Drop => {
                            event(&mut socket,json!({"type":"response.output_text.delta","delta":"Début conservé."}));
                            if matches!(fault,Fault::Partial) {
                                while !late.load(Ordering::SeqCst) && !s.load(Ordering::SeqCst) { std::thread::sleep(Duration::from_millis(5)); }
                                if !s.load(Ordering::SeqCst) {
                                    event(&mut socket,json!({"type":"response.output_text.delta","delta":"Résultat tardif à ignorer."}));
                                    event(&mut socket,json!({"type":"response.completed"}));
                                    finished.store(true,Ordering::SeqCst);
                                }
                            }
                        },
                        Fault::Keepalive => { hold(&mut socket,&s,true); holds.fetch_add(1, Ordering::SeqCst); }
                        Fault::EndlessEvents => { while !s.load(Ordering::SeqCst) { event(&mut socket,json!({"type":"response.in_progress"})); std::thread::sleep(Duration::from_millis(25)); } },
                        Fault::ToolStall => { event(&mut socket,json!({"type":"response.output_item.added","item":{"type":"function_call","id":"fc","call_id":"waiting","name":"local_exec"}})); hold(&mut socket,&s,true); },
                        Fault::ToolApproval => { event(&mut socket,json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc","call_id":"ask","name":"run_shell","arguments":"{\"command\":\"echo faux\",\"cwd\":\"/tmp\",\"timeout_ms\":1000,\"output_limit\":1024}"}})); event(&mut socket,json!({"type":"response.completed"})); },
                        Fault::Invalid => { let _=socket.write_all(b"data: {broken}\n\n"); },
                        Fault::Refusal => event(&mut socket,json!({"type":"response.refusal.delta","delta":"refus factice"})),
                        Fault::Incomplete => event(&mut socket,json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}})),
                        _ => { event(&mut socket,json!({"type":"response.output_text.delta","delta":"Réponse factice terminée."})); event(&mut socket,json!({"type":"response.completed"})); },
                    }
                }));
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            queue,
            seen,
            refresh,
            side_hang,
            slow_summary,
            summaries,
            partial_released,
            partial_finished,
            holds_finished,
            thread: Some(thread),
        }
    }
    fn endpoints(&self) -> Endpoints {
        Endpoints {
            api: format!("{}/backend-api/codex", self.base),
            token: format!("{}/token", self.base),
            ..Endpoints::default()
        }
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
struct Fixture {
    fake: Fake,
    db: PathBuf,
    provider: Provider,
    app: App,
    chat: String,
}
impl Fixture {
    fn new(signed: bool) -> Self {
        Self::with_deadline(signed, Duration::from_secs(2))
    }
    fn with_deadlines(signed: bool, first: Duration, stall: Duration, total: Duration) -> Self {
        let fixture = Self::with_deadline(signed, first);
        fixture
            .provider
            .configure_fake_deadlines(first, stall, total);
        fixture
    }
    fn with_deadline(signed: bool, deadline: Duration) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-reliable-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("app.sqlite");
        let fake = Fake::new();
        if signed {
            save_token(&db, false);
        }
        let provider = Provider::with_backends(&db, fake.endpoints(), fake.endpoints(), false);
        provider.configure_fake_chat(deadline, false);
        let mut app = App::open_with(&db, provider.clone()).unwrap();
        let chat = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        // Populate the static catalogue before launching a turn.
        until(|| {
            app.api("GET", "/api/state", b"").unwrap()["provider"]["models_ready"] == true
                || !signed
        });
        Self {
            fake,
            db,
            provider,
            app,
            chat,
        }
    }
    fn send(&mut self, fault: Fault) {
        self.fake.queue.lock().unwrap().push_back(fault);
        post(
            &mut self.app,
            "/api/send",
            json!({"conversation_id":self.chat,"message_id":"question","body":"Question privée jamais dans les logs"}),
        );
    }
    fn reply(&mut self) -> Value {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == self.chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reply-question")
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn settled(&mut self) -> Value {
        until(|| !self.reply().is_null() && self.reply()["state"] != "streaming");
        self.reply()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.db.parent().unwrap());
    }
}
fn save_token(db: &std::path::Path, expired: bool) {
    std::fs::write(db.with_extension("codex-auth.json"),json!({"host_id":"fake-only","account_id":"fake-only","client_id":"fake","access_token":token(),"refresh_token":"fake-refresh-never-log","expires_at":if expired {1u64} else {4102444800u64},"scopes":"openid offline_access","welcomed":true}).to_string()).unwrap();
}
fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}
fn until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "fake chat did not settle"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn every_transport_hang_ends_with_a_persisted_french_error_and_retry_works() {
    for fault in [
        Fault::Headers,
        Fault::Silent,
        Fault::Partial,
        Fault::Keepalive,
        Fault::EndlessEvents,
        Fault::Drop,
        Fault::Invalid,
        Fault::Refusal,
        Fault::Incomplete,
        Fault::Http(503),
    ] {
        let mut f = Fixture::new(true);
        let start = Instant::now();
        f.send(fault);
        let failed = f.settled();
        assert_eq!(failed["state"], "failed", "{failed}");
        assert!(!failed["error"].as_str().unwrap().is_empty());
        assert!(start.elapsed() < Duration::from_secs(15));
        f.fake.queue.lock().unwrap().push_back(Fault::Happy);
        post(
            &mut f.app,
            "/api/regenerate",
            json!({"conversation_id":f.chat,"message_id":"question"}),
        );
        assert_eq!(f.settled()["state"], "echoed");
        let log = std::fs::read_to_string(f.db.parent().unwrap().join("logs/engine.log")).unwrap();
        assert!(log.contains("code=completed"));
        assert!(
            !log.contains("Question privée")
                && !log.contains("Réponse factice")
                && !log.contains("fake-refresh")
                && !log.contains(&token())
        );
    }
}
#[test]
fn stop_is_immediate_and_late_worker_cannot_mutate_the_turn() {
    // Cancellation, not network-deadline timing, is the subject of this test.
    let mut f = Fixture::with_deadline(true, Duration::from_secs(10));
    f.send(Fault::Partial);
    until(|| {
        let reply = f.reply();
        assert_ne!(
            reply["state"], "failed",
            "partial response failed before cancellation: {reply}"
        );
        reply["body"] == "Début conservé."
    });
    let start = Instant::now();
    post(
        &mut f.app,
        "/api/stop",
        json!({"conversation_id":f.chat,"message_id":"question"}),
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(f.reply()["state"], "stopped");
    f.fake.queue.lock().unwrap().push_back(Fault::Happy);
    post(
        &mut f.app,
        "/api/regenerate",
        json!({"conversation_id":f.chat,"message_id":"question"}),
    );
    let reply = f.settled();
    assert_eq!(reply["body"], "Réponse factice terminée.");
    // Release a real late completion only after the replacement has finished.
    // A controlled event is stronger than hoping an old socket timed out.
    f.fake.partial_released.store(true, Ordering::SeqCst);
    until(|| f.fake.partial_finished.load(Ordering::SeqCst));
    assert_eq!(f.reply(), reply);
}
#[test]
fn refresh_hang_never_blocks_state_or_dispatches_after_cancellation() {
    let mut f = Fixture::new(true);
    save_token(&f.db, true);
    *f.fake.refresh.lock().unwrap() = Some(Fault::RefreshHang);
    f.send(Fault::Happy);
    let start = Instant::now();
    assert!(f.app.api("GET", "/api/state", b"").is_ok());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(f.settled()["state"], "failed");
    assert!(f.fake.seen.lock().unwrap().is_empty());
    *f.fake.refresh.lock().unwrap() = None;
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        f.fake.seen.lock().unwrap().is_empty(),
        "late refresh must not dispatch an abandoned request"
    );
    post(
        &mut f.app,
        "/api/regenerate",
        json!({"conversation_id":f.chat,"message_id":"question"}),
    );
    assert_eq!(f.settled()["state"], "echoed");
}
#[test]
fn missing_or_expired_signin_is_a_durable_card_not_simulated_text() {
    for expired in [false, true] {
        let mut f = Fixture::new(expired);
        if expired {
            save_token(&f.db, true);
            *f.fake.refresh.lock().unwrap() = Some(Fault::RefreshExpired);
        }
        f.send(Fault::Happy);
        let reply = f.settled();
        assert_eq!(
            reply["error_kind"],
            if expired { "signin_expired" } else { "signin" }
        );
        if expired {
            assert!(reply["error"]
                .as_str()
                .unwrap()
                .contains("session ChatGPT a expiré"));
        }
        assert_eq!(reply["body"], "");
        let mut reopened = App::open_with(&f.db, f.provider.clone()).unwrap();
        let state = reopened.api("GET", "/api/state", b"").unwrap();
        assert!(state["conversations"].to_string().contains("signin"));
        save_token(&f.db, false);
        until(|| f.provider.status()["models_ready"] == true);
        post(
            &mut f.app,
            "/api/regenerate",
            json!({"conversation_id":f.chat,"message_id":"question"}),
        );
        assert_eq!(f.settled()["state"], "echoed");
    }
}
#[test]
fn group_without_signin_gets_a_card_and_an_empty_catalogue_cannot_queue_forever() {
    for signed in [false, true] {
        let mut f = Fixture::new(signed);
        if signed {
            std::fs::write(f.db.with_extension("chatgpt-auth.json"),json!({"host_id":"fake","client_id":"fake","access_token":"fake","refresh_token":"fake","expires_at":4102444800u64,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
            f.provider.select_backend("sharing").unwrap();
            until(|| f.provider.status()["models_ready"] == true);
        }
        post(
            &mut f.app,
            "/api/bots",
            json!({"id":"peer","name":"Pair","color":"cyan","instructions":""}),
        );
        let group = post(
            &mut f.app,
            "/api/conversations",
            json!({"bot_ids":["grokbot","peer"]}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":group,"message_id":"group-question","body":"Bonjour"}),
        );
        until(|| {
            f.app.tick(1792512000000);
            let state = f.app.api("GET", "/api/state", b"").unwrap();
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == group)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "assistant" && m["state"] == "failed")
        });
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        let chat = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == group)
            .unwrap();
        assert!(!chat["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["state"] == "streaming"));
        assert!(chat["messages"].to_string().contains(if signed {
            "Aucun modèle"
        } else {
            "signin"
        }));
    }
}

#[test]
fn pending_tool_is_visible_then_times_out_without_dispatch() {
    let mut f = Fixture::new(true);
    f.send(Fault::ToolStall);
    until(|| {
        f.reply()["notice"]
            .as_str()
            .is_some_and(|s| s.starts_with("Préparation d’un outil"))
    });
    assert_eq!(f.settled()["state"], "failed");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let calls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_run_events WHERE kind='tool_call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(calls, 0);
}
#[test]
fn stopping_tool_preparation_clears_the_durable_notice() {
    let mut f = Fixture::with_deadline(true, Duration::from_secs(10));
    f.send(Fault::ToolStall);
    until(|| {
        f.reply()["notice"]
            .as_str()
            .is_some_and(|s| s.starts_with("Préparation d’un outil"))
    });
    let run = f.reply()["payload"]["run_id"].clone();
    post(
        &mut f.app,
        "/api/stop",
        json!({"conversation_id":f.chat,"message_id":"question"}),
    );
    until(|| {
        f.reply()["state"] == "stopped"
            && !f.provider.status()["running"].as_bool().unwrap_or(false)
    });
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    until(|| {
        let notice: Option<String> = conn
            .query_row(
                "SELECT notice FROM wrapper_engine_runs WHERE run_id=?1",
                [run.as_str().unwrap()],
                |r| r.get(0),
            )
            .unwrap();
        !notice
            .unwrap_or_default()
            .starts_with("Préparation d’un outil")
    });
}

#[test]
fn slow_compaction_is_foreground_work_and_logs_the_user_run() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::with_deadline(true, Duration::from_secs(25));
    post(
        &mut f.app,
        "/api/engine/config",
        json!({"estimated_tokens":12000}),
    );
    f.fake.slow_summary.store(true, Ordering::SeqCst);
    let mut slow_run = None;
    for n in 0..6 {
        let id = format!("long-{n}");
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":f.chat,"message_id":id,"body":format!("{n}{}", "x".repeat(12000))}),
        );
        let mut reply = Value::Null;
        until(|| {
            let state = f.app.api("GET", "/api/state", b"").unwrap();
            reply = state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == f.chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == format!("reply-{id}"))
                .cloned()
                .unwrap_or(Value::Null);
            !reply.is_null() && reply["state"] != "streaming"
        });
        assert_eq!(reply["state"], "echoed", "{reply}");
        if f.fake.summaries.load(Ordering::SeqCst) > 0 {
            slow_run = reply["payload"]["run_id"].as_str().map(str::to_owned);
            break;
        }
    }
    let run = slow_run.expect("long history must compact");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_summaries WHERE record IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(count > 0);
    let hash = format!("{:x}", Sha256::digest(run.as_bytes()));
    let log = std::fs::read_to_string(f.db.parent().unwrap().join("logs/engine.log")).unwrap();
    assert_eq!(
        log.lines()
            .filter(|line| line.contains(&format!("turn={}", &hash[..16]))
                && line.contains("code=completed"))
            .count(),
        2,
        "compaction and reply must share the run: {log}"
    );
    assert!(!log.contains("code=turn_timeout"));
}

#[test]
fn background_memory_and_title_do_not_block_a_second_turn() {
    let mut f = Fixture::new(true);
    f.fake.side_hang.store(true, Ordering::SeqCst);
    f.send(Fault::Happy);
    assert_eq!(f.settled()["state"], "echoed");
    f.app.tick(1792512000000);
    let start = Instant::now();
    post(
        &mut f.app,
        "/api/send",
        json!({"conversation_id":f.chat,"message_id":"second","body":"Autre question"}),
    );
    until(|| {
        f.app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == f.chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["id"] == "reply-second" && message["state"] == "echoed")
    });
    assert!(start.elapsed() < Duration::from_secs(6));
}

#[test]
fn title_socket_obeys_the_twelve_second_cap_not_the_session_stall() {
    // The session stall is 30s. A title is capped at 12s only if that cap
    // reaches the worker's read, not merely the caller wait.
    let mut f = Fixture::with_deadlines(
        true,
        Duration::from_secs(30),
        Duration::from_secs(30),
        Duration::from_secs(120),
    );
    f.fake.side_hang.store(true, Ordering::SeqCst);
    let mut chats = Vec::new();
    for n in 0..4 {
        let chat = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.fake.queue.lock().unwrap().push_back(Fault::Happy);
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":chat,"message_id":format!("title-cap-{n}"),"body":format!("Sujet distinct {n}")}),
        );
        chats.push(chat);
    }
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        chats.iter().all(|chat| {
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == *chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "assistant" && m["state"] == "echoed")
        })
    });
    let base = f.fake.holds_finished.load(Ordering::SeqCst);
    let started = Instant::now();
    let mut released = 0;
    while started.elapsed() < Duration::from_secs(16) && released < 4 {
        released = f
            .fake
            .holds_finished
            .load(Ordering::SeqCst)
            .saturating_sub(base);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        released >= 4 && started.elapsed() < Duration::from_secs(14),
        "title sockets still open after {:?} ({released} released, base {base}); the 12s cap never reached the worker",
        started.elapsed()
    );
}
#[test]
fn a_hanging_pre_description_never_blocks_the_main_reply() {
    let mut f = Fixture::new(true);
    f.fake.side_hang.store(true, Ordering::SeqCst);
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=").unwrap();
    let image = f.app.api("POST", "/api/attachments", &png).unwrap()["id"].clone();
    post(
        &mut f.app,
        "/api/attachments/precaption",
        json!({"conversation_id":f.chat,"attachment_id":image,"draft":"Décris"}),
    );
    until(|| !f.fake.seen.lock().unwrap().is_empty());
    let start = Instant::now();
    f.send(Fault::Happy);
    assert_eq!(f.settled()["state"], "echoed");
    assert!(start.elapsed() < Duration::from_secs(6));
}

#[test]
fn connection_test_is_bounded_and_reports_precise_error_then_ok() {
    let mut f = Fixture::new(true);
    f.fake.queue.lock().unwrap().push_back(Fault::Http(503));
    post(&mut f.app, "/api/provider/test", json!({}));
    until(|| f.provider.status()["connection_test"]["state"] == "failed");
    assert!(f.provider.status()["connection_test"]["message"]
        .as_str()
        .unwrap()
        .contains("indisponible"));
    post(&mut f.app, "/api/provider/test", json!({}));
    until(|| f.provider.status()["connection_test"]["state"] == "ok");
    let bodies = f.fake.seen.lock().unwrap();
    assert!(bodies.iter().all(|b| b.get("prompt_cache_key").is_none()
        && b.get("tools").is_none()
        && b.get("max_output_tokens").is_none()));
    drop(bodies);
    f.provider.signout().unwrap();
    assert_eq!(f.provider.status()["connection_test"]["state"], "idle");
}

#[test]
fn sharing_connection_probe_does_not_impose_a_sixteen_token_reasoning_limit() {
    let mut f = Fixture::new(false);
    std::fs::write(f.db.with_extension("chatgpt-auth.json"), json!({"host_id":"fake-only","client_id":"fake","access_token":"fake-sharing","refresh_token":"fake-refresh","expires_at":4102444800u64,"scopes":"chatgpt.tokens.use.direct","welcomed":true}).to_string()).unwrap();
    f.provider = Provider::with_endpoints(&f.db, f.fake.endpoints(), false);
    f.provider
        .configure_fake_chat(Duration::from_secs(10), false);
    f.app = App::open_with(&f.db, f.provider.clone()).unwrap();
    post(&mut f.app, "/api/provider/test", json!({}));
    until(|| f.provider.status()["connection_test"]["state"] == "ok");
    let requests = f.fake.seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("max_output_tokens").is_none());
    assert!(requests[0].get("reasoning").is_none());
}

#[test]
fn connection_probe_accepts_a_reasoning_token_cap_as_proof_of_reachability() {
    let mut f = Fixture::new(true);
    f.fake.queue.lock().unwrap().push_back(Fault::Incomplete);
    post(&mut f.app, "/api/provider/test", json!({}));
    until(|| f.provider.status()["connection_test"]["state"] == "ok");
    assert_eq!(
        f.provider.status()["connection_test"]["message"],
        "OK — ChatGPT répond."
    );
}

#[test]
fn signing_out_fences_an_inflight_connection_test_result() {
    let mut f = Fixture::with_deadline(true, Duration::from_secs(10));
    f.fake.queue.lock().unwrap().push_back(Fault::Partial);
    post(&mut f.app, "/api/provider/test", json!({}));
    until(|| !f.fake.seen.lock().unwrap().is_empty());
    assert_eq!(f.provider.status()["connection_test"]["state"], "running");
    f.provider.signout().unwrap();
    f.fake.partial_released.store(true, Ordering::SeqCst);
    until(|| f.fake.partial_finished.load(Ordering::SeqCst));
    // Wait for the receiver to consume the terminal event, not just the fake writer.
    until(|| {
        std::fs::read_to_string(f.db.parent().unwrap().join("logs/engine.log"))
            .unwrap_or_default()
            .contains("code=completed")
    });
    assert_eq!(f.provider.status()["connection_test"]["state"], "idle");
}

#[test]
fn connection_test_without_an_account_explains_the_required_action_without_network() {
    let mut f = Fixture::new(false);
    let result = post(&mut f.app, "/api/provider/test", json!({}));
    assert_eq!(
        result["provider"]["connection_test"]["state"],
        "unavailable"
    );
    assert_eq!(
        result["provider"]["connection_test"]["message"],
        "Aucun compte connecté. Connectez-vous via Codex ci-dessus."
    );
    assert!(f.fake.seen.lock().unwrap().is_empty());
}

#[test]
fn five_concurrent_chats_are_not_mistaken_for_abandoned_connections() {
    let mut f = Fixture::with_deadline(true, Duration::from_secs(10));
    let mut chats = vec![];
    for n in 0..5 {
        let chat = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.fake.queue.lock().unwrap().push_back(Fault::Partial);
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":chat,"message_id":format!("parallel-{n}"),"body":"Question simultanée"}),
        );
        chats.push(chat);
    }
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        chats.iter().all(|chat| {
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == *chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| {
                    m["role"] == "assistant"
                        && m["body"] == "Début conservé."
                        && m["state"] == "streaming"
                })
        })
    });
    f.fake.partial_released.store(true, Ordering::SeqCst);
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        chats.iter().all(|chat| {
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == *chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "assistant" && m["state"] == "echoed")
        })
    });
}

#[test]
fn four_abandoned_sockets_refuse_the_next_turn_until_they_close() {
    // Stall stays longer than this case. The read timeout must not close the
    // sockets before the immediate refusal is observed; the next byte does.
    let mut f = Fixture::with_deadline(true, Duration::from_secs(15));
    let mut held = vec![];
    for n in 0..4 {
        let chat = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let message = format!("hold-{n}");
        f.fake.queue.lock().unwrap().push_back(Fault::Partial);
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":chat,"message_id":message,"body":"Question en attente"}),
        );
        held.push((chat, message));
    }
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        held.iter().all(|(chat, message)| {
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == *chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == format!("reply-{message}") && m["body"] == "Début conservé.")
        })
    });
    for (chat, message) in &held {
        post(
            &mut f.app,
            "/api/stop",
            json!({"conversation_id":chat,"message_id":message}),
        );
    }
    let log_path = f.db.parent().unwrap().join("logs/engine.log");
    until(|| {
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .matches("code=cancelled")
            .count()
            >= 4
    });
    let blocked = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let started = Instant::now();
    post(
        &mut f.app,
        "/api/send",
        json!({"conversation_id":blocked,"message_id":"blocked","body":"Doit attendre la fermeture"}),
    );
    let mut refusal = Value::Null;
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        refusal = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == blocked)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reply-blocked")
            .cloned()
            .unwrap_or(Value::Null);
        !refusal.is_null() && refusal["state"] != "streaming"
    });
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "refusal must not wait for a network deadline: {refusal}"
    );
    assert_eq!(refusal["state"], "failed", "{refusal}");
    assert_eq!(
        refusal["error"],
        "Les connexions précédentes se ferment encore. Réessayez dans une minute."
    );
    f.fake.partial_released.store(true, Ordering::SeqCst);
    // The abandoned read ends on the next byte. Stall is 15s, so this wait
    // does not depend on the read timeout.
    std::thread::sleep(Duration::from_secs(3));
    let resumed = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.fake.queue.lock().unwrap().push_back(Fault::Happy);
    post(
        &mut f.app,
        "/api/send",
        json!({"conversation_id":resumed,"message_id":"after","body":"Nouvelle question"}),
    );
    let mut reply = Value::Null;
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        reply = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == resumed)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reply-after")
            .cloned()
            .unwrap_or(Value::Null);
        !reply.is_null() && reply["state"] != "streaming"
    });
    assert_eq!(reply["state"], "echoed", "{reply}");
}

#[test]
fn four_blocked_reads_accept_another_turn_once_the_read_timeout_elapses() {
    // ureq 2.12.1 lets `.timeout()` replace `.timeout_read()` on each body
    // read. The overall deadline stays at the production 10 minutes; only the
    // read timeout may release these abandoned sockets.
    let read = Duration::from_secs(3);
    let mut f = Fixture::with_deadlines(true, read, read, Duration::from_secs(600));
    let mut held = vec![];
    for n in 0..4 {
        let chat = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let message = format!("blocked-read-{n}");
        f.fake.queue.lock().unwrap().push_back(Fault::Partial);
        post(
            &mut f.app,
            "/api/send",
            json!({"conversation_id":chat,"message_id":message,"body":"Lecture bloquée"}),
        );
        held.push((chat, message));
    }
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        held.iter().all(|(chat, message)| {
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == *chat)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == format!("reply-{message}") && m["body"] == "Début conservé.")
        })
    });
    for (chat, message) in &held {
        post(
            &mut f.app,
            "/api/stop",
            json!({"conversation_id":chat,"message_id":message}),
        );
    }
    let blocked = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let refused_at = Instant::now();
    post(
        &mut f.app,
        "/api/send",
        json!({"conversation_id":blocked,"message_id":"still-closing","body":"Doit être refusé"}),
    );
    let mut refusal = Value::Null;
    until(|| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        refusal = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == blocked)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reply-still-closing")
            .cloned()
            .unwrap_or(Value::Null);
        !refusal.is_null() && refusal["state"] != "streaming"
    });
    assert!(
        refused_at.elapsed() < Duration::from_secs(2),
        "refusal must not wait for a network deadline: {refusal}"
    );
    assert_eq!(refusal["state"], "failed", "{refusal}");
    // The server never sends another byte. Later turns stay refused until
    // ureq's read timeout closes the sockets — well before the 10 minute
    // overall deadline.
    let recover = Instant::now();
    let mut attempt = 0u32;
    let mut chat = String::new();
    let mut message = String::new();
    let mut reply = Value::Null;
    loop {
        assert!(
            recover.elapsed() < Duration::from_secs(8),
            "fifth turn still blocked after {:?} (read timeout {read:?}, overall deadline 600s): {reply}",
            recover.elapsed()
        );
        let in_flight = !reply.is_null() && reply["state"] == "streaming";
        if !in_flight && reply["state"] != "echoed" {
            attempt += 1;
            chat = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            message = format!("after-read-timeout-{attempt}");
            f.fake.queue.lock().unwrap().push_back(Fault::Happy);
            post(
                &mut f.app,
                "/api/send",
                json!({"conversation_id":chat,"message_id":message,"body":"Nouvelle question"}),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        reply = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == chat)
            .and_then(|c| {
                c["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["id"] == format!("reply-{message}"))
            })
            .cloned()
            .unwrap_or(Value::Null);
        if reply["state"] == "echoed" {
            break;
        }
    }
    assert!(
        recover.elapsed() < Duration::from_secs(8),
        "recovery took {:?}, not the read timeout",
        recover.elapsed()
    );
}

#[test]
fn approved_tool_wait_has_a_card_and_denial_settles_without_execution() {
    let mut f = Fixture::new(true);
    post(
        &mut f.app,
        "/api/local/bot",
        json!({"bot_id":"grokbot","shell":true,"files":false}),
    );
    f.send(Fault::ToolApproval);
    let mut request = None;
    until(|| {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let run = f.reply()["payload"]["run_id"].as_str().unwrap().to_owned();
        request = grokbot_forms::requests::list_for_run(&conn, &run)
            .unwrap()
            .into_iter()
            .next();
        request.is_some() && f.provider.status()["titling"] == false
    });
    let request = request.unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"].to_string().contains("approval"));
    post(
        &mut f.app,
        "/api/approvals/resolve",
        json!({"entry_id":request.entry_id,"submission_id":"deny-fake","conversation_id":request.route.target_conversation_id,"decision":"deny"}),
    );
    until(|| {
        f.app.tick(1792512000000);
        f.reply()["state"] == "echoed"
    });
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let admissions: i64 = conn
        .query_row("SELECT COUNT(*) FROM engine_tool_admissions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(admissions, 0);
}

#[test]
fn restart_exposes_interruption_and_recovers_the_same_durable_run() {
    use grokbot_engine::{model::RunStatus, store};
    // This case tests lifecycle recovery, not network deadlines. Keep a wide
    // transport margin so contention cannot turn recovery into a timeout test.
    let mut f = Fixture::with_deadline(true, Duration::from_secs(10));
    f.send(Fault::Happy);
    let reply = f.settled();
    let old = reply["payload"]["run_id"].as_str().unwrap().to_owned();
    until(|| f.provider.status()["titling"] == false);
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let mut run = store::required(&conn, &old).unwrap();
    run.run_id = "restart-fixture".into();
    run.trigger = grokbot_engine::contracts::Trigger::UserMessage {
        message_id: format!("{}/question", f.chat),
        attempt: 1,
    };
    run.status = RunStatus::Queued;
    run.turn_end = None;
    let id = run.run_id.clone();
    let tx = conn.unchecked_transaction().unwrap();
    grokbot_engine::model::start(&tx, &run).unwrap();
    grokbot_engine::model::set_running_at(&tx, &id, run.started_at).unwrap();
    tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) SELECT ?1,input_json,live,model FROM wrapper_engine_runs WHERE run_id=?2", [&id,&old]).unwrap();
    tx.commit().unwrap();
    let restarted = Provider::with_backends(&f.db, f.fake.endpoints(), f.fake.endpoints(), false);
    restarted.configure_fake_chat(Duration::from_secs(10), false);
    f.app = App::open_with(&f.db, restarted.clone()).unwrap();
    assert_eq!(f.reply()["state"], "failed");
    assert!(f.reply()["error"].as_str().unwrap().contains("interrompue"));
    until(|| restarted.status()["models_ready"] == true);
    f.app.tick(1792512000000);
    // Observe the durable lifecycle, not an already-visible Interrupted card.
    // If recovery genuinely fails, print its persisted reason and events instead
    // of hiding it behind a generic polling timeout.
    until(|| store::required(&conn, &id).unwrap().status.is_terminal());
    let recovered = f.reply();
    assert_eq!(
        recovered["state"],
        "echoed",
        "reply={recovered}; events={:?}",
        store::events(&conn, &id, 0).unwrap()
    );
    assert_eq!(recovered["payload"]["run_id"], id);
}
