use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{
    body_limit, default_database_path, header_safe, too_large, validate_headers_for_path, ApiError,
    ApiResponse, App, LaunchSecret, Provider,
};
use serde_json::json;
use tiny_http::{Header, Request, Response, Server, StatusCode};

mod signature;

const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const WOFF2: &str = "font/woff2";

/// A served static file. `gzip` and `brotli` are computed by build.rs. gzip is
/// kept only when smaller than `identity`, Brotli only when smaller than the
/// smallest other variant.
struct Asset {
    path: &'static str,
    content_type: &'static str,
    identity: &'static [u8],
    gzip: Option<&'static [u8]>,
    brotli: Option<&'static [u8]>,
}

impl Asset {
    fn encodings(&self) -> Vec<Encoding> {
        [(Encoding::Brotli, self.brotli), (Encoding::Gzip, self.gzip)]
            .into_iter()
            .filter_map(|(encoding, bytes)| bytes.map(|_| encoding))
            .collect()
    }

    fn body(&self, encoding: Encoding) -> &'static [u8] {
        match encoding {
            Encoding::Identity => self.identity,
            Encoding::Gzip => self.gzip.unwrap_or(self.identity),
            Encoding::Brotli => self.brotli.unwrap_or(self.identity),
        }
    }
}

const fn variant(bytes: &'static [u8]) -> Option<&'static [u8]> {
    if bytes.is_empty() {
        None
    } else {
        Some(bytes)
    }
}

macro_rules! asset {
    ($path:literal, $type:expr, $folder:literal, $key:literal, $name:literal) => {
        Asset {
            path: $path,
            content_type: $type,
            identity: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../app/",
                $folder,
                "/",
                $name
            )),
            gzip: variant(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/gzip/",
                $key,
                "/",
                $name,
                ".gz"
            ))),
            brotli: variant(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/brotli/",
                $key,
                "/",
                $name,
                ".br"
            ))),
        }
    };
    ($path:literal, $type:expr, ui $name:literal) => {
        asset!($path, $type, "ui", "ui", $name)
    };
    ($path:literal, $type:expr, public $name:literal) => {
        asset!($path, $type, "web/public", "public", $name)
    };
    ($path:literal, $type:expr, built $name:literal) => {
        Asset {
            path: $path,
            content_type: $type,
            identity: include_bytes!(concat!(env!("OUT_DIR"), "/", $name)),
            gzip: variant(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/gzip/",
                $name,
                ".gz"
            ))),
            brotli: variant(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/brotli/",
                $name,
                ".br"
            ))),
        }
    };
    ($path:literal, $type:expr, raw $folder:literal $name:literal) => {
        Asset {
            path: $path,
            content_type: $type,
            identity: include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../app/",
                $folder,
                "/",
                $name
            )),
            gzip: None,
            brotli: None,
        }
    };
}

/// Every file the UI is served from. A new path is added here and only here.
// chat.js and chat.css are built from app/web by `npm run build`.
static ASSETS: &[Asset] = &[
    asset!("/", HTML, ui "index.html"),
    asset!("/floating", HTML, ui "floating.html"),
    asset!("/styles.css", CSS, ui "styles.css"),
    asset!("/app.js", JS, ui "app.js"),
    asset!("/mascot.js", JS, ui "mascot.js"),
    asset!("/mascot.css", CSS, ui "mascot.css"),
    asset!("/chat.js", JS, ui "chat.js"),
    asset!("/chat.css", CSS, ui "chat.css"),
    asset!("/chat-code.js", JS, ui "chat-code.js"),
    asset!("/chat-mermaid.js", JS, ui "chat-mermaid.js"),
    asset!("/chat-math.js", JS, ui "chat-math.js"),
    asset!("/panels.js", JS, built "panels.js"),
    asset!("/panels.css", CSS, built "panels.css"),
    asset!("/iphone.js", JS, ui "iphone.js"),
    asset!("/iphone.css", CSS, ui "iphone.css"),
    asset!("/sw.js", JS, public "sw.js"),
    asset!("/manifest.webmanifest", "application/manifest+json", public "manifest.webmanifest"),
    asset!("/icon-192.png", "image/png", raw "web/public" "icon-192.png"),
    asset!("/icon-512.png", "image/png", raw "web/public" "icon-512.png"),
    asset!("/katex/KaTeX_AMS-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_AMS-Regular.woff2"),
    asset!("/katex/KaTeX_Caligraphic-Bold.woff2", WOFF2, raw "ui/katex" "KaTeX_Caligraphic-Bold.woff2"),
    asset!("/katex/KaTeX_Caligraphic-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Caligraphic-Regular.woff2"),
    asset!("/katex/KaTeX_Fraktur-Bold.woff2", WOFF2, raw "ui/katex" "KaTeX_Fraktur-Bold.woff2"),
    asset!("/katex/KaTeX_Fraktur-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Fraktur-Regular.woff2"),
    asset!("/katex/KaTeX_Main-Bold.woff2", WOFF2, raw "ui/katex" "KaTeX_Main-Bold.woff2"),
    asset!("/katex/KaTeX_Main-BoldItalic.woff2", WOFF2, raw "ui/katex" "KaTeX_Main-BoldItalic.woff2"),
    asset!("/katex/KaTeX_Main-Italic.woff2", WOFF2, raw "ui/katex" "KaTeX_Main-Italic.woff2"),
    asset!("/katex/KaTeX_Main-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Main-Regular.woff2"),
    asset!("/katex/KaTeX_Math-BoldItalic.woff2", WOFF2, raw "ui/katex" "KaTeX_Math-BoldItalic.woff2"),
    asset!("/katex/KaTeX_Math-Italic.woff2", WOFF2, raw "ui/katex" "KaTeX_Math-Italic.woff2"),
    asset!("/katex/KaTeX_SansSerif-Bold.woff2", WOFF2, raw "ui/katex" "KaTeX_SansSerif-Bold.woff2"),
    asset!("/katex/KaTeX_SansSerif-Italic.woff2", WOFF2, raw "ui/katex" "KaTeX_SansSerif-Italic.woff2"),
    asset!("/katex/KaTeX_SansSerif-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_SansSerif-Regular.woff2"),
    asset!("/katex/KaTeX_Script-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Script-Regular.woff2"),
    asset!("/katex/KaTeX_Size1-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Size1-Regular.woff2"),
    asset!("/katex/KaTeX_Size2-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Size2-Regular.woff2"),
    asset!("/katex/KaTeX_Size3-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Size3-Regular.woff2"),
    asset!("/katex/KaTeX_Size4-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Size4-Regular.woff2"),
    asset!("/katex/KaTeX_Typewriter-Regular.woff2", WOFF2, raw "ui/katex" "KaTeX_Typewriter-Regular.woff2"),
];

/// How an asset body travels. Brotli ranks above gzip on a tie.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Encoding {
    #[default]
    Identity,
    Gzip,
    Brotli,
}

impl Encoding {
    fn token(self) -> Option<&'static str> {
        match self {
            Encoding::Identity => None,
            Encoding::Gzip => Some("gzip"),
            Encoding::Brotli => Some("br"),
        }
    }
}

/// The coding an Accept-Encoding value picks among the variants an asset has:
/// the highest weight wins, Brotli then gzip break a tie, and a variant must weigh at
/// least as much as an explicit `identity` weight. When every coding is refused the
/// body still goes out as identity. An explicit coding beats `*`; q=0 refuses; an
/// unreadable weight counts as a refusal.
fn negotiate(accept_encoding: Option<&str>, available: &[Encoding]) -> Encoding {
    let mut explicit: [Option<f32>; 3] = [None, None, None];
    let mut wildcard = None;
    for item in accept_encoding.unwrap_or("").split(',') {
        let mut parts = item.split(';');
        let coding = parts.next().unwrap_or("").trim();
        let weight = parts
            .filter_map(|parameter| parameter.split_once('='))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("q"))
            .map_or(1.0, |(_, value)| {
                value
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|weight| (0.0..=1.0).contains(weight))
                    .unwrap_or(0.0)
            });
        if coding.eq_ignore_ascii_case("br") {
            explicit[0].get_or_insert(weight);
        } else if coding.eq_ignore_ascii_case("gzip") {
            explicit[1].get_or_insert(weight);
        } else if coding.eq_ignore_ascii_case("identity") {
            explicit[2].get_or_insert(weight);
        } else if coding == "*" {
            wildcard.get_or_insert(weight);
        }
    }
    let identity = explicit[2].unwrap_or(0.0);
    let mut chosen = (Encoding::Identity, 0.0);
    for (encoding, explicit) in [Encoding::Brotli, Encoding::Gzip].into_iter().zip(explicit) {
        let weight = explicit.or(wildcard).unwrap_or(0.0);
        if available.contains(&encoding) && weight > chosen.1 {
            chosen = (encoding, weight);
        }
    }
    if chosen.1 >= identity {
        chosen.0
    } else {
        Encoding::Identity
    }
}

/// What an asset reply adds to the shared headers.
#[derive(Clone, Copy, Default)]
struct Negotiated {
    encoding: Encoding,
    vary: bool,
}

const TICK_INTERVAL: Duration = Duration::from_millis(500);

struct Options {
    port: u16,
    db: PathBuf,
    routine_wake: bool,
    turns_summary: Option<String>,
}

fn options() -> Result<Option<Options>, String> {
    let mut args = std::env::args().skip(1);
    let mut port = 0;
    let mut routine_wake = false;
    let mut turns_summary = None;
    let mut db = default_database_path();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--routine-wake" | "--headless-routines" => routine_wake = true,
            "--port" => {
                port = args
                    .next()
                    .ok_or("--port attend un nombre entre 0 et 65535.")?
                    .parse()
                    .map_err(|_| "Port invalide.")?;
            }
            "--turns-summary" => {
                turns_summary = Some(args.next().ok_or("--turns-summary attend AAAA-MM-JJ.")?);
            }
            "--db" => {
                db = args.next().ok_or("--db attend un chemin SQLite.")?.into();
            }
            "--help" | "-h" => {
                println!("grokbot-wrapper [--port 0] [--db chemin.sqlite] [--routine-wake] [--turns-summary AAAA-MM-JJ]\nApplication locale, transport simulé. Écoute exclusivement 127.0.0.1.\n--turns-summary imprime le résumé du journal des tours de ce jour (heure locale) et sort.");
                return Ok(None);
            }
            _ => return Err(format!("Argument inconnu : {arg}")),
        }
    }
    Ok(Some(Options {
        port,
        db,
        routine_wake,
        turns_summary,
    }))
}

fn error(status: u16, message: &str) -> ApiError {
    ApiError {
        status,
        message: message.into(),
    }
}

/// tiny_http filters Connection from Response::add_header. For disabled access,
/// write the fixed denial explicitly so browsers/proxies retire their connection.
fn remote_disabled(request: Request) -> std::io::Result<()> {
    let body = json!({"error": grokbot_wrapper::features::iphone::ACCESS_DISABLED}).to_string();
    let mut writer = request.into_writer();
    write!(writer, "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

/// Retire every HTTP connection at its response. tiny_http reads each open socket
/// on one pool thread, and a burst larger than its four idle threads is queued
/// without starting more: a browser's kept-alive sockets would hold the queued
/// requests of its page load until it closes them. A private socket must also not
/// outlive a host restart.
/// Keep tiny_http's framing (including ranges, cookies, chunking and HEAD), but
/// add the hop-by-hop header on the wire because Response filters that header.
fn respond_retiring(
    request: Request,
    response: Response<std::io::Cursor<Vec<u8>>>,
) -> std::io::Result<()> {
    let mut encoded = Vec::new();
    response.raw_print(
        &mut encoded,
        request.http_version().clone(),
        request.headers(),
        request.method().as_str() == "HEAD",
        None,
    )?;
    let split = encoded
        .windows(2)
        .position(|p| p == b"\r\n")
        .ok_or(std::io::ErrorKind::InvalidData)?;
    let mut writer = request.into_writer();
    writer.write_all(&encoded[..split])?;
    writer.write_all(b"\r\nConnection: close")?;
    writer.write_all(&encoded[split..])?;
    writer.flush()
}

fn payload(
    request: &mut Request,
    port: u16,
    app: &mut App,
    remote: bool,
    secret: &LaunchSecret,
) -> Result<(ApiResponse, Negotiated), ApiError> {
    let method = request.method().as_str().to_owned();
    let path = request.url().to_owned();
    let headers: Vec<_> = request
        .headers()
        .iter()
        .map(|header| (header.field.as_str().to_string(), header.value.to_string()))
        .collect();
    let asset_path = if path.starts_with("/?notification=") {
        "/"
    } else {
        path.as_str()
    };
    let asset = (method == "GET")
        .then(|| ASSETS.iter().find(|asset| asset.path == asset_path))
        .flatten();
    if remote {
        grokbot_wrapper::features::iphone::validate_headers(&method, &headers)?;
        if !grokbot_wrapper::features::iphone::enabled(app) {
            return Err(error(
                403,
                grokbot_wrapper::features::iphone::ACCESS_DISABLED,
            ));
        }
    } else if asset.is_some() {
        grokbot_wrapper::validate_static_headers(port, &headers)?;
    } else {
        validate_headers_for_path(port, &method, &path, &headers, secret)?;
    }
    let limit = body_limit(&path);
    if request.body_length().is_some_and(|length| length > limit) {
        return Err(too_large(&path));
    }
    if let Some(asset) = asset {
        let accept_encoding = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let available = asset.encodings();
        let encoding = negotiate(Some(accept_encoding.as_str()), &available);
        return Ok((
            ApiResponse::Bytes {
                content_type: asset.content_type.into(),
                body: asset.body(encoding).to_vec(),
            },
            Negotiated {
                encoding,
                vary: !available.is_empty(),
            },
        ));
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take((limit + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| error(400, "Le corps de la requête est illisible."))?;
    // Chunked requests have no declared length. Check again after the bounded read.
    if body.len() > limit {
        return Err(too_large(&path));
    }
    if remote {
        grokbot_wrapper::features::iphone::remote(app, &method, &path, &body, &headers)
    } else {
        app.api_response(&method, &path, &body)
    }
    .map(|response| (response, Negotiated::default()))
}

/// tiny_http offers no read timeout, so a body reader lives until its sender
/// finishes or leaves. It never runs on the host thread; the number of live
/// readers, hence of sockets and body buffers a stalled sender can hold, is bounded.
const WEBHOOK_MAX_READERS: usize = 4;
static WEBHOOK_READERS: AtomicUsize = AtomicUsize::new(0);

/// An authenticated call whose body a reader thread has finished receiving.
struct WebhookBody {
    request: Request,
    call: grokbot_wrapper::features::routine_webhooks::Authorized,
    key: Option<String>,
    body: std::io::Result<Vec<u8>>,
}

/// Answer a webhook call and close its connection: an unread body must not leak
/// into the next request of a kept-alive socket.
fn webhook_respond(request: Request, reply: grokbot_wrapper::features::routine_webhooks::Reply) {
    let mut response = Response::from_data(reply.body.to_string().into_bytes())
        .with_status_code(StatusCode(reply.status));
    let mut headers = vec![
        ("Content-Type", "application/json; charset=utf-8"),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
    ];
    if reply.status == 401 {
        headers.push(("WWW-Authenticate", "Bearer"));
    }
    for (name, value) in headers {
        response.add_header(Header::from_bytes(name, value).expect("webhook response header"));
    }
    if let Err(error) = respond_retiring(request, response) {
        eprintln!("Réponse webhook interrompue : {error}");
    }
}

/// First half of a webhook call. The caller is authenticated from its headers
/// alone: a body is read only for a valid token, so an anonymous client cannot
/// hold anything. The body is read on a bounded reader thread and handed back
/// to the host loop through `done`, so a stalled upload never freezes the app.
/// Funnel proxies here; the listener itself is loopback.
fn webhook(request: Request, app: &App, done: &std::sync::mpsc::Sender<WebhookBody>) {
    use grokbot_wrapper::features::routine_webhooks as hooks;
    let header = |name: &str| {
        request
            .headers()
            .iter()
            .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
            .map(|h| h.value.to_string())
    };
    let (authorization, key) = (header("Authorization"), header("Idempotency-Key"));
    let method = request.method().as_str().to_owned();
    let url = request.url().to_owned();
    let too_big = request.body_length().is_some_and(|n| n > hooks::BODY_LIMIT);
    let early = |status: u16, text: &str| hooks::Reply {
        status,
        body: json!({ "error": text }),
    };
    let call = match hooks::authorize(app, &method, &url, authorization.as_deref()) {
        Err(reply) => {
            return webhook_respond(request, reply);
        }
        Ok(call) => call,
    };
    if too_big {
        let reply = early(413, "Le corps dépasse la borne du prompt d’une routine.");
        return webhook_respond(request, reply);
    }
    if WEBHOOK_READERS.fetch_add(1, Ordering::SeqCst) >= WEBHOOK_MAX_READERS {
        WEBHOOK_READERS.fetch_sub(1, Ordering::SeqCst);
        let reply = early(
            503,
            "Trop d’envois de corps en cours. Réessayez dans un instant.",
        );
        return webhook_respond(request, reply);
    }
    let done = done.clone();
    std::thread::spawn(move || {
        let mut request = request;
        let mut body = Vec::new();
        let read = request
            .as_reader()
            .take((hooks::BODY_LIMIT + 1) as u64)
            .read_to_end(&mut body);
        WEBHOOK_READERS.fetch_sub(1, Ordering::SeqCst);
        let _ = done.send(WebhookBody {
            request,
            call,
            key,
            body: read.map(|_| body),
        });
    });
}

/// Second half, on the host loop: the body is complete, record the call.
fn webhook_finish(received: WebhookBody, app: &mut App) {
    use grokbot_wrapper::features::routine_webhooks as hooks;
    let reply = match received.body {
        Ok(body) => hooks::accept(app, &received.call, received.key.as_deref(), &body),
        Err(_) => hooks::Reply {
            status: 400,
            body: json!({"error": "Le corps de la requête est illisible."}),
        },
    };
    webhook_respond(received.request, reply);
}

#[cfg(debug_assertions)]
mod fake_signin;

/// Les builds de développement peuvent viser un faux serveur OpenAI pour essayer l'interface
/// sans compte. Un build de production ne lit jamais cette variable.
/// `EZERBOT_FAKE_SIGNIN=1` n’existe que dans un binaire de debug : il démarre son propre
/// faux OpenAI sur 127.0.0.1 et ouvre une session locale, sans OAuth.
fn provider(db: &std::path::Path) -> Provider {
    #[cfg(debug_assertions)]
    if let Some(provider) = fake_signin::connect(db) {
        return provider;
    }
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("GROKBOT_FAKE_OPENAI") {
        let base = fake_openai_base(&base)
            .expect("GROKBOT_FAKE_OPENAI doit viser http://127.0.0.1:<port>.");
        let endpoints = grokbot_wrapper::Endpoints {
            authorize: format!("{base}/authorize"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            jwks: format!("{base}/jwks"),
            issuer: base.clone(),
            api: format!("{base}/v1"),
        };
        let codex = grokbot_wrapper::Endpoints {
            api: format!("{base}/backend-api/codex"),
            ..endpoints.clone()
        };
        let provider = Provider::with_backends(db, endpoints, codex, false);
        if let Some(ms) = std::env::var("GROKBOT_FAKE_CHAT_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|ms| (100..=60000).contains(ms))
        {
            provider.configure_fake_chat(
                std::time::Duration::from_millis(ms),
                std::env::var_os("GROKBOT_DISABLE_SIMULATION").is_none(),
            );
        }
        return provider;
    }
    Provider::new(db)
}

#[cfg(debug_assertions)]
fn fake_openai_base(base: &str) -> Option<String> {
    // L’override de QA ne peut jamais détourner les identifiants vers un hôte distant.
    let port = base
        .strip_prefix("http://127.0.0.1:")?
        .parse::<u16>()
        .ok()?;
    (port != 0).then(|| format!("http://127.0.0.1:{port}"))
}

/// A single byte range is enough for WebKit's media loader. Reject malformed,
/// multiple and unsatisfiable ranges rather than silently returning the full WAV.
fn audio_range(value: &str, length: usize) -> Option<(usize, usize)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if length == 0 {
        return None;
    }
    let decimal = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<usize>().ok())
            .flatten()
    };
    if start.is_empty() {
        let suffix = decimal(end)?;
        return (suffix > 0).then(|| (length.saturating_sub(suffix), length - 1));
    }
    let start = decimal(start)?;
    let end = if end.is_empty() {
        length - 1
    } else {
        decimal(end)?.min(length - 1)
    };
    (start < length && start <= end).then_some((start, end))
}

static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_quit(_: nix::libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

/// The shell stops the app with SIGTERM and gives it a second. Record that the
/// app is going away, so routines know whether anything was asked to run while
/// it was closed, then leave as the default action would have.
fn leave_on_quit(db: PathBuf) {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};
    let action = SigAction::new(
        SigHandler::Handler(on_quit),
        SaFlags::empty(),
        SigSet::empty(),
    );
    for signal in [Signal::SIGTERM, Signal::SIGINT] {
        // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
        if unsafe { sigaction(signal, &action) }.is_err() {
            return;
        }
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(25));
        if QUIT.load(Ordering::SeqCst) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64;
            grokbot_wrapper::features::routines::depart_database(&db, now);
            std::process::exit(0);
        }
    });
}

/// Résumé du journal des tours sur la base ouverte en lecture seule : jamais d'écriture ni de migration.
fn turns_summary(db: &std::path::Path, day: &str) -> Result<(), String> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let conn = rusqlite::Connection::open_with_flags(db, flags)
        .map_err(|error| format!("Base illisible ({}) : {error}", db.display()))?;
    let summary = grokbot_wrapper::features::turns::summary(&conn, day).map_err(|e| e.message)?;
    println!("{summary}");
    Ok(())
}

fn serve(options: Options) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use grokbot_wrapper::features::routines_background::{self, Owner};
    let secret = if options.routine_wake {
        None
    } else {
        Some(LaunchSecret::read_pipe()?)
    };
    // Acquire BEFORE App::open: its crash recovery must not interrupt a peer.
    let _owner = if options.routine_wake {
        let Some(owner) = Owner::try_acquire(&options.db)? else {
            return Ok(());
        };
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as i64;
        if !routines_background::due(&options.db, now)? {
            return Ok(());
        }
        owner
    } else {
        Owner::foreground(&options.db)?
    };
    grokbot_forms::vault::enable_os_vault(signature::stable_identity());
    let mut app = App::open_with(&options.db, provider(&options.db))?;
    // First-start migration runs before serving the page. Keychain denial falls
    // back to the owner-only file and is projected in Paramètres, never logged.
    grokbot_wrapper::features::forms::vault(&app.feature_context())?;
    if options.routine_wake {
        let code = match routines_background::run(&mut app) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("Ezerbot Local : {error}");
                1
            }
        };
        // run() has drained every engine worker (including on interruption).
        // Retain Owner until process death; idle provider threads cannot outlive it.
        std::process::exit(code);
    }
    let secret = secret.ok_or("Le canal de lancement sécurisé est indisponible.")?;
    leave_on_quit(options.db.clone());
    let server = Server::http(("127.0.0.1", options.port))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Adresse locale non disponible.")?
        .port();
    if port == grokbot_wrapper::features::iphone::port() {
        return Err("Le port 47863 est réservé à l’accès iPhone.".into());
    }
    grokbot_wrapper::features::yolo::register_control_port(port);
    grokbot_wrapper::features::yolo::register_control_port(
        grokbot_wrapper::features::iphone::port(),
    );
    let mut remote_server: Option<Server> = None;
    let mut hook_server: Option<Server> = None;
    let mut hook_retry = std::time::Instant::now();
    let (hook_sender, hook_bodies) = std::sync::mpsc::channel::<WebhookBody>();
    let mut remote_turn = false;
    println!("{}", json!({"url": format!("http://127.0.0.1:{port}")}));
    std::io::stdout().flush()?;
    loop {
        let iphone = grokbot_wrapper::features::iphone::enabled(&app);
        if iphone && remote_server.is_none() {
            match Server::http(("127.0.0.1", grokbot_wrapper::features::iphone::port())) {
                Ok(listener) => {
                    remote_server = Some(listener);
                    grokbot_wrapper::features::iphone::listener_error(&app, None);
                }
                Err(_) => grokbot_wrapper::features::iphone::listener_error(
                    &app,
                    Some("Le port 47863 est occupé. Libérez-le ou désactivez l’accès iPhone."),
                ),
            }
        }
        // The webhook listener lives with the open app, loopback only, and only
        // while a routine has a webhook. Its bind result gates Funnel publication.
        if hook_server.is_none()
            && hook_retry <= std::time::Instant::now()
            && grokbot_wrapper::features::routine_webhooks::wanted(&app)
        {
            match Server::http((
                "127.0.0.1",
                grokbot_wrapper::features::routine_webhooks::port(),
            )) {
                Ok(listener) => {
                    hook_server = Some(listener);
                    grokbot_wrapper::features::routine_webhooks::listener(&app, None);
                }
                Err(_) => {
                    hook_retry = std::time::Instant::now() + Duration::from_secs(3);
                    grokbot_wrapper::features::routine_webhooks::listener(
                        &app,
                        Some("Le port local des webhooks est occupé. Libérez-le avant de publier."),
                    );
                }
            }
        }
        while let Ok(received) = hook_bodies.try_recv() {
            webhook_finish(received, &mut app);
        }
        if let Some(request) = hook_server
            .as_ref()
            .and_then(|s| s.try_recv().ok().flatten())
        {
            webhook(request, &app, &hook_sender);
        }
        // Retain this loopback listener once enabled. Dropping tiny_http::Server
        // strands accepted keep-alive sockets in its old queue; a re-created
        // listener can never answer those sockets. Disabled access is a denial,
        // including for old connections, not an orphaned request queue.
        // Never change the desktop listener's loopback/Host protections. The
        // stable second port is also loopback: only Tailscale Serve exposes it.
        // Alternate when both queues are busy: a phone (even an unpaired one)
        // must never starve the Mac window's revoke/disable controls.
        let local_request = if remote_turn {
            None
        } else {
            server.try_recv()?
        };
        let remote_request = if local_request.is_none() {
            remote_server
                .as_ref()
                .and_then(|s| s.try_recv().ok().flatten())
        } else {
            None
        };
        let (request, remote) = if remote_request.is_some() {
            (remote_request, true)
        } else if local_request.is_some() {
            (local_request, false)
        } else {
            (
                server.recv_timeout(if remote_server.is_some() || hook_server.is_some() {
                    Duration::from_millis(50)
                } else {
                    TICK_INTERVAL
                })?,
                false,
            )
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        app.tick(now_ms);
        let Some(mut request) = request else {
            continue;
        };
        remote_turn = !remote;
        if remote && !grokbot_wrapper::features::iphone::enabled(&app) {
            if let Err(error) = remote_disabled(request) {
                eprintln!("Réponse iPhone désactivé interrompue : {error}");
            }
            continue;
        }
        let is_api = request.url().starts_with("/api/");
        // Timed through JSON encoding, which grows with the history (#255).
        let mut state_elapsed = None;
        let state_started = (request.method() == &tiny_http::Method::Get
            && request.url().split('?').next() == Some("/api/state"))
        .then(std::time::Instant::now);
        if state_started.is_some() {
            let client = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("X-Ezerbot-Client"))
                .map(|header| header.value.as_str().to_owned());
            app.set_state_client(remote, client.as_deref());
        }
        let mut cookie = None;
        let mut negotiated = Negotiated::default();
        let (mut status, mut body, content_type) = match payload(
            &mut request,
            port,
            &mut app,
            remote,
            &secret,
        ) {
            Ok((mut response, reply)) => {
                if remote && request.url() == "/api/iphone/exchange" {
                    if let ApiResponse::Json(ref mut value) = response {
                        if let Some(credential) = value
                            .as_object_mut()
                            .and_then(|v| v.remove("credential"))
                            .and_then(|v| v.as_str().map(str::to_owned))
                        {
                            let secure = if cfg!(debug_assertions) {
                                ""
                            } else {
                                "; Secure"
                            };
                            cookie = Some(format!("ezerbot_device={credential}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{secure}"));
                            value["paired"] = json!(true);
                        }
                    }
                }
                let (body, content_type) = response.into_parts();
                state_elapsed = state_started.map(|started| started.elapsed());
                if header_safe(&content_type) {
                    negotiated = reply;
                    (200, body, content_type)
                } else {
                    eprintln!("Type de contenu invalide refusé.");
                    (
                        500,
                        json!({"error": "La réponse de l'application est invalide."})
                            .to_string()
                            .into_bytes(),
                        "application/json; charset=utf-8".into(),
                    )
                }
            }
            Err(error) => (
                error.status,
                json!({"error": error.message}).to_string().into_bytes(),
                "application/json; charset=utf-8".into(),
            ),
        };
        let mut media_headers = Vec::new();
        if status == 200 && content_type.starts_with("audio/") {
            media_headers.push(("Accept-Ranges", "bytes".to_owned()));
            if let Some(range) = request.headers().iter().find(|h| h.field.equiv("Range")) {
                let length = body.len();
                if let Some((start, end)) = audio_range(range.value.as_str(), length) {
                    body = body[start..=end].to_vec();
                    status = 206;
                    media_headers.push(("Content-Range", format!("bytes {start}-{end}/{length}")));
                } else {
                    body.clear();
                    status = 416;
                    media_headers.push(("Content-Range", format!("bytes */{length}")));
                }
            }
        }
        let mut response = Response::from_data(body).with_status_code(StatusCode(status));
        if let Some(cookie) = cookie {
            response.add_header(Header::from_bytes("Set-Cookie", cookie).expect("device cookie"));
        }
        for (name, value) in media_headers {
            response.add_header(Header::from_bytes(name, value).expect("media response header"));
        }
        for (name, value) in [
            ("Content-Type", content_type.as_str()),
            ("Cache-Control", if is_api || remote { "no-store" } else { "no-cache" }),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
            ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: ezerbot-resource:; font-src 'self' data:; media-src 'self' blob: ezerbot-resource:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("validated response header"));
        }
        if negotiated.vary {
            response.add_header(
                Header::from_bytes("Vary", "Accept-Encoding").expect("validated response header"),
            );
        }
        if let Some(token) = negotiated.encoding.token() {
            response.add_header(
                Header::from_bytes("Content-Encoding", token).expect("validated response header"),
            );
        }
        // Retiring drains the body first, and a local refusal can precede a body that never comes.
        let result = if remote || status < 400 {
            respond_retiring(request, response)
        } else {
            request.respond(response)
        };
        if let Err(error) = result {
            eprintln!("Réponse HTTP interrompue : {error}");
        }
        if let Some(elapsed) = state_elapsed {
            app.log_state_served(elapsed);
        }
    }
}

fn main() {
    let result = match options() {
        Ok(Some(options)) => match options.turns_summary.as_deref() {
            Some(day) => turns_summary(&options.db, day),
            None => serve(options).map_err(|error| error.to_string()),
        },
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("Ezerbot Local : {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod asset_tests {
    use super::{negotiate, Encoding, ASSETS};
    use std::io::Read;

    use Encoding::{Brotli, Gzip, Identity};

    #[test]
    fn the_encoding_is_chosen_from_accept_encoding_among_the_variants_that_exist() {
        let both = [Encoding::Brotli, Encoding::Gzip];
        for (value, expected) in [
            (None, Identity),
            (Some(""), Identity),
            (Some("gzip"), Gzip),
            (Some("GZip"), Gzip),
            (Some("  gzip  "), Gzip),
            (Some("br"), Brotli),
            (Some("BR"), Brotli),
            (Some("gzip, deflate, br"), Brotli),
            (Some("br, gzip"), Brotli),
            (Some("gzip;q=1, br;q=0.5"), Gzip),
            (Some("gzip;q=0.5, br"), Brotli),
            (Some("gzip;q=0.5, br;q=0.5"), Brotli),
            (Some("br;q=0, gzip"), Gzip),
            (Some("br;q=0"), Identity),
            (Some("gzip;q=0, br;q=0"), Identity),
            (Some("deflate, gzip;q=0.5"), Gzip),
            (Some("gzip ; q = 0.001"), Gzip),
            (Some("gzip;q=1"), Gzip),
            (Some("gzip;q=0"), Identity),
            (Some("gzip;q=0.0"), Identity),
            (Some("gzip;Q=0"), Identity),
            (Some("gzip;q=oops"), Identity),
            (Some("br;q=oops, gzip"), Gzip),
            (Some("deflate"), Identity),
            (Some("identity"), Identity),
            (Some("*"), Brotli),
            (Some("*;q=0"), Identity),
            (Some("identity;q=1, br;q=0.1"), Identity),
            (Some("identity;q=0.5, gzip;q=0.8, br;q=0.2"), Gzip),
            (Some("br;q=0.5, identity;q=0.5"), Brotli),
            (Some("*;q=0.5, br;q=0.4"), Gzip),
            (Some("identity;q=0, gzip;q=0, br;q=0"), Identity),
            (Some("identity;q=0, br;q=0.1"), Brotli),
            (Some("*;q=0.1, deflate"), Brotli),
            (Some("gzip;q=0, *"), Brotli),
            (Some("br;q=0, *"), Gzip),
            (Some("*;q=0, gzip"), Gzip),
            (Some("*;q=0, br"), Brotli),
            (Some("gzip, *;q=0.5"), Gzip),
            (Some("br;q=0.4, *;q=0.9"), Gzip),
            (Some("gzipped"), Identity),
            (Some("brotli"), Identity),
        ] {
            assert_eq!(negotiate(value, &both), expected, "{value:?}");
        }
    }

    #[test]
    fn a_coding_the_asset_lacks_is_never_chosen() {
        for (available, value, expected) in [
            (&[Gzip][..], Some("gzip, deflate, br"), Gzip),
            (&[Gzip], Some("br"), Identity),
            (&[Gzip], Some("*"), Gzip),
            (&[Gzip], Some("br;q=0, gzip"), Gzip),
            (&[Gzip], Some("br, gzip;q=0.1"), Gzip),
            (&[Brotli], Some("gzip"), Identity),
            (&[Brotli], Some("gzip, br"), Brotli),
            (&[Brotli], Some("*"), Brotli),
            (&[Brotli], Some("br;q=0, *"), Identity),
            (&[], Some("gzip, deflate, br"), Identity),
            (&[], Some("*"), Identity),
            (&[], None, Identity),
        ] {
            assert_eq!(
                negotiate(value, available),
                expected,
                "{available:?} {value:?}"
            );
        }
    }

    #[test]
    fn served_paths_are_the_explicit_list_and_compressed_variants_decode_to_identity() {
        let mut paths: Vec<_> = ASSETS.iter().map(|asset| asset.path).collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            [
                "/",
                "/app.js",
                "/chat-code.js",
                "/chat-math.js",
                "/chat-mermaid.js",
                "/chat.css",
                "/chat.js",
                "/floating",
                "/icon-192.png",
                "/icon-512.png",
                "/iphone.css",
                "/iphone.js",
                "/katex/KaTeX_AMS-Regular.woff2",
                "/katex/KaTeX_Caligraphic-Bold.woff2",
                "/katex/KaTeX_Caligraphic-Regular.woff2",
                "/katex/KaTeX_Fraktur-Bold.woff2",
                "/katex/KaTeX_Fraktur-Regular.woff2",
                "/katex/KaTeX_Main-Bold.woff2",
                "/katex/KaTeX_Main-BoldItalic.woff2",
                "/katex/KaTeX_Main-Italic.woff2",
                "/katex/KaTeX_Main-Regular.woff2",
                "/katex/KaTeX_Math-BoldItalic.woff2",
                "/katex/KaTeX_Math-Italic.woff2",
                "/katex/KaTeX_SansSerif-Bold.woff2",
                "/katex/KaTeX_SansSerif-Italic.woff2",
                "/katex/KaTeX_SansSerif-Regular.woff2",
                "/katex/KaTeX_Script-Regular.woff2",
                "/katex/KaTeX_Size1-Regular.woff2",
                "/katex/KaTeX_Size2-Regular.woff2",
                "/katex/KaTeX_Size3-Regular.woff2",
                "/katex/KaTeX_Size4-Regular.woff2",
                "/katex/KaTeX_Typewriter-Regular.woff2",
                "/manifest.webmanifest",
                "/mascot.css",
                "/mascot.js",
                "/panels.css",
                "/panels.js",
                "/styles.css",
                "/sw.js",
            ]
        );
        for asset in ASSETS {
            assert!(!asset.identity.is_empty(), "{}", asset.path);
            assert!(
                !asset.path.ends_with(".png") || (asset.gzip.is_none() && asset.brotli.is_none())
            );
            if let Some(gzip) = asset.gzip {
                assert!(gzip.len() < asset.identity.len(), "{}", asset.path);
                let mut decoded = Vec::new();
                flate2::read::GzDecoder::new(gzip)
                    .read_to_end(&mut decoded)
                    .unwrap();
                assert_eq!(decoded, asset.identity, "{}", asset.path);
            }
            if let Some(brotli) = asset.brotli {
                let smallest = asset.gzip.map_or(asset.identity.len(), <[u8]>::len);
                assert!(brotli.len() < smallest, "{}", asset.path);
                let mut decoded = Vec::new();
                brotli::Decompressor::new(brotli, 4096)
                    .read_to_end(&mut decoded)
                    .unwrap();
                assert_eq!(decoded, asset.identity, "{}", asset.path);
            }
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod fake_tests {
    #[test]
    fn override_accepts_only_an_explicit_loopback_port() {
        assert_eq!(
            super::fake_openai_base("http://127.0.0.1:12345").as_deref(),
            Some("http://127.0.0.1:12345")
        );
        for value in [
            "https://api.openai.com",
            "http://example.com:123",
            "http://127.0.0.1:0",
            "http://127.0.0.1:123@evil.test",
            "http://127.0.0.1:123/path",
        ] {
            assert!(super::fake_openai_base(value).is_none());
        }
    }
}
