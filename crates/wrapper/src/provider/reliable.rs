//! Bounded foreground transport. The network worker never owns a database or a
//! UI callback. Dropping its receiver fences all late results, including a late
//! refresh: no new Responses request is dispatched after abandonment. ureq closes
//! the socket on its next read/deadline (it has no socket-abort API).
use super::*;
use grokbot_engine::contracts::{Error, ModelEvent, Result};
use std::cell::RefCell;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};

struct Flight {
    deadlines: Deadlines,
}
thread_local! {
    static FLIGHT: RefCell<Option<Flight>> = const { RefCell::new(None) };
}
struct FlightGuard;
impl Drop for FlightGuard {
    fn drop(&mut self) {
        FLIGHT.with(|slot| *slot.borrow_mut() = None);
    }
}
fn begin_flight(deadlines: Deadlines) -> FlightGuard {
    FLIGHT.with(|slot| *slot.borrow_mut() = Some(Flight { deadlines }));
    FlightGuard
}
/// Deadlines for the request running on this worker. Titles install a shorter
/// cap here; every other caller keeps the session deadlines.
pub(super) fn flight_deadlines(fallback: Deadlines) -> Deadlines {
    FLIGHT.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|flight| flight.deadlines)
            .unwrap_or(fallback)
    })
}

#[derive(Clone, Copy)]
pub(super) struct Deadlines {
    pub first: Duration,
    pub stall: Duration,
    pub total: Duration,
}
impl Default for Deadlines {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(25),
            stall: Duration::from_secs(60),
            total: Duration::from_secs(600),
        }
    }
}
enum Wire {
    Headers(u16),
    Progress,
    Notice(String),
    Failure(Failure),
    Event(ModelEvent),
}
struct Settled {
    worker_done: bool,
    counted: bool,
}
/// Caller side of one transport. Dropping it tells the worker to stop and, if
/// that worker is still inside a read ureq cannot abort, counts the socket as
/// closing until the worker actually exits.
struct Abandon {
    flag: Arc<AtomicBool>,
    inner: Arc<Inner>,
    settled: Arc<Mutex<Settled>>,
}
impl Drop for Abandon {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
        let mut settled = lock(&self.settled);
        if !settled.worker_done {
            settled.counted = true;
            self.inner.closing.fetch_add(1, Ordering::SeqCst);
        }
    }
}
struct Worker {
    inner: Arc<Inner>,
    settled: Arc<Mutex<Settled>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.inner.transports.fetch_sub(1, Ordering::SeqCst);
        let mut settled = lock(&self.settled);
        settled.worker_done = true;
        if settled.counted {
            self.inner.closing.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
fn send(tx: &SyncSender<Wire>, value: Wire) -> Result<()> {
    tx.send(value)
        .map_err(|_| Error::InvalidInput("transport abandoned".into()))
}
impl Session {
    pub(super) fn reliable_stream(
        &self,
        body: Value,
        cancel: &AtomicBool,
        turn: Option<&str>,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()> {
        let directory = self.inner.file.parent().unwrap_or(Path::new("."));
        let log = |status, code| {
            crate::upstream::turn(directory, turn.unwrap_or("internal"), status, code)
        };
        if cancel.load(Ordering::SeqCst) {
            return Err(Error::InvalidInput("run cancelled".into()));
        }
        // Live turns, titles and compaction stay admitted. Only sockets already
        // abandoned — and still blocked in a read — can pile up.
        if self.inner.closing.load(Ordering::SeqCst) >= 4 {
            let failure = Failure::other(
                "Les connexions précédentes se ferment encore. Réessayez dans une minute.",
            );
            log(0, "transport_busy");
            metadata(None, Some(&failure))?;
            return emit(ModelEvent::Failed {
                reason: failure.message,
            });
        }
        let _ = self.inner.transports.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::sync_channel(32);
        let abandoned = Arc::new(AtomicBool::new(false));
        let settled = Arc::new(Mutex::new(Settled {
            worker_done: false,
            counted: false,
        }));
        let session = self.clone();
        let worker_abandoned = abandoned.clone();
        let worker_settled = settled.clone();
        let mut limits = *lock(&self.inner.deadlines);
        if turn.is_none() {
            // Titles are best-effort bounded work, not another foreground turn.
            // Compaction carries its run identity and keeps normal turn deadlines.
            limits.first = limits.first.min(Duration::from_secs(10));
            limits.stall = limits.stall.min(Duration::from_secs(10));
            limits.total = limits.total.min(Duration::from_secs(12));
        }
        thread::spawn(move || {
            let _worker = Worker {
                inner: session.inner.clone(),
                settled: worker_settled,
            };
            // request() reads this cap. The caller loop alone would leave the
            // socket on the session stall (60s) after a title already gave up.
            let _flight = begin_flight(limits);
            let abandoned = worker_abandoned;
            let mut notices = Vec::new();
            let result = (|| -> Result<()> {
                let response = match session.responses_checked(
                    body,
                    true,
                    &mut notices,
                    &abandoned,
                    &mut |status| {
                        let _ = send(&tx, Wire::Headers(status));
                    },
                ) {
                    Ok(response) => response,
                    Err(failure) => {
                        send(&tx, Wire::Failure(failure.clone()))?;
                        return send(
                            &tx,
                            Wire::Event(ModelEvent::Failed {
                                reason: failure.message,
                            }),
                        );
                    }
                };
                if abandoned.load(Ordering::SeqCst) {
                    return Ok(());
                }
                for notice in notices {
                    send(&tx, Wire::Notice(notice))?;
                }
                let diagnostics = session.diagnostics(&response);
                let reader = Cancellable {
                    reader: BufReader::new(std::io::Read::take(
                        response.into_reader(),
                        32 * 1024 * 1024,
                    )),
                    cancel: &abandoned,
                };
                crate::gateway::stream_response_observed(
                    reader,
                    &diagnostics,
                    &mut |data| {
                        send(&tx, Wire::Progress)?;
                        if let Ok(event) = serde_json::from_str::<Value>(data) {
                            if event["type"] == "response.output_item.added"
                                && event["item"]["type"] == "function_call"
                            {
                                send(&tx, Wire::Notice("Préparation d’un outil par OpenAI… Aucune action n’a encore été exécutée.".into()))?;
                            }
                        }
                        Ok(())
                    },
                    &mut |event| {
                        if matches!(event, ModelEvent::Incomplete { .. }) {
                            send(
                                &tx,
                                Wire::Failure(Failure {
                                    message: "OpenAI a arrêté cette réponse avant la fin.".into(),
                                    kind: "incomplete",
                                }),
                            )?;
                        }
                        if let ModelEvent::Failed { reason } = &event {
                            send(
                                &tx,
                                Wire::Failure(Failure {
                                    message: reason.clone(),
                                    kind: diagnostics.failure_kind(),
                                }),
                            )?;
                        }
                        send(&tx, Wire::Event(event))
                    },
                )
            })();
            let _ = result; // Receiver is the sole persistence owner; never log content/errors here.
        });
        let _abandon = Abandon {
            flag: abandoned.clone(),
            inner: self.inner.clone(),
            settled: settled.clone(),
        };
        log(0, "started");
        let started = Instant::now();
        let mut last = started;
        let mut first = false;
        let mut http = 0;
        let mut tool_notice = false;
        let mut previous_notice = String::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                // ureq cannot abort the read it is already inside. Interrupted
                // keeps only the checkpoint taken when this call returns, so
                // leaving now would drop the rest of that interval. Count the
                // socket immediately: admission must see it while we wait.
                abandoned.store(true, Ordering::SeqCst);
                {
                    let mut guard = lock(&settled);
                    if !guard.worker_done && !guard.counted {
                        guard.counted = true;
                        self.inner.closing.fetch_add(1, Ordering::SeqCst);
                    }
                }
                log(http, "cancelled");
                loop {
                    if lock(&settled).worker_done {
                        break;
                    }
                    match rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(_) => {}
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                if tool_notice {
                    metadata(Some(&previous_notice), None)?;
                }
                return Err(Error::InvalidInput("run cancelled".into()));
            }
            let timeout = if started.elapsed() >= limits.total {
                Some((
                    "turn_timeout",
                    "La réponse a dépassé le délai maximal. Réessayez.",
                ))
            } else if last.elapsed() >= if first { limits.stall } else { limits.first } {
                Some(if first {
                    (
                        "stream_stalled",
                        "La réponse d’OpenAI est bloquée : aucun nouvel événement reçu. Réessayez.",
                    )
                } else {
                    ("first_byte_timeout", "OpenAI n’a pas répondu à temps (connexion ou renouvellement de session). Réessayez.")
                })
            } else {
                None
            };
            if let Some((code, message)) = timeout {
                log(http, code);
                let failure = Failure::other(message);
                metadata(
                    if tool_notice {
                        Some(&previous_notice)
                    } else {
                        None
                    },
                    Some(&failure),
                )?;
                return emit(ModelEvent::Failed {
                    reason: message.into(),
                });
            }
            match rx.recv_timeout(Duration::from_millis(25)) {
                Ok(Wire::Headers(status)) => {
                    http = status;
                    log(http, "headers");
                }
                Ok(Wire::Progress) => {
                    first = true;
                    last = Instant::now();
                }
                Ok(Wire::Notice(notice)) => {
                    tool_notice = notice.starts_with("Préparation d’un outil");
                    if !tool_notice {
                        previous_notice = notice.clone();
                    }
                    metadata(Some(&notice), None)?;
                }
                Ok(Wire::Failure(failure)) => metadata(None, Some(&failure))?,
                Ok(Wire::Event(event)) => {
                    let terminal = match &event {
                        ModelEvent::Completed => Some("completed"),
                        ModelEvent::Failed { .. } => Some("failed"),
                        ModelEvent::Incomplete { .. } => Some("incomplete"),
                        _ => None,
                    };
                    if let Some(code) = terminal {
                        log(http, code);
                        if tool_notice {
                            metadata(Some(&previous_notice), None)?;
                        }
                    }
                    emit(event)?;
                    if terminal.is_some() {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    log(http, "transport_closed");
                    let failure =
                        Failure::other("La connexion à OpenAI a été interrompue. Réessayez.");
                    metadata(
                        if tool_notice {
                            Some(&previous_notice)
                        } else {
                            None
                        },
                        Some(&failure),
                    )?;
                    return emit(ModelEvent::Failed {
                        reason: failure.message,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod elapsed_interval {
    use crate::{App, Endpoints, Provider};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };

    const BLOCKED_READ: Duration = Duration::from_millis(800);

    /// Portable stand-in for the macOS routine pass: cancel while ureq is blocked
    /// in the response read, then require that checkpoint to keep the interval.
    #[test]
    fn interrupt_checkpoints_the_blocked_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let started = Arc::new(AtomicBool::new(false));
        let flag = started.clone();
        thread::spawn(move || {
            for _ in 0..24 {
                let Ok((mut socket, _)) = listener.accept() else {
                    break;
                };
                let flag = flag.clone();
                thread::spawn(move || respond(&mut socket, &flag));
            }
        });
        let root = std::env::temp_dir().join(format!(
            "ezerbot-elapsed-{}-{}",
            std::process::id(),
            crate::now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("app.sqlite");
        let base = format!("http://{address}");
        std::fs::write(
            root.join("app.codex-auth.json"),
            serde_json::json!({
                "host_id": "elapsed",
                "account_id": "elapsed",
                "client_id": "elapsed",
                "access_token": token(),
                "refresh_token": "elapsed-refresh",
                "expires_at": 4_102_444_800u64,
                "scopes": "openid offline_access",
                "welcomed": true
            })
            .to_string(),
        )
        .unwrap();
        let endpoints = Endpoints {
            api: format!("{base}/backend-api/codex"),
            token: format!("{base}/token"),
            ..Endpoints::default()
        };
        let provider = Provider::with_backends(&db, endpoints.clone(), endpoints, false);
        let mut app = App::open_with(&db, provider.clone()).unwrap();
        let ready = Instant::now();
        while app.api("GET", "/api/state", b"").unwrap()["provider"]["models_ready"] != true {
            assert!(
                ready.elapsed() < Duration::from_secs(10),
                "codex catalogue did not load"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let chat = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        app.api(
            "POST",
            "/api/send",
            serde_json::json!({
                "conversation_id": chat,
                "message_id": "question",
                "body": "Bilan de test"
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let marked = Instant::now();
        while !started.load(Ordering::SeqCst) {
            assert!(
                marked.elapsed() < Duration::from_secs(10),
                "the blocked read never started"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let blocked = Instant::now();
        provider.interrupt_runs();
        while provider.has_run_workers() {
            assert!(
                blocked.elapsed() < Duration::from_secs(10),
                "the interrupted worker did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        let (status, elapsed): (String, i64) = conn
            .query_row(
                "SELECT status, json_extract(usage,'$.elapsed_ms') FROM engine_runs WHERE trigger_kind='user_message'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "interrupted");
        assert!(
            elapsed >= 700 && blocked.elapsed() >= Duration::from_millis(700),
            "elapsed interval lost: {elapsed}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn respond(socket: &mut std::net::TcpStream, started: &AtomicBool) {
        socket.set_nonblocking(false).unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
        let mut length = 0usize;
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0; length];
        if length > 0 && reader.read_exact(&mut body).is_err() {
            return;
        }
        let instructions = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value["instructions"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let user_turn = path.contains("/responses")
            && !instructions.contains("titre")
            && !instructions.starts_with("Extraction");
        if user_turn {
            started.store(true, Ordering::SeqCst);
            thread::sleep(BLOCKED_READ);
        }
        let payload = if path.contains("/responses") {
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Bilan.\"}\n\ndata: {\"type\":\"response.completed\"}\n\n".to_owned()
        } else {
            "{}".to_owned()
        };
        let content = if path.contains("/responses") {
            "text/event-stream"
        } else {
            "application/json"
        };
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: {content}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
            payload.len()
        );
    }

    fn token() -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(
                serde_json::json!({"https://api.openai.com/auth":{"chatgpt_account_id":"elapsed"}})
                    .to_string()
            )
        )
    }
}
