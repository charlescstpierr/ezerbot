//! Gmail connection and its closed read adapter. The sign-in sends only the public
//! Google `client_id` of the build, with PKCE and a return on 127.0.0.1: no
//! `client_secret` is read, sent or stored, and nothing is kept in the vault before the
//! first connection. Tokens are written once, after Google names the account, and a
//! vault that refuses the write keeps nothing: there is no file copy. The tools are a
//! fixed list: reads, and one send that waits on a draft card (`email`). A new message is
//! also a normalized event, delivered once.
//! The same consent also reads the agenda and a named Drive file (`agenda`, `drive`):
//! they use this account and this vault, and they only ever issue GET requests.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::{io::Read, sync::Mutex, time::Duration};
mod agenda;
mod drive;
pub use agenda::{CalendarCase, CalendarEvent};

mod email;
pub(in super::super) use email::TOOL as EMAIL_TOOL;

pub(in super::super) const TOOL: &str = "gmail_list_recent_messages";
pub(in super::super) fn is_tool(name: &str) -> bool {
    matches!(name, TOOL | agenda::TOOL | drive::TOOL)
}
/// Gmail's REST host; the tests reach their own provider through `endpoint`.
pub(super) const API: &str = "https://gmail.googleapis.com";
const LATEST: u32 = 5;
const MOST: u32 = 10;
/// Characters of a message body kept for the model and for an event.
const EXCERPT: usize = 2000;
/// Messages looked at when watching for new ones.
const WATCHED: u32 = 25;
const NOT_CONNECTED: &str =
    "Gmail n’est pas connecté. Connectez-le dans Paramètres › Connecteurs, puis réessayez.";
const DISCONNECTED: &str =
    "Gmail est déconnecté. Reconnectez Gmail dans Paramètres › Connecteurs, puis réessayez.";
const ENABLE: &str =
    "Gmail n’est pas activé pour ce bot. Activez-le dans Paramètres › Connecteurs, puis réessayez.";
pub(super) const RECONNECT: &str =
    "La connexion Gmail a expiré ou a été révoquée. Reconnectez Gmail dans Paramètres › Connecteurs.";
const MODEL: &str = "Gmail · lire les derniers courriels de la boîte de réception (lecture seule, compte Gmail connecté dans Ezerbot ; cinq par défaut, dix au plus). Reprenez le champ `texte` du résultat tel quel dans votre réponse : il nomme le compte et donne expéditeur, objet et date de chaque courriel ; le champ `courriels` porte le texte de chacun pour le résumer.";
const SUMMARY: &str = "Lire les derniers courriels de la boîte de réception Gmail (cinq par défaut), en lecture seule, avec le compte Gmail connecté.";

/// Serializes the watch: two callers must not both see a message as new.
static WATCH: Mutex<()> = Mutex::new(());

/// A new message, as the routines read it. It carries no token and no provider detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailEvent {
    pub id: String,
    pub expediteur: String,
    pub objet: String,
    pub date: String,
    pub texte: String,
}

/// Public value of the build, replaceable by the environment for tests and the judge.
pub(super) fn client_id() -> Option<String> {
    let value = std::env::var("EZERBOT_GOOGLE_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("EZERBOT_GOOGLE_CLIENT_ID").map(str::to_owned))?;
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}
pub(super) fn not_configured() -> ApiError {
    invalid("Le client Google d’Ezerbot n’est pas configuré dans cette version. Il n’y a ni identifiant ni secret à saisir : la connexion à Gmail reste indisponible tant que ce client n’est pas préparé.")
}
pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    agenda::migrate(conn)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_gmail_mailbox (account_id TEXT PRIMARY KEY,since_ms INTEGER NOT NULL DEFAULT 0);CREATE TABLE IF NOT EXISTS wrapper_gmail_delivered (account_id TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(account_id,message_id));")?;
    email::migrate(conn)
}
/// What the connection forgets about the mailbox once it is disconnected.
pub(super) fn forget_mailbox(tx: &rusqlite::Transaction<'_>, server: &str) -> Result<(), ApiError> {
    for table in [
        "wrapper_gmail_delivered",
        "wrapper_gmail_mailbox",
        "wrapper_calendar_delivered",
        "wrapper_calendar_watch",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE account_id=(SELECT lower(account_email) FROM wrapper_connector_catalogue WHERE server_id=?1)"),
            [server],
        )?;
    }
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(12))
        .build()
}
/// What a read keeps of an answer: its JSON, or the first bytes of a text.
#[derive(Clone, Copy)]
enum Want {
    Json,
    Text,
}
/// Bytes of a text kept from a file: far more than the excerpt the model gets.
const TEXT_BYTES: u64 = 32 * 1024;
struct Reply {
    status: u16,
    body: Value,
    /// The start of a text answer (`Want::Text`), and whether more followed.
    text: String,
    cut: bool,
}
/// One authenticated call, no redirect, bounded body. Transport failures carry no detail.
fn request(
    method: &str,
    url: &str,
    access: &str,
    body: Option<&Value>,
    want: Want,
) -> Result<Reply, ()> {
    let request = agent()
        .request(method, url)
        .set("Authorization", &format!("Bearer {access}"))
        .set("Accept", "application/json");
    let response = match match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    } {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return Err(()),
    };
    let status = response.status();
    let limit = match want {
        Want::Json => 2 * 1024 * 1024,
        Want::Text => TEXT_BYTES,
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    let cut = bytes.len() as u64 > limit;
    if cut && matches!(want, Want::Json) {
        return Err(());
    }
    bytes.truncate(limit as usize);
    Ok(match want {
        Want::Json => Reply {
            status,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            text: String::new(),
            cut,
        },
        Want::Text => Reply {
            status,
            body: if status == 200 {
                Value::Null
            } else {
                serde_json::from_slice(&bytes).unwrap_or(Value::Null)
            },
            text: if status == 200 {
                String::from_utf8_lossy(&bytes).into_owned()
            } else {
                String::new()
            },
            cut,
        },
    })
}
fn get(url: &str, access: &str, want: Want) -> Result<Reply, ()> {
    request("GET", url, access, None, want)
}
fn unreachable_google() -> ApiError {
    ApiError::new(
        503,
        "Gmail est injoignable pour le moment. Vérifiez la connexion, puis réessayez.",
    )
}

/// The profile of the account that just signed in: its address is what the card shows.
pub(super) fn profile_email(m: Manifest, access: &str) -> Result<String, ApiError> {
    let url = format!("{}/gmail/v1/users/me/profile", endpoint(m, "api"));
    let reply = get(&url, access, Want::Json).map_err(|_| unreachable_google())?;
    reply.body["emailAddress"]
        .as_str()
        .filter(|s| reply.status == 200 && s.len() < 254 && s.contains('@') && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| invalid("Google n’a pas donné l’adresse du compte Gmail. Aucun jeton n’a été enregistré : reconnectez Gmail."))
}
/// The vault refused the tokens: the sentence says why, and nothing was written.
pub(super) fn refused_write(ctx: &Context<'_>) -> ApiError {
    let locked = crate::features::forms::vault(ctx)
        .is_ok_and(|vault| vault.storage_status().backend == "file_fallback");
    invalid(if locked {
        "Le trousseau macOS est verrouillé ou inaccessible : la connexion Gmail n’a pas été conservée et aucun jeton n’a été écrit. Déverrouillez-le, puis reconnectez Gmail."
    } else {
        "Impossible de conserver la connexion Gmail dans le coffre local. Vérifiez le stockage des secrets, puis reconnectez Gmail."
    })
}

fn schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "count":{"type":"integer","minimum":1,"maximum":MOST,"description":"Nombre de courriels à lire, du plus récent au plus ancien ; cinq si omis."}
    }})
}
/// Model-facing tools, advertised only while Gmail is connected and enabled for the bot.
/// All of them read; none sends, deletes or writes in the agenda or in Drive.
pub(in super::super) fn tool_specs() -> Vec<grokbot_engine::contracts::ToolSpec> {
    let read = |name: &str, description: &str, schema: Value| grokbot_engine::contracts::ToolSpec {
        name: name.into(),
        description: description.into(),
        schema,
        requires_approval: false,
    };
    vec![
        read(TOOL, MODEL, schema()),
        read(agenda::TOOL, agenda::MODEL, agenda::schema()),
        read(drive::TOOL, drive::MODEL, drive::schema()),
        email::tool_spec(),
    ]
}
/// The connection has no process: it is native, and its tools are this fixed list.
pub(super) fn attach(ctx: &Context<'_>, server: &Server) -> Result<Value, ApiError> {
    let tools = json!([
        {"name":"list_recent_messages","description":SUMMARY,"inputSchema":schema()},
        {"name":"list_events","description":agenda::SUMMARY,"inputSchema":agenda::schema()},
        {"name":"read_file","description":drive::SUMMARY,"inputSchema":drive::schema()},
        email::tool_definition()
    ]);
    ctx.connection.execute(
        "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
        params![server.id, tools.to_string()],
    )?;
    phase(
        ctx,
        &server.id,
        "connected",
        "Connecté. Activez les outils pour les bots de votre choix.",
    )?;
    Ok(
        json!({"message":"Gmail est connecté, avec l’agenda et Drive en lecture. Activez ses outils pour un bot : la lecture des derniers courriels, des rendez-vous d’un jour et d’un fichier Drive nommé part sans carte d’approbation, et un courriel n’est envoyé qu’après votre clic sur sa carte brouillon.","outils":tools}),
    )
}

/// A connected Gmail account, ready to be called.
struct Account {
    server: Server,
    m: Manifest,
    email: String,
    /// Enabled for the bot that asks.
    enabled: bool,
}
/// The account that holds a live connection, or the sentence that says why there is none.
fn account(ctx: &Context<'_>) -> Result<Account, ApiError> {
    let row: Option<(String, String, Option<String>)> = ctx.connection.query_row(
        "SELECT server_id,phase,account_email FROM wrapper_connector_catalogue WHERE id='gmail'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional()?;
    let Some((server_id, phase, email)) = row else {
        return Err(invalid(NOT_CONNECTED));
    };
    let (true, Some(email)) = (phase == "connected", email) else {
        return Err(invalid(if phase == "disconnected" {
            DISCONNECTED
        } else {
            RECONNECT
        }));
    };
    let server = super::super::servers(ctx.connection)?
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| invalid(DISCONNECTED))?;
    let enabled = ctx
        .bot_id
        .is_none_or(|bot| super::super::enabled(ctx.connection, &server.id, bot));
    Ok(Account {
        server,
        m: manifest("gmail")?,
        email,
        enabled,
    })
}
/// The account a bot may read with: connected, and given to that bot.
fn usable(ctx: &Context<'_>) -> Result<Account, ApiError> {
    let account = account(ctx)?;
    if account.enabled {
        Ok(account)
    } else {
        Err(invalid(ENABLE))
    }
}
/// Why the Gmail tool is not offered to this bot, as a sentence rather than an unknown tool.
pub(in super::super) fn unavailable(ctx: &Context<'_>) -> String {
    match usable(ctx) {
        Err(error) => error.message,
        Ok(_) => ENABLE.into(),
    }
}

/// One authenticated read. An access past its expiry is renewed first, and a 401 renews
/// once more and calls again: a rejected call has not touched the mailbox.
fn read(
    ctx: &Context<'_>,
    account: &Account,
    api: &str,
    route: &str,
    want: Want,
) -> Result<Reply, ApiError> {
    exchange(ctx, account, api, route, None, want)
}
/// A read, or with a `body` the one POST of a send. A 401 is a rejection before anything
/// happened, so the renewal and the second call are safe for a send too; a transport
/// failure is not, and a send then answers that its receipt is missing.
fn exchange(
    ctx: &Context<'_>,
    account: &Account,
    api: &str,
    route: &str,
    body: Option<&Value>,
    want: Want,
) -> Result<Reply, ApiError> {
    let (server, m) = (&account.server, account.m);
    let (method, lost, during, nothing) = match body {
        Some(_) => (
            "POST",
            email::unconfirmed as fn() -> ApiError,
            "l’envoi",
            "envoyé",
        ),
        None => (
            "GET",
            unreachable_google as fn() -> ApiError,
            "la lecture",
            "lu",
        ),
    };
    super::ensure(ctx, server)?;
    let url = format!("{}/{route}", endpoint(m, api));
    let attempt = || -> Result<(String, Reply), ApiError> {
        // A sign-in replaces the account and its tokens under this lock: the read of the
        // credentials sees one account, and the call below keeps that token.
        let access = {
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            // The token of a replacement account is never read for a request that was
            // bound to another one, nor for a bot that is no longer given Gmail.
            let current = self::account(ctx)?;
            if current.email != account.email || !current.enabled {
                return Err(invalid(&format!(
                    "Le compte Gmail a changé pendant {during} : rien n’a été {nothing}. Réessayez."
                )));
            }
            credentials(ctx, &server.id)?
                .and_then(|env| env.get("ACCESS_TOKEN").cloned())
                .ok_or_else(|| invalid(RECONNECT))?
        };
        let reply = request(method, &url, &access, body, want).map_err(|_| lost())?;
        Ok((access, reply))
    };
    let (used, mut reply) = attempt()?;
    if reply.status == 401 {
        {
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            let mut env = credentials(ctx, &server.id)?.ok_or_else(|| invalid(RECONNECT))?;
            if env.get("ACCESS_TOKEN") == Some(&used)
                && super::oauth::refresh(ctx, server, m, &mut env).is_err()
            {
                phase(ctx, &server.id, "reconnect", RECONNECT)?;
                return Err(invalid(RECONNECT));
            }
        }
        reply = attempt()?.1;
        if reply.status == 401 {
            phase(ctx, &server.id, "reconnect", RECONNECT)?;
            return Err(invalid(RECONNECT));
        }
    }
    Ok(reply)
}
/// The statuses that say nothing about one message are already a sentence here.
fn refused(reply: &Reply, email: &str) -> Option<ApiError> {
    refused_for(reply, email, "la boîte")
}
/// The same sentences for what is read: `what` is "la boîte", "l’agenda" or "le Drive".
fn refused_for(reply: &Reply, email: &str, what: &str) -> Option<ApiError> {
    match reply.status {
        200 | 404 => None,
        429 => Some(invalid(&format!("Google limite le débit des requêtes pour le compte {email}. Réessayez dans quelques minutes."))),
        403 => Some(invalid(&format!("Google a refusé l’accès à {what} du compte {email}. Reconnectez Gmail et accordez toutes les autorisations demandées : courriel, agenda et Drive."))),
        500..=599 => Some(ApiError::new(503, "Gmail est indisponible pour le moment. Réessayez plus tard.")),
        code => Some(invalid(&format!("Gmail a répondu de façon inattendue (code {code}). Réessayez plus tard."))),
    }
}

/// Identifiers Gmail gives are short ASCII tokens; anything else never reaches a URL.
fn message_id(text: &str) -> Option<&str> {
    (!text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')))
    .then_some(text)
}
fn clean(text: &str, limit: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(limit)
        .collect()
}
/// Printable text, cut to the excerpt: only tabs and line breaks survive.
fn excerpt(text: &str) -> String {
    excerpt_of(text, EXCERPT)
}
fn excerpt_of(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| *c != '\r')
        .map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn header<'a>(payload: &'a Value, name: &str) -> Option<&'a str> {
    payload["headers"]
        .as_array()?
        .iter()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str())
}
/// The first plain-text part, depth first. Gmail encodes bodies as URL-safe base64.
fn plain_text(part: &Value, depth: u8, budget: &mut u16) -> Option<String> {
    *budget = budget.checked_sub(1)?;
    if part["mimeType"] == "text/plain" {
        if let Some(data) = part["body"]["data"].as_str() {
            let bytes = URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')).ok()?;
            return Some(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    if depth == 0 {
        return None;
    }
    part["parts"]
        .as_array()?
        .iter()
        .find_map(|child| plain_text(child, depth - 1, budget))
}
/// A message and the instant Gmail received it, in milliseconds.
fn mail_of(message: &Value) -> Option<(MailEvent, i64)> {
    let id = message_id(message["id"].as_str()?)?.to_owned();
    let payload = &message["payload"];
    let text = plain_text(payload, 8, &mut 64)
        .or_else(|| message["snippet"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let received = message["internalDate"]
        .as_str()
        .and_then(|ms| ms.parse().ok())
        .unwrap_or(0);
    Some((
        MailEvent {
            id,
            expediteur: clean(header(payload, "From").unwrap_or("inconnu"), 200),
            objet: clean(header(payload, "Subject").unwrap_or("(sans objet)"), 300),
            date: clean(header(payload, "Date").unwrap_or(""), 100),
            texte: excerpt(&text),
        },
        received,
    ))
}
/// The newest identifiers of the inbox, newest first.
fn latest_ids(ctx: &Context<'_>, account: &Account, count: u32) -> Result<Vec<String>, ApiError> {
    let reply = read(
        ctx,
        account,
        "api",
        &format!("gmail/v1/users/me/messages?labelIds=INBOX&maxResults={count}"),
        Want::Json,
    )?;
    if let Some(error) = refused(&reply, &account.email) {
        return Err(error);
    }
    if reply.status != 200 {
        return Err(invalid("La réponse de Gmail est illisible."));
    }
    Ok(reply.body["messages"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| message_id(item["id"].as_str()?).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}
/// One message; `None` when it was deleted since it was listed.
fn message(
    ctx: &Context<'_>,
    account: &Account,
    id: &str,
) -> Result<Option<(MailEvent, i64)>, ApiError> {
    let reply = read(
        ctx,
        account,
        "api",
        &format!("gmail/v1/users/me/messages/{id}?format=full"),
        Want::Json,
    )?;
    if let Some(error) = refused(&reply, &account.email) {
        return Err(error);
    }
    if reply.status == 404 {
        return Ok(None);
    }
    mail_of(&reply.body)
        .map(Some)
        .ok_or_else(|| invalid("La réponse de Gmail est illisible."))
}

pub(in super::super) use email::{
    call as email_call, call_id as email_call_id, edit as email_edit, prepare as email_prepare,
    validate as email_validate,
};

pub(in super::super) fn call(
    ctx: &Context<'_>,
    name: &str,
    arguments: &Value,
) -> grokbot_engine::contracts::ToolOutcome {
    let result = match name {
        agenda::TOOL => agenda::list(ctx, arguments),
        drive::TOOL => drive::read_file(ctx, arguments),
        _ => list_recent(ctx, arguments),
    };
    match result {
        Ok(output) => grokbot_engine::contracts::ToolOutcome::Done { output },
        Err(error) => grokbot_engine::contracts::ToolOutcome::Failed {
            reason: error.message,
        },
    }
}
fn list_recent(ctx: &Context<'_>, arguments: &Value) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        count: Option<u32>,
    }
    let args: Args = serde_json::from_value(arguments.clone()).map_err(|_| {
        invalid(
            "Arguments invalides : indiquez, si besoin, le nombre de courriels (entre 1 et 10).",
        )
    })?;
    let count = args.count.unwrap_or(LATEST);
    if !(1..=MOST).contains(&count) {
        return Err(invalid(
            "Nombre de courriels invalide : choisissez entre 1 et 10.",
        ));
    }
    let account = usable(ctx)?;
    let mut mails = Vec::new();
    for id in latest_ids(ctx, &account, count)? {
        mails.extend(message(ctx, &account, &id)?.map(|(mail, _)| mail));
    }
    let email = account.email.as_str();
    let text = if mails.is_empty() {
        format!("Aucun courriel dans la boîte de réception (compte Gmail : {email}).")
    } else {
        let mut text =
            format!("Derniers courriels de la boîte de réception · compte Gmail : {email}");
        for (n, mail) in mails.iter().enumerate() {
            text.push_str(&format!(
                "\n{}. {} · de {} · {}",
                n + 1,
                mail.objet,
                mail.expediteur,
                mail.date
            ));
        }
        text
    };
    let courriels: Vec<Value> = mails
        .iter()
        .map(|m| json!({"id":m.id,"expediteur":m.expediteur,"objet":m.objet,"date":m.date,"texte":m.texte}))
        .collect();
    Ok(json!({"compte":email,"courriels":courriels,"texte":text}))
}

/// The messages that arrived since the last call, the oldest first, each identifier
/// once. The first call only learns what the inbox already holds, so nothing old is
/// "new". The consumer owns what it receives: a returned message is not returned again.
pub(in super::super) fn new_messages(ctx: &Context<'_>) -> Result<Vec<MailEvent>, ApiError> {
    let _watch = WATCH.lock().unwrap_or_else(|e| e.into_inner());
    let account = account(ctx)?;
    let owner = account.email.to_lowercase();
    let since: Option<i64> = ctx
        .connection
        .query_row(
            "SELECT since_ms FROM wrapper_gmail_mailbox WHERE account_id=?1",
            [&owner],
            |r| r.get(0),
        )
        .optional()?;
    let listed = latest_ids(ctx, &account, WATCHED)?;
    let Some(since) = since else {
        // The first look learns the box. Its floor is when the newest message arrived: an
        // older message that only enters the window later (after an archive) is not new.
        let floor = match listed.first() {
            Some(id) => message(ctx, &account, id)?.map(|(_, received)| received),
            None => None,
        }
        .unwrap_or(ctx.now_ms as i64);
        let tx = write_transaction(ctx.connection)?;
        tx.execute(
            "INSERT OR REPLACE INTO wrapper_gmail_mailbox(account_id,since_ms) VALUES (?1,?2)",
            params![owner, floor],
        )?;
        for id in &listed {
            tx.execute(
                "INSERT OR IGNORE INTO wrapper_gmail_delivered(account_id,message_id) VALUES (?1,?2)",
                params![owner, id],
            )?;
        }
        tx.commit()?;
        return Ok(Vec::new());
    };
    let mut stmt = ctx.connection.prepare(
        "SELECT EXISTS(SELECT 1 FROM wrapper_gmail_delivered WHERE account_id=?1 AND message_id=?2)",
    )?;
    let mut fresh = Vec::new();
    for id in listed.iter().rev() {
        if !stmt.query_row(params![owner, id], |r| r.get::<_, bool>(0))? {
            fresh.push(id.clone());
        }
    }
    drop(stmt);
    let mut events = Vec::new();
    let mut handled = Vec::new();
    let mut newest = since;
    for id in fresh {
        match message(ctx, &account, &id) {
            Ok(mail) => {
                // A message received before the floor is known mail, not an arrival.
                if let Some((mail, received)) =
                    mail.filter(|(_, received)| *received == 0 || *received > since)
                {
                    newest = newest.max(received);
                    events.push(mail);
                }
                handled.push(id);
            }
            // What could not be read now is tried again next time; what was read is kept.
            Err(error) if events.is_empty() => return Err(error),
            Err(_) => break,
        }
    }
    let tx = write_transaction(ctx.connection)?;
    tx.execute(
        "UPDATE wrapper_gmail_mailbox SET since_ms=?2 WHERE account_id=?1",
        params![owner, newest],
    )?;
    for id in &handled {
        tx.execute(
            "INSERT OR IGNORE INTO wrapper_gmail_delivered(account_id,message_id) VALUES (?1,?2)",
            params![owner, id],
        )?;
    }
    // Only the most recent identifiers matter: older ones are no longer listed.
    tx.execute(
        "DELETE FROM wrapper_gmail_delivered WHERE account_id=?1 AND rowid NOT IN (SELECT rowid FROM wrapper_gmail_delivered WHERE account_id=?1 ORDER BY rowid DESC LIMIT 500)",
        [&owner],
    )?;
    tx.commit()?;
    Ok(events)
}

/// Whether a Gmail account is connected, whoever may use it: the routines arm a new-mail
/// trigger only when a message could reach it.
pub(in super::super) fn connected(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_connector_catalogue WHERE id='gmail' AND phase='connected' AND account_email IS NOT NULL)",
        [],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// Nobody reads the inbox any more: the next look learns it afresh, so that mail which
/// arrived meanwhile is not delivered as new to a consumer that comes later.
pub(in super::super) fn stop_watching(conn: &Connection) -> Result<(), ApiError> {
    let _watch = WATCH.lock().unwrap_or_else(|e| e.into_inner());
    for table in ["wrapper_gmail_delivered", "wrapper_gmail_mailbox"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE account_id=(SELECT lower(account_email) FROM wrapper_connector_catalogue WHERE id='gmail')"),
            [],
        )?;
    }
    Ok(())
}

/// The agenda events that happened since the last call: created, modified, cancelled or
/// begun, each version once. The first call only learns the agenda.
pub(in super::super) fn calendar_events(ctx: &Context<'_>) -> Result<Vec<CalendarEvent>, ApiError> {
    agenda::new_events(ctx)
}
