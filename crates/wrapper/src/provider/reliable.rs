//! Bounded foreground transport. The network worker never owns a database or a
//! UI callback. Dropping its receiver fences all late results, including a late
//! refresh: no new Responses request is dispatched after abandonment. ureq closes
//! the socket on its next read/deadline (it has no socket-abort API).
use super::*;
use grokbot_engine::contracts::{Error, ModelEvent, Result};
use std::cell::RefCell;
use std::io::{ErrorKind, Read};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};

/// Only a finished SSE `data:` event refreshes the read deadline. A trickle of
/// raw octets, or a comment line such as `: ping`, must not.
const SSE_PENDING_CAP: usize = 64 * 1024;

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

/// Local relay for the worker that is inside `request()`. ureq has no abort,
/// and `.timeout()` would replace `.timeout_read()` on every body read. The
/// relay closes the sockets at `stall` unless a real burst arrived, and always
/// at `total`. Abandon does not close early: Stop must keep the closing slot
/// until the read deadline, and the HTTP handler must return in under a second.
pub(super) fn spawn_deadline_proxy() -> Option<std::io::Result<String>> {
    let limits = FLIGHT.with(|slot| slot.borrow().as_ref().map(|flight| flight.deadlines))?;
    Some((|| {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        thread::spawn(move || accept_relay(listener, limits));
        Ok(addr.to_string())
    })())
}

fn accept_relay(listener: TcpListener, limits: Deadlines) {
    let started = Instant::now();
    let mut served = 0u8;
    let mut idle = Instant::now();
    while served < 3 && started.elapsed() < limits.total.saturating_add(Duration::from_secs(2)) {
        match listener.accept() {
            Ok((sock, _)) => {
                served = served.saturating_add(1);
                let _ = sock.set_nonblocking(false);
                let _ = serve_relay(sock, limits);
                idle = Instant::now();
            }
            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                if served > 0 && idle.elapsed() > Duration::from_millis(300) {
                    break;
                }
                if served == 0 && started.elapsed() > Duration::from_secs(20) {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
}

fn serve_relay(mut client: TcpStream, limits: Deadlines) -> std::io::Result<()> {
    let started = Instant::now();
    let _ = client.set_nodelay(true);
    // ureq writes the whole body before it reads a response. Take that body
    // before dialing upstream: leaving it unread while the origin is polled
    // makes the client reset, and the origin then sees a truncated JSON body.
    let (head, body) = read_request(&mut client)?;
    let target = parse_target(&head)?;
    let mut upstream = connect_host(&target.host, target.port)?;
    if target.tunnel {
        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    } else {
        upstream.write_all(&target.prelude)?;
        if !body.is_empty() {
            upstream.write_all(&body)?;
        }
    }
    pump(client, upstream, limits, started)
}

fn read_request(sock: &mut TcpStream) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    sock.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let header_end = loop {
        if data.len() > 64 * 1024 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "en-tête HTTP trop long",
            ));
        }
        let n = read_some(sock, &mut buf)?;
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|mark| mark == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let need = content_length(&data[..header_end]);
    while data.len() - header_end < need {
        if data.len() > 32 * 1024 * 1024 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "corps HTTP trop long",
            ));
        }
        let n = read_some(sock, &mut buf)?;
        data.extend_from_slice(&buf[..n]);
    }
    let body = data.split_off(header_end);
    Ok((data, body))
}

fn read_some(sock: &mut TcpStream, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match sock.read(buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "requête relais interrompue",
                ));
            }
            Ok(n) => return Ok(n),
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

fn content_length(head: &[u8]) -> usize {
    let text = String::from_utf8_lossy(head);
    text.split("\r\n")
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0)
}

struct RelayTarget {
    host: String,
    port: u16,
    prelude: Vec<u8>,
    tunnel: bool,
}

fn parse_target(head: &[u8]) -> std::io::Result<RelayTarget> {
    let text = String::from_utf8_lossy(head);
    let request = text.split("\r\n").next().unwrap_or("");
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let invalid = || std::io::Error::new(ErrorKind::InvalidData, "requête relais illisible");
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = target.rsplit_once(':').ok_or_else(invalid)?;
        return Ok(RelayTarget {
            host: host.to_owned(),
            port: port.parse().map_err(|_| invalid())?,
            prelude: Vec::new(),
            tunnel: true,
        });
    }
    let url = url::Url::parse(target).map_err(|_| invalid())?;
    let host = url.host_str().ok_or_else(invalid)?.to_owned();
    let port = url.port_or_known_default().ok_or_else(invalid)?;
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    let mut prelude = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
    if let Some(split) = head.windows(2).position(|mark| mark == b"\r\n") {
        prelude.extend_from_slice(&head[split + 2..]);
    }
    Ok(RelayTarget {
        host,
        port,
        prelude,
        tunnel: false,
    })
}

const CONNECT_ATTEMPT: Duration = Duration::from_secs(5);
const CONNECT_BUDGET: Duration = Duration::from_secs(15);

fn connect_host(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let addrs = (host, port).to_socket_addrs()?.collect::<Vec<_>>();
    connect_addrs(&addrs, CONNECT_ATTEMPT, CONNECT_BUDGET)
}

/// Tries every resolved address. One unreachable AAAA must not skip the IPv4
/// that follows. Each attempt is capped, and the whole dial stays inside `budget`.
fn connect_addrs(
    addrs: &[SocketAddr],
    per_attempt: Duration,
    budget: Duration,
) -> std::io::Result<TcpStream> {
    if addrs.is_empty() {
        return Err(std::io::Error::new(ErrorKind::NotFound, "hôte introuvable"));
    }
    let started = Instant::now();
    let mut last = std::io::Error::new(ErrorKind::NotFound, "hôte introuvable");
    for addr in addrs {
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        // A lone address keeps the whole budget. Several addresses share it,
        // so a blackholed AAAA cannot spend the 15s before IPv4 is tried.
        let timeout = if addrs.len() == 1 {
            remaining
        } else {
            per_attempt.min(remaining)
        };
        if timeout.is_zero() {
            break;
        }
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Err(err) => last = err,
        }
    }
    Err(last)
}

fn pump(
    mut client: TcpStream,
    mut upstream: TcpStream,
    limits: Deadlines,
    started: Instant,
) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_millis(200)))?;
    upstream.set_read_timeout(Some(Duration::from_millis(200)))?;
    client.set_write_timeout(Some(Duration::from_secs(15)))?;
    upstream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut last = Instant::now();
    let mut pending = Vec::new();
    let mut client_open = true;
    let mut buf = [0u8; 8192];
    let result = loop {
        if started.elapsed() >= limits.total || last.elapsed() >= limits.stall {
            break Ok(());
        }
        // The origin closing ends the response. The client closing does not:
        // the request body is already forwarded, and the reply may still flow.
        if transfer(
            &mut upstream,
            &mut client,
            &mut buf,
            true,
            &mut pending,
            &mut last,
        )? {
            break Ok(());
        }
        if client_open
            && transfer(
                &mut client,
                &mut upstream,
                &mut buf,
                false,
                &mut pending,
                &mut last,
            )?
        {
            client_open = false;
        }
    };
    let _ = client.shutdown(Shutdown::Both);
    let _ = upstream.shutdown(Shutdown::Both);
    result
}

fn transfer(
    from: &mut TcpStream,
    to: &mut TcpStream,
    buf: &mut [u8],
    count_events: bool,
    pending: &mut Vec<u8>,
    last: &mut Instant,
) -> std::io::Result<bool> {
    match from.read(buf) {
        Ok(0) => Ok(true),
        Ok(n) => {
            to.write_all(&buf[..n])?;
            if count_events && sse_data_progress(pending, &buf[..n]) {
                *last = Instant::now();
            }
            Ok(false)
        }
        Err(err)
            if matches!(
                err.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

/// True when `chunk` completes an SSE event whose block contains a `data:` line.
fn sse_data_progress(pending: &mut Vec<u8>, chunk: &[u8]) -> bool {
    pending.extend_from_slice(chunk);
    let mut progressed = false;
    while let Some(end) = sse_boundary(pending) {
        let event: Vec<u8> = pending.drain(..end).collect();
        if event_has_data_field(&event) {
            progressed = true;
        }
    }
    if pending.len() > SSE_PENDING_CAP {
        let drop_to = pending.len() - 8 * 1024;
        pending.drain(..drop_to);
    }
    progressed
}

fn sse_boundary(buf: &[u8]) -> Option<usize> {
    let crlf = buf
        .windows(4)
        .position(|mark| mark == b"\r\n\r\n")
        .map(|index| index + 4);
    let lf = buf
        .windows(2)
        .position(|mark| mark == b"\n\n")
        .map(|index| index + 2);
    match (crlf, lf) {
        (Some(crlf), Some(lf)) => Some(crlf.min(lf)),
        (Some(end), None) | (None, Some(end)) => Some(end),
        (None, None) => None,
    }
}

fn event_has_data_field(event: &[u8]) -> bool {
    String::from_utf8_lossy(event)
        .split(['\n', '\r'])
        .any(|line| line.starts_with("data:"))
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
fn admission_refused(live: usize, closing: usize) -> bool {
    // `transports` counts workers still alive. A closing tally that outlived
    // the last of them must not refuse the next turn.
    live > 0 && closing >= 4
}
fn terminal_event(value: &Wire) -> bool {
    matches!(
        value,
        Wire::Event(
            ModelEvent::Completed | ModelEvent::Failed { .. } | ModelEvent::Incomplete { .. }
        )
    )
}
/// Hands the caller its last event. The worker must already be finished so
/// dropping the caller does not count a socket that is about to exit.
fn deliver_terminal(settled: &Mutex<Settled>, tx: &SyncSender<Wire>, value: Wire) -> Result<()> {
    if terminal_event(&value) {
        lock(settled).worker_done = true;
    }
    send(tx, value)
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
        if admission_refused(
            self.inner.transports.load(Ordering::SeqCst),
            self.inner.closing.load(Ordering::SeqCst),
        ) {
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
        let emit_settled = worker_settled.clone();
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
                        return deliver_terminal(
                            &emit_settled,
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
                        deliver_terminal(&emit_settled, &tx, Wire::Event(event))
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
        net::{SocketAddr, TcpListener},
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

    #[test]
    fn connect_reaches_the_second_address_when_the_first_is_dead() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap();
        // TEST-NET-1 (RFC 5737): packets are discarded, so the dial waits out
        // the per-attempt budget instead of failing immediately.
        let dead = SocketAddr::from(([192, 0, 2, 1], 9));
        let started = Instant::now();
        let connected = super::connect_addrs(
            &[dead, live],
            Duration::from_millis(400),
            Duration::from_secs(2),
        );
        let elapsed = started.elapsed();
        connected.expect("the live address must be reached after the dead one");
        assert!(
            elapsed >= Duration::from_millis(250),
            "the dead address was skipped: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1200),
            "the dead address consumed the global budget: {elapsed:?}"
        );
        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_ok(),
            "the live listener saw no connection"
        );
    }

    #[test]
    fn a_stale_closing_tally_does_not_block_when_no_worker_remains() {
        assert!(super::admission_refused(4, 4));
        assert!(
            !super::admission_refused(0, 7),
            "closing outlived every transport"
        );
        assert!(!super::admission_refused(3, 3));
        assert!(!super::admission_refused(0, 0));
    }

    #[test]
    fn a_terminal_event_does_not_occupy_a_closing_slot() {
        let inner = std::sync::Arc::new(sample_inner());
        let settled = std::sync::Arc::new(super::super::Mutex::new(super::Settled {
            worker_done: false,
            counted: false,
        }));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        super::deliver_terminal(
            &settled,
            &tx,
            super::Wire::Event(grokbot_engine::contracts::ModelEvent::Completed),
        )
        .unwrap();
        drop(tx);
        let _ = rx.recv();
        {
            let _abandon = super::Abandon {
                flag: std::sync::Arc::new(AtomicBool::new(false)),
                inner: inner.clone(),
                settled: settled.clone(),
            };
        }
        assert_eq!(
            inner.closing.load(Ordering::SeqCst),
            0,
            "a worker that already delivered its last event was counted as closing"
        );
    }

    fn sample_inner() -> super::super::Inner {
        super::super::Inner {
            codex: true,
            deadlines: std::sync::Mutex::new(super::Deadlines::default()),
            transports: std::sync::atomic::AtomicUsize::new(0),
            closing: std::sync::atomic::AtomicUsize::new(0),
            simulation: AtomicBool::new(false),
            probe: std::sync::Mutex::new(serde_json::Value::Null),
            probe_generation: std::sync::atomic::AtomicUsize::new(0),
            local_revoked: AtomicBool::new(false),
            file: std::path::PathBuf::from("unused.sqlite"),
            endpoints: super::super::Endpoints::default(),
            open_browser: false,
            credentials: std::sync::Mutex::new(()),
            signin: std::sync::Mutex::new(super::super::SignIn {
                phase: super::super::Phase::Idle,
                cancel: None,
            }),
            runs: std::sync::Mutex::new(std::collections::HashMap::new()),
            interrupting: AtomicBool::new(false),
            titling: std::sync::atomic::AtomicUsize::new(0),
            catalog: std::sync::Mutex::new(super::super::Catalog::default()),
            quota: std::sync::Mutex::new(super::super::codex::UsageCache::default()),
            mcp: crate::mcp::Manager::default(),
        }
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
