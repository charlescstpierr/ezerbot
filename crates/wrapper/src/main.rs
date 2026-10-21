use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{
    body_limit, default_database_path, header_safe, too_large, validate_headers_for_path, ApiError,
    ApiResponse, App, Provider,
};
use serde_json::json;
use tiny_http::{Header, Request, Response, Server, StatusCode};

mod signature;

const INDEX: &str = include_str!("../../../app/ui/index.html");
const FLOATING: &str = include_str!("../../../app/ui/floating.html");
const STYLES: &str = include_str!("../../../app/ui/styles.css");
const SCRIPT: &str = include_str!("../../../app/ui/app.js");
const MASCOT_SCRIPT: &str = include_str!("../../../app/ui/mascot.js");
const MASCOT_STYLES: &str = include_str!("../../../app/ui/mascot.css");
// Built from app/web by `npm run build`.
const CHAT_SCRIPT: &str = include_str!("../../../app/ui/chat.js");
const CHAT_STYLES: &str = include_str!("../../../app/ui/chat.css");
const PANELS_SCRIPT: &str = include_str!(concat!(env!("OUT_DIR"), "/panels.js"));
const PANELS_STYLES: &str = include_str!(concat!(env!("OUT_DIR"), "/panels.css"));
const TICK_INTERVAL: Duration = Duration::from_millis(500);

struct Options {
    port: u16,
    db: PathBuf,
    routine_wake: bool,
}

fn options() -> Result<Option<Options>, String> {
    let mut args = std::env::args().skip(1);
    let mut port = 0;
    let mut routine_wake = false;
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
            "--db" => {
                db = args.next().ok_or("--db attend un chemin SQLite.")?.into();
            }
            "--help" | "-h" => {
                println!("grokbot-wrapper [--port 0] [--db chemin.sqlite] [--routine-wake]\nApplication locale, transport simulé. Écoute exclusivement 127.0.0.1.");
                return Ok(None);
            }
            _ => return Err(format!("Argument inconnu : {arg}")),
        }
    }
    Ok(Some(Options {
        port,
        db,
        routine_wake,
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

/// Retire private HTTP connections at every response, also across host restarts.
/// Keep tiny_http's framing (including ranges, cookies, chunking and HEAD), but
/// add the hop-by-hop header on the wire because Response filters that header.
fn respond_remote(
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
) -> Result<ApiResponse, ApiError> {
    let method = request.method().as_str().to_owned();
    let path = request.url().to_owned();
    let headers: Vec<_> = request
        .headers()
        .iter()
        .map(|header| (header.field.as_str().to_string(), header.value.to_string()))
        .collect();
    if remote {
        grokbot_wrapper::features::iphone::validate_headers(&method, &headers)?;
        if !grokbot_wrapper::features::iphone::enabled(app) {
            return Err(error(
                403,
                grokbot_wrapper::features::iphone::ACCESS_DISABLED,
            ));
        }
    } else {
        validate_headers_for_path(port, &method, &path, &headers)?;
    }
    let limit = body_limit(&path);
    if request.body_length().is_some_and(|length| length > limit) {
        return Err(too_large(&path));
    }
    if method == "GET" {
        let asset_path = if path.starts_with("/?notification=") {
            "/"
        } else {
            path.as_str()
        };
        let asset = match asset_path {
            "/" => Some((INDEX, "text/html; charset=utf-8")),
            "/floating" => Some((FLOATING, "text/html; charset=utf-8")),
            "/styles.css" => Some((STYLES, "text/css; charset=utf-8")),
            "/app.js" => Some((SCRIPT, "text/javascript; charset=utf-8")),
            "/mascot.js" => Some((MASCOT_SCRIPT, "text/javascript; charset=utf-8")),
            "/mascot.css" => Some((MASCOT_STYLES, "text/css; charset=utf-8")),
            "/chat.js" => Some((CHAT_SCRIPT, "text/javascript; charset=utf-8")),
            "/chat.css" => Some((CHAT_STYLES, "text/css; charset=utf-8")),
            "/panels.js" => Some((PANELS_SCRIPT, "text/javascript; charset=utf-8")),
            "/panels.css" => Some((PANELS_STYLES, "text/css; charset=utf-8")),
            "/iphone.js" => Some((
                include_str!("../../../app/ui/iphone.js"),
                "text/javascript; charset=utf-8",
            )),
            "/iphone.css" => Some((
                include_str!("../../../app/ui/iphone.css"),
                "text/css; charset=utf-8",
            )),
            "/sw.js" => Some((
                include_str!("../../../app/web/public/sw.js"),
                "text/javascript; charset=utf-8",
            )),
            "/manifest.webmanifest" => Some((
                include_str!("../../../app/web/public/manifest.webmanifest"),
                "application/manifest+json",
            )),
            _ => None,
        };
        let icon = match path.as_str() {
            "/icon-192.png" => {
                Some(include_bytes!("../../../app/web/public/icon-192.png").as_slice())
            }
            "/icon-512.png" => {
                Some(include_bytes!("../../../app/web/public/icon-512.png").as_slice())
            }
            _ => None,
        };
        if let Some(bytes) = icon {
            return Ok(ApiResponse::Bytes {
                content_type: "image/png".into(),
                body: bytes.to_vec(),
            });
        }
        if let Some((content, content_type)) = asset {
            return Ok(ApiResponse::Bytes {
                content_type: content_type.into(),
                body: content.as_bytes().to_vec(),
            });
        }
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
}

/// Les builds de développement peuvent viser un faux serveur OpenAI pour essayer l'interface
/// sans compte. Un build de production ne lit jamais cette variable.
fn provider(db: &std::path::Path) -> Provider {
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

fn serve(options: Options) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use grokbot_wrapper::features::routines_background::{self, Owner};
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
    let server = Server::http(("127.0.0.1", options.port))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Adresse locale non disponible.")?
        .port();
    if port == grokbot_wrapper::features::iphone::PORT {
        return Err("Le port 47863 est réservé à l’accès iPhone.".into());
    }
    let mut remote_server: Option<Server> = None;
    let mut remote_turn = false;
    println!("{}", json!({"url": format!("http://127.0.0.1:{port}")}));
    std::io::stdout().flush()?;
    loop {
        let iphone = grokbot_wrapper::features::iphone::enabled(&app);
        if iphone && remote_server.is_none() {
            match Server::http(("127.0.0.1", grokbot_wrapper::features::iphone::PORT)) {
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
                server.recv_timeout(if remote_server.is_some() {
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
        let mut cookie = None;
        let (mut status, mut body, content_type) = match payload(
            &mut request,
            port,
            &mut app,
            remote,
        ) {
            Ok(mut response) => {
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
                if header_safe(&content_type) {
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
            ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("validated response header"));
        }
        let result = if remote {
            respond_remote(request, response)
        } else {
            request.respond(response)
        };
        if let Err(error) = result {
            eprintln!("Réponse HTTP interrompue : {error}");
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

fn main() {
    let result = match options() {
        Ok(Some(options)) => serve(options).map_err(|error| error.to_string()),
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("Ezerbot Local : {error}");
        std::process::exit(1);
    }
}
