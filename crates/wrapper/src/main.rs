use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{
    body_limit, header_safe, too_large, validate_headers_for_path, ApiError, ApiResponse, App,
    Provider,
};
use serde_json::json;
use tiny_http::{Header, Request, Response, Server, StatusCode};

mod signature;

const INDEX: &str = include_str!("../../../app/ui/index.html");
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
    let mut db = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/Grokbot Local/workspace.sqlite");
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

fn payload(request: &mut Request, port: u16, app: &mut App) -> Result<ApiResponse, ApiError> {
    let method = request.method().as_str().to_owned();
    let path = request.url().to_owned();
    let headers: Vec<_> = request
        .headers()
        .iter()
        .map(|header| (header.field.as_str().to_string(), header.value.to_string()))
        .collect();
    validate_headers_for_path(port, &method, &path, &headers)?;
    let limit = body_limit(&path);
    if request.body_length().is_some_and(|length| length > limit) {
        return Err(too_large(&path));
    }
    if method == "GET" {
        let asset = match path.as_str() {
            "/" => Some((INDEX, "text/html; charset=utf-8")),
            "/styles.css" => Some((STYLES, "text/css; charset=utf-8")),
            "/app.js" => Some((SCRIPT, "text/javascript; charset=utf-8")),
            "/mascot.js" => Some((MASCOT_SCRIPT, "text/javascript; charset=utf-8")),
            "/mascot.css" => Some((MASCOT_STYLES, "text/css; charset=utf-8")),
            "/chat.js" => Some((CHAT_SCRIPT, "text/javascript; charset=utf-8")),
            "/chat.css" => Some((CHAT_STYLES, "text/css; charset=utf-8")),
            "/panels.js" => Some((PANELS_SCRIPT, "text/javascript; charset=utf-8")),
            "/panels.css" => Some((PANELS_STYLES, "text/css; charset=utf-8")),
            _ => None,
        };
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
    app.api_response(&method, &path, &body)
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
    println!("{}", json!({"url": format!("http://127.0.0.1:{port}")}));
    std::io::stdout().flush()?;
    loop {
        let request = server.recv_timeout(TICK_INTERVAL)?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        app.tick(now_ms);
        let Some(mut request) = request else {
            continue;
        };
        let is_api = request.url().starts_with("/api/");
        let (mut status, mut body, content_type) = match payload(&mut request, port, &mut app) {
            Ok(response) => {
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
        for (name, value) in media_headers {
            response.add_header(Header::from_bytes(name, value).expect("media response header"));
        }
        for (name, value) in [
            ("Content-Type", content_type.as_str()),
            ("Cache-Control", if is_api { "no-store" } else { "no-cache" }),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
            ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("validated response header"));
        }
        if let Err(error) = request.respond(response) {
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
