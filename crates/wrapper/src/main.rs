use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{
    body_limit, header_safe, too_large, validate_headers_for_path, ApiError, ApiResponse, App,
    Provider,
};
use serde_json::json;
use tiny_http::{Header, Request, Response, Server, StatusCode};

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
}

fn options() -> Result<Option<Options>, String> {
    let mut args = std::env::args().skip(1);
    let mut port = 0;
    let mut db = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/Grokbot Local/workspace.sqlite");
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
                println!("grokbot-wrapper [--port 0] [--db chemin.sqlite]\nApplication locale, transport simulé. Écoute exclusivement 127.0.0.1.");
                return Ok(None);
            }
            _ => return Err(format!("Argument inconnu : {arg}")),
        }
    }
    Ok(Some(Options { port, db }))
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
        let endpoints = grokbot_wrapper::Endpoints {
            authorize: format!("{base}/authorize"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            jwks: format!("{base}/jwks"),
            issuer: base.clone(),
            api: format!("{base}/v1"),
        };
        return Provider::with_endpoints(db, endpoints, false);
    }
    Provider::new(db)
}

fn serve(options: Options) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    grokbot_forms::vault::enable_os_vault();
    let mut app = App::open_with(&options.db, provider(&options.db))?;
    // First-start migration runs before serving the page. Keychain denial falls
    // back to the owner-only file and is projected in Paramètres, never logged.
    grokbot_wrapper::features::forms::vault(&app.feature_context())?;
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
        let (status, body, content_type) = match payload(&mut request, port, &mut app) {
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
        let mut response = Response::from_data(body).with_status_code(StatusCode(status));
        for (name, value) in [
            ("Content-Type", content_type.as_str()),
            ("Cache-Control", if is_api { "no-store" } else { "no-cache" }),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
            ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("validated response header"));
        }
        if let Err(error) = request.respond(response) {
            eprintln!("Réponse HTTP interrompue : {error}");
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
