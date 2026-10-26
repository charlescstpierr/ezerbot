mod support;

use grokbot_engine::{contracts::RunEventKind, model::RunStatus, store};
use grokbot_wrapper::{App, Endpoints, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, CLIENT};

enum Frame {
    Progress,
    Complete,
}

struct StreamServer {
    base: String,
    frames: Option<Sender<Frame>>,
    opened: Receiver<()>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl StreamServer {
    fn new() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let (frames, receive) = mpsc::channel();
        let (opened, ready) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            let mut held = false;
            while !stopping.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                if request.url() == "/v1/models" {
                    let _ = request.respond(tiny_http::Response::from_string(
                        json!({"models":[{"slug":"gpt-test","visibility":"list"}]}).to_string(),
                    ));
                    continue;
                }
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                if held || !body.contains("transport-watchdog-fixture") {
                    let _ = request.respond(tiny_http::Response::from_string(
                        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n",
                    ));
                    continue;
                }
                held = true;
                let mut writer: Box<dyn Write + Send> = request.into_writer();
                if writer
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    return;
                }
                let _ = opened.send(());
                while let Ok(frame) = receive.recv() {
                    let (payload, complete) = match frame {
                        Frame::Progress => (
                            "data: {\"type\":\"response.in_progress\"}\n\n",
                            false,
                        ),
                        Frame::Complete => (
                            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Terminé.\"}\n\ndata: {\"type\":\"response.completed\"}\n\n",
                            true,
                        ),
                    };
                    if writer
                        .write_all(payload.as_bytes())
                        .and_then(|()| writer.flush())
                        .is_err()
                        || complete
                    {
                        break;
                    }
                }
            }
        });
        Self {
            base,
            frames: Some(frames),
            opened: ready,
            stop,
            worker: Some(worker),
        }
    }

    fn send(&self, frame: Frame) {
        self.frames.as_ref().unwrap().send(frame).unwrap();
    }

    fn endpoints(&self) -> Endpoints {
        let base = &self.base;
        Endpoints {
            authorize: format!("{base}/authorize"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            jwks: format!("{base}/jwks"),
            issuer: base.clone(),
            api: format!("{base}/v1"),
        }
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.frames.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Fixture {
    server: StreamServer,
    app: App,
    provider: Provider,
    connection: Connection,
    _dir: grokbot_test_support::TestDir,
}

#[test]
fn ignored_sse_event_clears_silence_before_any_model_event_is_committed() {
    let dir = grokbot_test_support::TestDir::new("watchdog-stream");
    let path = dir.join("workspace.sqlite");
    let server = StreamServer::new();
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::fs::write(
        path.with_extension("chatgpt-auth.json"),
        json!({"host_id":"test-host","client_id":CLIENT,"access_token":"access-2","refresh_token":"refresh-2","expires_at":start / 1000 + 3600,"scopes":"chatgpt.tokens.use.direct"}).to_string(),
    )
    .unwrap();
    let provider = Provider::with_endpoints(&path, server.endpoints(), false);
    provider.watchdog_test_clock(start);
    provider.refresh_models();
    wait("loopback catalogue", || {
        provider.status()["models_ready"] == true
    });
    let app = App::open_with(&path, provider.clone()).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection.busy_timeout(Duration::from_secs(3)).unwrap();
    let mut f = Fixture {
        server,
        app,
        provider,
        connection,
        _dir: dir,
    };
    let conversation = f.app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.app
        .api(
            "POST",
            "/api/send",
            &serde_json::to_vec(&json!({
                "conversation_id":conversation,"message_id":"stream","body":"transport-watchdog-fixture"
            }))
            .unwrap(),
        )
        .unwrap();
    f.server
        .opened
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    let run_id: String = f
        .connection
        .query_row(
            "SELECT run_id FROM engine_runs WHERE conversation_id=?1 AND status='running'",
            [&conversation],
            |row| row.get(0),
        )
        .unwrap();
    let before = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(before["silence"], json!({}));
    f.provider.watchdog_test_clock(start + 240_000);
    f.app.tick(start + 240_000);
    let silent = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(silent["silence"][&run_id], start);
    assert_ne!(silent["state_revision"], before["state_revision"]);
    assert_eq!(
        f.app.api("GET", "/api/ping", b"").unwrap()["revision"],
        silent["state_revision"]
    );
    let events_before = store::events(&f.connection, &run_id, 0).unwrap().len();
    f.provider.watchdog_test_clock(start + 241_000);
    f.server.send(Frame::Progress);
    let mut active = Value::Null;
    wait(
        "ignored SSE clears silence while the stream stays open",
        || {
            active = f.app.api("GET", "/api/state", b"").unwrap();
            active["silence"] == json!({})
        },
    );
    assert_ne!(active["state_revision"], silent["state_revision"]);
    assert_eq!(
        f.app.api("GET", "/api/ping", b"").unwrap()["revision"],
        active["state_revision"]
    );
    assert_eq!(
        store::required(&f.connection, &run_id).unwrap().status,
        RunStatus::Running
    );
    let events = store::events(&f.connection, &run_id, 0).unwrap();
    assert_eq!(events.len(), events_before);
    assert!(!events
        .iter()
        .any(|event| event.kind == RunEventKind::TextDelta));
    f.server.send(Frame::Complete);
    wait("loopback stream completes", || {
        store::required(&f.connection, &run_id)
            .unwrap()
            .status
            .is_terminal()
    });
    assert_eq!(
        store::required(&f.connection, &run_id).unwrap().status,
        RunStatus::Finished
    );
    f.provider.wait_for_background_work().unwrap();
}
