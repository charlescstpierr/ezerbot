//! Private iPhone host. REA interbot-mobile-protocol I1/I2/I7 establishes named
//! device credentials/presence, NOT a recovered pairing protocol. Local choice:
//! 2-minute single-use code, Mac confirmation, 256-bit HttpOnly device cookie.
use super::Context;
use crate::{ApiError, ApiResponse, App};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

mod push;
pub const PORT: u16 = 47863;
/// Private iPhone port. Debug builds accept GROKBOT_IPHONE_PORT so tests never
/// collide with an installed app already listening on 47863.
pub fn port() -> u16 {
    if cfg!(debug_assertions) {
        if let Some(port) = std::env::var("GROKBOT_IPHONE_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .filter(|p| *p != 0)
        {
            return port;
        }
    }
    PORT
}
/// Sentence the iPhone client compares verbatim. Server copies must use this.
pub const ACCESS_DISABLED: &str = "L’accès iPhone est désactivé sur le Mac.";
const LIFE: u64 = 120_000;
empty_hooks!(
    tools,
    call_tool,
    context,
    on_run_event,
    link_attachments,
    entry
);
fn bad(status: u16, text: &str) -> ApiError {
    ApiError::new(status, text)
}
fn random() -> Result<String, ApiError> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| bad(500, "Le générateur sécurisé est indisponible."))?;
    Ok(B64.encode(bytes))
}
fn hash(value: &str) -> String {
    B64.encode(Sha256::digest(value.as_bytes()))
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS iphone_settings (
        id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL DEFAULT 0,
        code_hash TEXT, expires INTEGER NOT NULL DEFAULT 0,
        attempts INTEGER NOT NULL DEFAULT 0, attempt_window INTEGER NOT NULL DEFAULT 0,
        listener_error TEXT, push_tick INTEGER NOT NULL DEFAULT 0);
        INSERT OR IGNORE INTO iphone_settings(id) VALUES(1);
        CREATE TABLE IF NOT EXISTS iphone_devices (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, credential_hash TEXT NOT NULL,
        receipt_hash TEXT UNIQUE NOT NULL, status TEXT NOT NULL,
        expires INTEGER NOT NULL, created_at INTEGER NOT NULL, last_seen INTEGER NOT NULL DEFAULT 0,
        conversation_id TEXT, visible INTEGER NOT NULL DEFAULT 0,
        subscription TEXT, push_error TEXT);
        CREATE TABLE IF NOT EXISTS iphone_push_claims (
        device_id TEXT NOT NULL, notification_id TEXT NOT NULL, PRIMARY KEY(device_id,notification_id));")?;
    push::migrate(conn)
}

pub fn enabled(app: &App) -> bool {
    app.connection
        .query_row("SELECT enabled FROM iphone_settings WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap_or(false)
}
pub fn listener_error(app: &App, error: Option<&str>) {
    let _ = app.connection.execute(
        "UPDATE iphone_settings SET listener_error=?1 WHERE id=1 AND listener_error IS NOT ?1",
        [error],
    );
}

fn snapshot(conn: &Connection) -> Result<Value, ApiError> {
    let now = crate::now();
    let mut stmt = conn.prepare("SELECT id,name,status,last_seen,expires,push_error,subscription IS NOT NULL FROM iphone_devices WHERE status='active' OR (status IN ('pending','confirmed') AND expires>?1) ORDER BY created_at")?;
    let devices = stmt.query_map([now], |r| {
        let seen: u64 = r.get(3)?;
        Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,
            "last_seen":seen,"connected": seen>0 && now.saturating_sub(seen)<20_000,
            "expires":r.get::<_,u64>(4)?,"push_error":r.get::<_,Option<String>>(5)?,"push":r.get::<_,bool>(6)?}))
    })?.collect::<Result<Vec<_>,_>>()?;
    let (enabled, error): (bool, Option<String>) = conn.query_row(
        "SELECT enabled,listener_error FROM iphone_settings WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(json!({"enabled":enabled,"port":port(),"listener_error":error,"devices":devices}))
}
pub fn state(ctx: &Context<'_>) -> Value {
    snapshot(ctx.connection).unwrap_or(json!({}))
}
pub fn tick(ctx: &Context<'_>) {
    if let Err(error) = push::tick(ctx) {
        eprintln!("Notifications iPhone : {}", error.message);
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Result<Option<&'a str>, ApiError> {
    let mut values = headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name));
    let value = values.next().map(|(_, v)| v.as_str());
    if values.next().is_some() {
        return Err(bad(403, "En-tête répété refusé."));
    }
    Ok(value)
}
/// The listener is loopback-only; Tailscale terminates TLS. Do not trust identity
/// headers supplied by a proxy: pairing is required even inside the tailnet.
pub fn validate_headers(method: &str, headers: &[(String, String)]) -> Result<(), ApiError> {
    let host = header(headers, "host")?.ok_or_else(|| bad(403, "Hôte manquant."))?;
    let dns = host.ends_with(".ts.net")
        && host.len() < 254
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    let local = cfg!(debug_assertions) && host == format!("127.0.0.1:{}", port());
    if !dns && !local {
        return Err(bad(403, "Utilisez l’adresse HTTPS Tailscale de ce Mac."));
    }
    if let Some(origin) = header(headers, "origin")? {
        let expected = format!("{}://{host}", if local { "http" } else { "https" });
        if origin != expected {
            return Err(bad(403, "Origine distante refusée."));
        }
    }
    if method != "GET" && method != "HEAD" {
        if header(headers, "x-grokbot-request")? != Some("local") {
            return Err(bad(403, "Requête distante refusée."));
        }
        let mime = header(headers, "content-type")?
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("");
        if mime != "application/json" && !mime.starts_with("image/") {
            return Err(bad(403, "Format de requête refusé."));
        }
    }
    Ok(())
}
fn credential(headers: &[(String, String)]) -> Result<Option<String>, ApiError> {
    let cookie = header(headers, "cookie")?.unwrap_or("");
    let mut values = cookie
        .split(';')
        .filter_map(|item| item.trim().strip_prefix("ezerbot_device="));
    let token = values.next();
    if values.next().is_some() {
        return Err(bad(401, "Identifiant d’appareil ambigu."));
    }
    Ok(token.filter(|s| s.len() == 43).map(hash))
}
fn device(app: &App, headers: &[(String, String)]) -> Result<String, ApiError> {
    let digest = credential(headers)?.ok_or_else(|| bad(401, "Associez cet iPhone sur le Mac."))?;
    app.connection
        .query_row(
            "SELECT id FROM iphone_devices WHERE credential_hash=?1 AND status='active'",
            [digest],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| bad(401, "Cet appareil n’est pas associé ou a été révoqué."))
}

/// Called ONLY by the second HTTP listener, before App's ordinary dispatcher.
/// Exact allowlist prevents remote administration or direct native-control APIs.
pub fn remote(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
    headers: &[(String, String)],
) -> Result<ApiResponse, ApiError> {
    if !enabled(app) {
        return Err(bad(403, ACCESS_DISABLED));
    }
    if path == "/api/iphone/claim" && method == "POST" {
        return claim(app, body).map(ApiResponse::Json);
    }
    if path == "/api/iphone/exchange" && method == "POST" {
        return exchange(app, body).map(ApiResponse::Json);
    }
    let id = device(app, headers)?;
    let now = crate::now();
    app.connection.execute(
        "UPDATE iphone_devices SET last_seen=?2 WHERE id=?1 AND last_seen<?2-5000",
        params![id, now],
    )?;
    if path == "/api/iphone/presence" && method == "POST" {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Presence {
            conversation_id: Option<String>,
            visible: bool,
        }
        let input: Presence = crate::parse(body)?;
        if let Some(ref id) = input.conversation_id {
            crate::validate_conversation_id(id)?;
        }
        app.connection.execute(
            "UPDATE iphone_devices SET conversation_id=?2,visible=?3,last_seen=?4 WHERE id=?1",
            params![id, input.conversation_id, input.visible, now],
        )?;
        let push: bool = app.connection.query_row(
            "SELECT subscription IS NOT NULL FROM iphone_devices WHERE id=?1",
            [&id],
            |row| row.get(0),
        )?;
        return Ok(ApiResponse::Json(json!({"ok":true,"push":push})));
    }
    if path == "/api/iphone/push-key" && method == "GET" {
        return push::public_key(&app.connection).map(|key| ApiResponse::Json(json!({"key":key})));
    }
    if path == "/api/iphone/push" && method == "POST" {
        return push::subscribe(&app.connection, &id, body).map(ApiResponse::Json);
    }
    let allowed = match method {
        "GET" => {
            matches!(path, "/api/state" | "/api/ping" | "/api/notifications")
                || path.starts_with("/api/attachments/")
                || path.starts_with("/api/generated/")
                || path.starts_with("/api/screen/shot?")
                || path.starts_with("/api/voice/audio/")
        }
        "POST" => {
            matches!(
                path,
                "/api/send"
                    | "/api/reconcile"
                    | "/api/regenerate"
                    | "/api/stop"
                    | "/api/local/tasks/stop"
                    | "/api/delegation/cancel"
                    | "/api/provider/welcomed"
                    | "/api/storage/drafts"
                    | "/api/conversations/members/add"
                    | "/api/conversations/members/remove"
                    | "/api/conversations/coordinator"
                    | "/api/conversations"
                    | "/api/conversations/update"
                    | "/api/conversations/delete"
                    | "/api/preferences"
                    | "/api/forms/submit"
                    | "/api/forms/site-submit"
                    | "/api/forms/cancel"
                    | "/api/forms/skill-content"
                    | "/api/approvals/resolve"
                    | "/api/notifications/open"
                    | "/api/notifications/read"
                    | "/api/notifications/settings"
                    | "/api/documents"
                    | "/api/attachments/precaption"
            ) || path == "/api/attachments"
                || path.starts_with("/api/attachments?")
        }
        _ => false,
    };
    if !allowed {
        return Err(bad(
            403,
            "Cette action reste disponible uniquement sur le Mac.",
        ));
    }
    let mut response = app.api_response(method, path, body)?;
    if path == "/api/state" {
        if let ApiResponse::Json(ref mut value) = response {
            value["iphone"] = json!({"remote":true});
            // Administration is desktop-only, even though the transcript itself
            // (including approval cards) is the same authenticated workspace.
            for key in [
                "mcp",
                "storage",
                "local",
                "screen",
                "voice",
                "routines_background",
                "routine_webhooks",
                "updates",
            ] {
                value.as_object_mut().unwrap().remove(key);
            }
        }
    }
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    code: String,
    name: String,
}
fn claim(app: &App, body: &[u8]) -> Result<Value, ApiError> {
    let input: Claim = crate::parse(body)?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(bad(
            400,
            "Donnez un nom de 1 à 60 caractères à cet appareil.",
        ));
    }
    let now = crate::now();
    // Commit the attempt even on failure, globally rather than trusting an IP
    // forwarded by the local Tailscale proxy. Persistence prevents restart bypass.
    app.connection.execute("UPDATE iphone_settings SET attempts=CASE WHEN attempt_window<?1-60000 THEN 1 ELSE attempts+1 END, attempt_window=CASE WHEN attempt_window<?1-60000 THEN ?1 ELSE attempt_window END WHERE id=1",[now])?;
    let (attempts, valid): (u64, bool) = app.connection.query_row(
        "SELECT attempts,code_hash=?1 AND expires>?2 FROM iphone_settings WHERE id=1",
        params![
            hash(&input.code.to_uppercase().replace([' ', '-'], "")),
            now
        ],
        |r| Ok((r.get(0)?, r.get::<_, Option<bool>>(1)?.unwrap_or(false))),
    )?;
    if attempts > 5 {
        return Err(bad(429, "Trop de tentatives. Attendez une minute."));
    }
    if !valid {
        return Err(bad(
            403,
            "Code incorrect ou expiré. Demandez un nouveau code sur le Mac.",
        ));
    }
    let count:u64=app.connection.query_row("SELECT COUNT(*) FROM iphone_devices WHERE status='active' OR (status IN ('pending','confirmed') AND expires>?1)",[now],|r|r.get(0))?;
    if count >= 32 {
        return Err(bad(
            409,
            "Révoquez un appareil avant d’en associer un autre.",
        ));
    }
    let receipt = random()?;
    let id = random()?;
    let tx = super::write_transaction(&app.connection)?;
    if tx.execute(
        "UPDATE iphone_settings SET code_hash=NULL,expires=0 WHERE id=1 AND code_hash=?1 AND expires>?2",
        params![hash(&input.code.to_uppercase().replace([' ', '-'], "")), crate::now()],
    )? != 1 {
        return Err(bad(403, "Ce code a expiré ou a déjà été utilisé."));
    }
    // A domain-separated bearer is derived from the unguessable receipt, so
    // neither the receipt nor credential persists in plaintext. Pending/denied
    // records never authorize requests, even if their bearer were known.
    let credential = hash(&format!("ezerbot-device:{receipt}"));
    tx.execute("INSERT INTO iphone_devices(id,name,credential_hash,receipt_hash,status,expires,created_at) VALUES(?1,?2,?3,?4,'pending',?5,?6)",params![id,name,hash(&credential),hash(&receipt),now+LIFE,now])?;
    tx.commit()?;
    Ok(json!({"receipt":receipt,"expires":now+LIFE}))
}
fn exchange(app: &App, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Exchange {
        receipt: String,
    }
    let input: Exchange = crate::parse(body)?;
    if input.receipt.len() != 43 {
        return Err(bad(403, "Demande d’association invalide."));
    }
    let row: Option<(String, String, u64)> = app
        .connection
        .query_row(
            "SELECT id,status,expires FROM iphone_devices WHERE receipt_hash=?1",
            [hash(&input.receipt)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (id, status, expires) = row.ok_or_else(|| bad(403, "Association refusée ou expirée."))?;
    if expires <= crate::now() || status == "denied" {
        return Err(bad(403, "Association refusée ou expirée."));
    }
    if status == "pending" {
        return Ok(json!({"pending":true}));
    }
    if status != "confirmed" {
        return Err(bad(403, "Cette association a déjà été utilisée."));
    }
    if app.connection.execute(
        "UPDATE iphone_devices SET status='active',last_seen=?2 WHERE id=?1 AND status='confirmed' AND expires>?2",
        params![id, crate::now()],
    )? != 1 {
        return Err(bad(403, "Cette association a expiré ou a déjà été utilisée."));
    }
    Ok(json!({"credential":hash(&format!("ezerbot-device:{}",input.receipt))}))
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !path.starts_with("/api/iphone/") {
        return None;
    }
    Some((|| {
        let conn = &app.connection;
        let value = match (method, path) {
            ("GET", "/api/iphone/status") => snapshot(conn)?,
            ("GET", "/api/iphone/tailscale") => tailscale_status(),
            ("POST", "/api/iphone/enabled") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    enabled: bool,
                }
                let input: Input = crate::parse(body)?;
                conn.execute(
                    "UPDATE iphone_settings SET enabled=?1,code_hash=NULL,expires=0 WHERE id=1",
                    [input.enabled],
                )?;
                conn.execute("DELETE FROM iphone_devices WHERE status!='active'", [])?;
                snapshot(conn)?
            }
            ("POST", "/api/iphone/code") => {
                if !enabled(app) {
                    return Err(bad(409, "Activez d’abord l’accès iPhone."));
                }
                let mut bytes = [0; 5];
                getrandom::getrandom(&mut bytes)
                    .map_err(|_| bad(500, "Code sécurisé indisponible."))?;
                let code = bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
                let expires = crate::now() + LIFE;
                conn.execute(
                    "UPDATE iphone_settings SET code_hash=?1,expires=?2 WHERE id=1",
                    params![hash(&code), expires],
                )?;
                json!({"code":code,"expires":expires})
            }
            ("POST", "/api/iphone/confirm") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    id: String,
                    allow: bool,
                }
                let input: Input = crate::parse(body)?;
                if conn.execute("UPDATE iphone_devices SET status=?2 WHERE id=?1 AND status='pending' AND expires>?3",params![input.id,if input.allow {"confirmed"}else{"denied"},crate::now()])?==0 {return Err(bad(409,"Cette demande a expiré ou a déjà été traitée."));}
                snapshot(conn)?
            }
            ("POST", "/api/iphone/revoke") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    id: String,
                }
                let input: Input = crate::parse(body)?;
                let tx = super::write_transaction(conn)?;
                tx.execute("DELETE FROM iphone_devices WHERE id=?1", [&input.id])?;
                tx.execute(
                    "DELETE FROM iphone_push_claims WHERE device_id=?1",
                    [&input.id],
                )?;
                tx.commit()?;
                snapshot(conn)?
            }
            ("POST", "/api/iphone/serve") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    start: bool,
                }
                let input: Input = crate::parse(body)?;
                if input.start && !enabled(app) {
                    return Err(bad(409, "Activez d’abord l’accès iPhone."));
                }
                if input.start
                    && conn.query_row(
                        "SELECT listener_error IS NOT NULL FROM iphone_settings WHERE id=1",
                        [],
                        |r| r.get::<_, bool>(0),
                    )?
                {
                    return Err(bad(409,"Le port privé n’est pas disponible. Publication refusée pour ne pas exposer une autre application."));
                }
                let args = if input.start {
                    vec![
                        "serve".into(),
                        "--bg".into(),
                        format!("http://127.0.0.1:{}", port()),
                    ]
                } else {
                    vec!["serve".into(), "--https=443".into(), "off".into()]
                };
                // Explicitly authorized button, as required by the feature brief.
                let output = tailscale(&args)?;
                if !output.status.success() {
                    return Err(bad(502,"Tailscale a refusé la commande. Vérifiez sa connexion et les instructions affichées."));
                }
                tailscale_status()
            }
            _ => return Err(bad(404, "Cette action iPhone n’existe pas sur le Mac.")),
        };
        Ok(ApiResponse::Json(value))
    })())
}
pub(crate) fn tailscale(args: &[String]) -> Result<std::process::Output, ApiError> {
    // Debug NEVER invokes a real CLI, including status. No installation, login,
    // network mutation or real Serve invocation can occur during tests/QA.
    #[cfg(debug_assertions)]
    let executable = {
        let path = std::env::var_os("GROKBOT_FAKE_TAILSCALE")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| bad(503, "Mode test : faux Tailscale non configuré."))?;
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| bad(503, "HOME de test manquant."))?;
        let canonical = path
            .canonicalize()
            .map_err(|_| bad(503, "Faux Tailscale introuvable."))?;
        if std::env::var_os("CFFIXED_USER_HOME") != Some(home.as_os_str().to_owned())
            || !canonical.starts_with(
                home.canonicalize()
                    .map_err(|_| bad(503, "HOME de test invalide."))?,
            )
        {
            return Err(bad(
                403,
                "Le faux Tailscale doit se trouver dans le HOME jetable.",
            ));
        }
        canonical
    };
    #[cfg(not(debug_assertions))]
    let executable = [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .find(|p| p.is_file())
    .ok_or_else(|| bad(404, "Tailscale n’est pas installé."))?;
    // Bound execution: a stuck CLI must not wedge the single HTTP host.
    use std::process::{Command, Stdio};
    let mut child = Command::new(executable)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| bad(503, "Tailscale est indisponible."))?;
    let start = std::time::Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|_| bad(503, "État Tailscale indisponible."))?
            .is_some()
        {
            return child
                .wait_with_output()
                .map_err(|_| bad(503, "Réponse Tailscale illisible."));
        }
        if start.elapsed() > std::time::Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(bad(504, "Tailscale ne répond pas. Réessayez."));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
fn tailscale_status() -> Value {
    let result = (|| -> Result<Value, ApiError> {
        let output = tailscale(&["status".into(), "--json".into()])?;
        if !output.status.success() {
            return Err(bad(503, "Tailscale n’est pas connecté."));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|_| bad(503, "État Tailscale illisible."))?;
        let dns = value["Self"]["DNSName"]
            .as_str()
            .unwrap_or("")
            .trim_end_matches('.');
        let valid = dns.ends_with(".ts.net")
            && dns
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
        Ok(
            json!({"installed":true,"running":value["BackendState"]=="Running","tailnet":value["CurrentTailnet"]["Name"],"url":if valid {Some(format!("https://{dns}"))}else{None}}),
        )
    })();
    result.unwrap_or_else(
        |error| json!({"installed":error.status!=404,"running":false,"error":error.message}),
    )
}
