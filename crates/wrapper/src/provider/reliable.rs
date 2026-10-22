//! Bounded foreground transport. The network worker never owns a database or a
//! UI callback. Dropping its receiver fences all late results, including a late
//! refresh: no new Responses request is dispatched after abandonment. ureq closes
//! the socket on its next read/deadline (it has no socket-abort API).
use super::*;
use grokbot_engine::contracts::{Error, ModelEvent, Result};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};

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
pub(super) struct Settled {
    pub(super) worker_done: bool,
    pub(super) counted: bool,
}
struct LiveRead {
    inner: Arc<Inner>,
    id: Option<String>,
}
impl Drop for LiveRead {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            lock(&self.inner.live_reads).remove(&id);
        }
    }
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
impl Worker {
    /// Declared before the last send: once the caller holds the terminal event no
    /// read is left to close, so dropping the caller must not count this socket.
    fn finish(&self) {
        let mut settled = lock(&self.settled);
        if settled.worker_done {
            return;
        }
        settled.worker_done = true;
        if settled.counted {
            self.inner.closing.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.finish();
    }
}
fn send(tx: &SyncSender<Wire>, value: Wire) -> Result<()> {
    tx.send(value)
        .map_err(|_| Error::InvalidInput("transport abandoned".into()))
}
impl Session {
    /// Compte le socket dès l'annulation, sans attendre que le fil de lecture se réveille.
    pub(super) fn account_live_cancel(&self, id: &str) {
        let Some(settled) = lock(&self.inner.live_reads).get(id).cloned() else {
            return;
        };
        let mut settled = lock(&settled);
        if !settled.worker_done && !settled.counted {
            settled.counted = true;
            self.inner.closing.fetch_add(1, Ordering::SeqCst);
        }
    }
    pub(super) fn account_all_live(&self) {
        let ids: Vec<String> = lock(&self.inner.live_reads).keys().cloned().collect();
        for id in ids {
            self.account_live_cancel(&id);
        }
    }
    #[cfg(test)]
    pub(crate) fn pause_turn(&self, id: &str) {
        lock(&self.inner.paused_turns).insert(id.to_owned());
    }
    #[cfg(test)]
    pub(crate) fn resume_turns(&self) {
        lock(&self.inner.paused_turns).clear();
    }
    #[cfg(test)]
    pub(crate) fn hold_worker_exits(&self, hold: bool) {
        self.inner.held_exits.store(hold, Ordering::SeqCst);
    }
    #[cfg(test)]
    pub(crate) fn closing_sockets(&self) -> usize {
        self.inner.closing.load(Ordering::SeqCst)
    }
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
        let (tx, rx) = mpsc::sync_channel(32);
        let abandoned = Arc::new(AtomicBool::new(false));
        let settled = Arc::new(Mutex::new(Settled {
            worker_done: false,
            counted: false,
        }));
        if let Some(id) = turn {
            lock(&self.inner.live_reads).insert(id.to_owned(), settled.clone());
        }
        let _live = LiveRead {
            inner: self.inner.clone(),
            id: turn.map(str::to_owned),
        };
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
        let within = turn.is_none().then_some(limits.total);
        thread::spawn(move || {
            let worker = Worker {
                inner: session.inner.clone(),
                settled: worker_settled,
            };
            let abandoned = worker_abandoned;
            let mut notices = Vec::new();
            let result = (|| -> Result<()> {
                let response = match session.responses_checked(
                    body,
                    true,
                    &mut notices,
                    &abandoned,
                    within,
                    &mut |status| {
                        let _ = send(&tx, Wire::Headers(status));
                    },
                ) {
                    Ok(response) => response,
                    Err(failure) => {
                        send(&tx, Wire::Failure(failure.clone()))?;
                        worker.finish();
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
                        if matches!(
                            event,
                            ModelEvent::Completed
                                | ModelEvent::Failed { .. }
                                | ModelEvent::Incomplete { .. }
                        ) {
                            worker.finish();
                        }
                        send(&tx, Wire::Event(event))
                    },
                )
            })();
            let _ = result; // Receiver is the sole persistence owner; never log content/errors here.
            #[cfg(test)]
            while session.inner.held_exits.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
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
            #[cfg(test)]
            if turn.is_some_and(|id| lock(&self.inner.paused_turns).contains(id)) {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
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
    use super::Session;
    use crate::{App, Endpoints, Provider};
    use grokbot_engine::contracts::ModelEvent;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
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

    /// A transport that delivered its last event has nothing left to close: a burst
    /// of finished turns must never be counted as sockets still shutting down.
    #[test]
    fn finished_workers_are_not_counted_as_closing_sockets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for _ in 0..24 {
                let Ok((mut socket, _)) = listener.accept() else {
                    break;
                };
                thread::spawn(move || respond(&mut socket, &AtomicBool::new(false)));
            }
        });
        let root = std::env::temp_dir().join(format!(
            "ezerbot-burst-{}-{}",
            std::process::id(),
            crate::now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("app.sqlite");
        std::fs::write(
            root.join("app.codex-auth.json"),
            serde_json::json!({
                "host_id": "burst",
                "account_id": "burst",
                "client_id": "burst",
                "access_token": token(),
                "refresh_token": "burst-refresh",
                "expires_at": 4_102_444_800u64,
                "scopes": "openid offline_access",
                "welcomed": true
            })
            .to_string(),
        )
        .unwrap();
        let base = format!("http://{address}");
        let endpoints = Endpoints {
            api: format!("{base}/backend-api/codex"),
            token: format!("{base}/token"),
            ..Endpoints::default()
        };
        let session = Session::with_endpoints(&db, endpoints, false, true);
        session.hold_worker_exits(true);
        let mut peak = 0;
        for burst in 0..8 {
            let mut events = Vec::new();
            session
                .reliable_stream(
                    serde_json::json!({"model": "gpt-test", "instructions": "Écris un titre", "input": []}),
                    &AtomicBool::new(false),
                    None,
                    &mut |_, _| Ok(()),
                    &mut |event| {
                        events.push(event);
                        Ok(())
                    },
                )
                .unwrap();
            peak = peak.max(session.closing_sockets());
            assert!(
                matches!(events.last(), Some(ModelEvent::Completed)),
                "turn {burst} was refused or failed: {events:?}"
            );
        }
        session.hold_worker_exits(false);
        assert!(peak <= 4, "closing peaked at {peak}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// The title cap bounds the request itself, not only the caller's wait: a
    /// server that never answers loses its connection when the cap expires.
    #[test]
    fn a_slow_title_request_is_dropped_within_its_cap() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (closed_tx, closed) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            socket
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
            let mut byte = [0u8; 1];
            loop {
                match socket.read(&mut byte) {
                    Ok(0) => break,
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Ok(_) | Err(_) => break,
                }
            }
            let _ = closed_tx.send(Instant::now());
        });
        let root = std::env::temp_dir().join(format!(
            "ezerbot-title-cap-{}-{}",
            std::process::id(),
            crate::now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("app.sqlite");
        std::fs::write(
            root.join("app.codex-auth.json"),
            serde_json::json!({
                "host_id": "cap",
                "account_id": "cap",
                "client_id": "cap",
                "access_token": token(),
                "refresh_token": "cap-refresh",
                "expires_at": 4_102_444_800u64,
                "scopes": "openid offline_access",
                "welcomed": true
            })
            .to_string(),
        )
        .unwrap();
        let base = format!("http://{address}");
        let endpoints = Endpoints {
            api: format!("{base}/backend-api/codex"),
            token: format!("{base}/token"),
            ..Endpoints::default()
        };
        let session = Session::with_endpoints(&db, endpoints, false, true);
        *super::lock(&session.inner.deadlines) = super::Deadlines {
            first: Duration::from_secs(30),
            stall: Duration::from_secs(30),
            total: Duration::from_secs(2),
        };
        let started = Instant::now();
        let mut events = Vec::new();
        session
            .reliable_stream(
                serde_json::json!({"model": "gpt-test", "instructions": "Écris un titre", "input": []}),
                &AtomicBool::new(false),
                None,
                &mut |_, _| Ok(()),
                &mut |event| {
                    events.push(event);
                    Ok(())
                },
            )
            .unwrap();
        assert!(matches!(events.last(), Some(ModelEvent::Failed { .. })));
        let dropped = closed
            .recv_timeout(Duration::from_secs(6))
            .expect("the title request was still open after its cap");
        assert!(
            dropped.duration_since(started) <= Duration::from_secs(4),
            "the request outlived the title cap by {:?}",
            dropped.duration_since(started)
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
            && !instructions.starts_with("Extraction")
            && !instructions.starts_with("Test de connexion.");
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

    #[test]
    fn cancel_refuses_the_next_turn_before_the_reader_thread_wakes() {
        let release = Arc::new(AtomicBool::new(false));
        let chats = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (flag, count) = (release.clone(), chats.clone());
        thread::spawn(move || {
            for _ in 0..32 {
                let Ok((mut socket, _)) = listener.accept() else {
                    break;
                };
                let (flag, count) = (flag.clone(), count.clone());
                thread::spawn(move || hold_chat(&mut socket, &flag, &count));
            }
        });
        let root = std::env::temp_dir().join(format!(
            "ezerbot-cancel-gate-{}-{}",
            std::process::id(),
            crate::now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db = root.join("app.sqlite");
        let base = format!("http://{address}");
        std::fs::write(
            root.join("app.codex-auth.json"),
            serde_json::json!({
                "host_id": "gate",
                "account_id": "gate",
                "client_id": "gate",
                "access_token": token(),
                "refresh_token": "gate-refresh",
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
        provider.configure_fake_deadlines(
            Duration::from_secs(3),
            Duration::from_secs(3),
            Duration::from_secs(30),
        );
        let mut app = App::open_with(&db, provider.clone()).unwrap();
        let ready = Instant::now();
        while app.api("GET", "/api/state", b"").unwrap()["provider"]["connected"] != true {
            assert!(
                ready.elapsed() < Duration::from_secs(10),
                "probe did not succeed"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let mut held = vec![];
        for n in 0..4 {
            let chat = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let message = format!("blocked-{n}");
            app.api(
                "POST",
                "/api/send",
                serde_json::json!({"conversation_id": chat, "message_id": message, "body": "Lecture bloquée"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
            held.push((chat, message));
        }
        let parked = Instant::now();
        let mut run_ids = vec![];
        while run_ids.len() < 4 {
            assert!(
                parked.elapsed() < Duration::from_secs(10),
                "partial deltas did not arrive"
            );
            run_ids.clear();
            for (chat, message) in &held {
                let reply = conversation_message(&mut app, chat, &format!("reply-{message}"));
                if reply["body"] == "Début conservé." {
                    run_ids.push(reply["payload"]["run_id"].as_str().unwrap().to_owned());
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        for id in &run_ids {
            provider.pause_turn(id);
        }
        thread::sleep(Duration::from_millis(50));
        for (chat, message) in &held {
            app.api(
                "POST",
                "/api/stop",
                serde_json::json!({"conversation_id": chat, "message_id": message})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        }
        let seen = chats.load(Ordering::SeqCst);
        let blocked = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let started = Instant::now();
        app.api(
            "POST",
            "/api/send",
            serde_json::json!({"conversation_id": blocked, "message_id": "still-closing", "body": "Doit être refusé"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let mut refusal = serde_json::Value::Null;
        while refusal["state"] == "streaming" || refusal.is_null() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "refusal did not settle: {refusal}"
            );
            refusal = conversation_message(&mut app, &blocked, "reply-still-closing");
            thread::sleep(Duration::from_millis(10));
        }
        provider.resume_turns();
        release.store(true, Ordering::SeqCst);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "refusal waited on the network: {refusal}"
        );
        assert_eq!(refusal["state"], "failed", "{refusal}");
        assert_eq!(
            refusal["error"],
            "Les connexions précédentes se ferment encore. Réessayez dans une minute."
        );
        assert_eq!(
            chats.load(Ordering::SeqCst),
            seen,
            "refused turn reached the model"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn conversation_message(app: &mut App, chat: &str, id: &str) -> serde_json::Value {
        app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["id"] == chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["id"] == id)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    fn hold_chat(socket: &mut std::net::TcpStream, release: &AtomicBool, chats: &AtomicUsize) {
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
        if !path.contains("/responses") {
            let payload = if path.ends_with("/token") {
                serde_json::json!({"access_token": token(), "refresh_token": "gate-refresh", "expires_in": 3600}).to_string()
            } else {
                "{}".to_owned()
            };
            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
            return;
        }
        let instructions = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value["instructions"].as_str().map(str::to_owned))
            .unwrap_or_default();
        let internal = instructions.starts_with("Test de connexion.")
            || instructions.contains("titre")
            || instructions.starts_with("Extraction");
        let payload = if internal {
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n"
        } else if chats.fetch_add(1, Ordering::SeqCst) >= 4 {
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Réponse factice terminée.\"}\n\ndata: {\"type\":\"response.completed\"}\n\n"
        } else {
            let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"Début conservé.\"}}\n\n");
            let _ = socket.flush();
            while !release.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(10));
            }
            return;
        };
        let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
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
