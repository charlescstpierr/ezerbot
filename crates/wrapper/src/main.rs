use std::io::{Read, Write};
use std::path::PathBuf;

use grokbot_wrapper::{validate_headers, ApiError, App, Provider, BODY_LIMIT};
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

fn payload(
    request: &mut Request,
    port: u16,
    app: &mut App,
) -> Result<(Vec<u8>, &'static str), ApiError> {
    let method = request.method().as_str().to_owned();
    let path = request.url().to_owned();
    let headers: Vec<_> = request
        .headers()
        .iter()
        .map(|header| (header.field.as_str().to_string(), header.value.to_string()))
        .collect();
    validate_headers(port, &method, &headers)?;
    if request
        .body_length()
        .is_some_and(|length| length > BODY_LIMIT)
    {
        return Err(error(413, "La requête dépasse 64 Ko."));
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
            _ => None,
        };
        if let Some((content, content_type)) = asset {
            return Ok((content.as_bytes().to_vec(), content_type));
        }
    }
    let mut body = Vec::new();
    request
        .as_reader()
        .take((BODY_LIMIT + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| error(400, "Le corps de la requête est illisible."))?;
    let value = app.api(&method, &path, &body)?;
    Ok((
        value.to_string().into_bytes(),
        "application/json; charset=utf-8",
    ))
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
    let mut app = App::open_with(&options.db, provider(&options.db))?;
    let server = Server::http(("127.0.0.1", options.port))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("Adresse locale non disponible.")?
        .port();
    println!("{}", json!({"url": format!("http://127.0.0.1:{port}")}));
    std::io::stdout().flush()?;
    for mut request in server.incoming_requests() {
        let is_api = request.url().starts_with("/api/");
        let (status, body, content_type) = match payload(&mut request, port, &mut app) {
            Ok((body, content_type)) => (200, body, content_type),
            Err(error) => (
                error.status,
                json!({"error": error.message}).to_string().into_bytes(),
                "application/json; charset=utf-8",
            ),
        };
        let mut response = Response::from_data(body).with_status_code(StatusCode(status));
        for (name, value) in [
            ("Content-Type", content_type),
            ("Cache-Control", if is_api { "no-store" } else { "no-cache" }),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "DENY"),
            ("Referrer-Policy", "no-referrer"),
            ("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
        ] {
            response.add_header(Header::from_bytes(name, value).expect("constant response header"));
        }
        if let Err(error) = request.respond(response) {
            eprintln!("Réponse HTTP interrompue : {error}");
        }
    }
    Ok(())
}

fn main() {
    let result = match options() {
        Ok(Some(options)) => serve(options).map_err(|error| error.to_string()),
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("Grokbot Local : {error}");
        std::process::exit(1);
    }
}
