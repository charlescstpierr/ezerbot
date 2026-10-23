//! Independent local connector management. REA protocol.md distinguishes config,
//! bot selection and grants; NM04 establishes JSON lines, NM07 EOF cleanup.
//! Command launch/removal are host-generated tool calls through forms.rs too.
use super::{write_transaction, Context};
mod catalogue;
mod execution;
use crate::{mcp::Client, parse, ApiError, ApiResponse, App, ACCOUNT, DEFAULT_BOT};
use grokbot_engine::contracts::{RunEventKind, ToolCall, ToolOutcome, ToolSpec};
use grokbot_forms::redact::redact_value;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::atomic::Ordering,
};

// No global MCP tools or custom transcript entries: Services supplies bot-scoped
// specs, and the engine/forms modules own the tool and approval cards.
empty_hooks!(context, entry, link_attachments, tools);
fn wall_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
pub fn tick(ctx: &Context<'_>) {
    let manager = ctx.provider.mcp();
    if manager
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
    {
        manager.2.store(false, Ordering::Release);
        manager.3.store(0, Ordering::Release);
        return;
    }
    let now = wall_clock();
    if manager.3.load(Ordering::Acquire) > now || manager.1.swap(true, Ordering::AcqRel) {
        return;
    }
    manager.3.store(now.saturating_add(200), Ordering::Release);
    let provider = ctx.provider.clone();
    let path = ctx.db_path.to_path_buf();
    std::thread::spawn(move || {
        struct Reset(crate::Provider);
        impl Drop for Reset {
            fn drop(&mut self) {
                self.0.mcp().1.store(false, Ordering::Release);
            }
        }
        let _reset = Reset(provider.clone());
        let result = (|| -> Result<(), ApiError> {
            let conn = Connection::open(&path)?;
            conn.busy_timeout(std::time::Duration::from_secs(3))?;
            let ctx = Context {
                connection: &conn,
                provider: &provider,
                db_path: &path,
                now_ms: 0,
                bot_id: None,
                conversation_id: None,
                run_id: None,
            };
            refresh_catalogues(&ctx)
        })();
        if result.is_err() {
            provider
                .mcp()
                .3
                .store(wall_clock().saturating_add(1000), Ordering::Release);
        }
        provider.mcp().2.store(result.is_err(), Ordering::Release);
    });
}
pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_mcp_servers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, command TEXT NOT NULL,
        arguments_json TEXT NOT NULL, environment_names_json TEXT NOT NULL,
        tools_json TEXT NOT NULL DEFAULT '[]', error TEXT);
        CREATE TABLE IF NOT EXISTS wrapper_mcp_bots (
        server_id TEXT NOT NULL REFERENCES wrapper_mcp_servers(id) ON DELETE CASCADE,
        bot_id TEXT NOT NULL, PRIMARY KEY(server_id,bot_id));
        CREATE TABLE IF NOT EXISTS wrapper_mcp_labels (tool TEXT PRIMARY KEY,display_json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_mcp_approval_chat (account_id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL);",
    )?;
    // A server added by address (HTTPS) has no command; its approved address lives here.
    let present: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_mcp_servers') WHERE name='endpoint')",
        [],
        |r| r.get(0),
    )?;
    if !present {
        conn.execute(
            "ALTER TABLE wrapper_mcp_servers ADD COLUMN endpoint TEXT",
            [],
        )?;
    }
    catalogue::migrate(conn)?;
    Ok(())
}
/// Settings actions have one stable, host-owned destination, never the most
/// recently edited user discussion. A deleted destination is recreated atomically.
pub(crate) fn approval_conversation(ctx: &Context<'_>) -> Result<String, ApiError> {
    let tx = write_transaction(ctx.connection)?;
    let existing: Option<String> = tx.query_row(
        "SELECT c.id FROM wrapper_mcp_approval_chat m JOIN wrapper_conversations c ON c.account_id=m.account_id AND c.id=m.conversation_id WHERE m.account_id=?1 AND c.bot_id=?2 AND c.kind='chat' AND c.deleted=0",
        params![ACCOUNT, DEFAULT_BOT.id], |r| r.get(0),
    ).optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id: String = tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
    tx.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,created_at,updated_at) VALUES (?1,?2,'Approbations',?3,?4,?4)",params![ACCOUNT,id,DEFAULT_BOT.id,ctx.now_ms])?;
    tx.execute("INSERT INTO wrapper_mcp_approval_chat VALUES (?1,?2) ON CONFLICT(account_id) DO UPDATE SET conversation_id=excluded.conversation_id", params![ACCOUNT,id])?;
    tx.commit()?;
    Ok(id)
}

/// The GitHub read tools: a native adapter, run outside any SQLite writer.
pub(crate) fn is_github_tool(name: &str) -> bool {
    catalogue::github::is_tool(name) || is_github_comment(name) || is_github_issue(name)
}
/// The Slack tools: a native adapter, run outside any SQLite writer.
pub(crate) fn is_slack_tool(name: &str) -> bool {
    name == catalogue::slack::TOOL || is_slack_post(name)
}
/// The Slack publishing tool opens a draft card and sends nothing until that card is validated.
pub(crate) fn is_slack_post(name: &str) -> bool {
    name == catalogue::slack::post::TOOL
}
/// Why the Slack tool the bot was not offered cannot run: a sentence, not an unknown tool.
pub(crate) fn slack_unavailable(ctx: &Context<'_>) -> String {
    catalogue::slack::unavailable(ctx)
}
pub(crate) fn run_slack_tool(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    if is_slack_post(&call.name) {
        catalogue::slack::post::call(ctx, call)
    } else {
        catalogue::slack::call(ctx, &call.arguments)
    }
}
/// Why a GitHub tool the bot was not offered cannot run: a sentence, not an unknown tool.
pub(crate) fn github_unavailable(ctx: &Context<'_>) -> String {
    catalogue::github::unavailable(ctx)
}
/// Why a routine cannot be given a GitHub trigger: `None` when an account is connected.
pub(crate) fn github_routine_gap(ctx: &Context<'_>) -> Result<Option<String>, ApiError> {
    catalogue::github::routine_gap(ctx)
}
pub(crate) fn run_github_tool(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    if is_github_comment(&call.name) {
        catalogue::github::comment::call(ctx, call)
    } else if is_github_issue(&call.name) {
        catalogue::github::issue::call(ctx, call)
    } else {
        catalogue::github::call(ctx, &call.name, &call.arguments)
    }
}
/// The Gmail tools: a native adapter, run outside any SQLite writer. The read starts
/// without a card; the send is a draft card.
pub(crate) fn is_gmail_tool(name: &str) -> bool {
    catalogue::gmail::is_tool(name) || is_gmail_email(name)
}
pub(crate) fn is_gmail_email(name: &str) -> bool {
    name == catalogue::gmail::EMAIL_TOOL
}
/// Why the Gmail tool a bot was not offered cannot run: a sentence, not an unknown tool.
pub(crate) fn gmail_unavailable(ctx: &Context<'_>) -> String {
    catalogue::gmail::unavailable(ctx)
}
pub(crate) fn run_gmail_tool(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    if is_gmail_email(&call.name) {
        catalogue::gmail::email_call(ctx, call)
    } else {
        catalogue::gmail::call(ctx, &call.name, &call.arguments)
    }
}
pub use catalogue::gmail::{CalendarCase, CalendarEvent, MailEvent};
/// The messages that arrived in the connected Gmail inbox since the last call, the oldest
/// first, each identifier once: the normalized event a routine reads. It never carries a
/// token. The first call only learns what the inbox already holds.
pub fn gmail_new_messages(ctx: &Context<'_>) -> Result<Vec<MailEvent>, ApiError> {
    catalogue::gmail::new_messages(ctx)
}
/// Whether a Gmail account is connected: a new-mail trigger of a routine can only be armed then.
pub fn gmail_connected(conn: &Connection) -> bool {
    catalogue::gmail::connected(conn)
}
/// Forget what the inbox held, once no routine watches it: the next look learns it afresh.
pub fn gmail_stop_watching(conn: &Connection) -> Result<(), ApiError> {
    catalogue::gmail::stop_watching(conn)
}
/// The agenda events of the connected Google account since the last call: created,
/// modified, cancelled or begun, each version once, with its identifier, case, start
/// instant and zone. It never carries a token. The first call only learns the agenda.
pub fn calendar_events(ctx: &Context<'_>) -> Result<Vec<CalendarEvent>, ApiError> {
    catalogue::gmail::calendar_events(ctx)
}
/// The publishing tools of the connections (GitHub, Gmail, Slack): each opens a draft card and sends nothing
/// until that card is validated.
pub(crate) fn is_draft(name: &str) -> bool {
    is_github_comment(name) || is_github_issue(name) || is_gmail_email(name) || is_slack_post(name)
}
pub(crate) fn is_github_comment(name: &str) -> bool {
    name == catalogue::github::COMMENT_TOOL
}
pub(crate) fn is_github_issue(name: &str) -> bool {
    name == catalogue::github::ISSUE_TOOL
}
/// A draft whose card has a title the user may correct: an issue title, a subject.
pub(crate) fn has_draft_title(name: &str) -> bool {
    is_github_issue(name) || is_gmail_email(name)
}
pub(crate) fn validate_draft(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    if is_slack_post(&call.name) {
        catalogue::slack::post::validate(ctx, call)
    } else if is_gmail_email(&call.name) {
        catalogue::gmail::email_validate(ctx, call)
    } else if is_github_issue(&call.name) {
        catalogue::github::issue::validate(ctx, call)
    } else {
        catalogue::github::comment::validate(ctx, call)
    }
}
/// Content of the draft card, stored with the approval request.
pub(crate) fn draft_display(
    ctx: &Context<'_>,
    request_id: &str,
    call: &ToolCall,
) -> Result<Value, ApiError> {
    if is_slack_post(&call.name) {
        catalogue::slack::post::prepare(ctx, request_id, call)
    } else if is_gmail_email(&call.name) {
        catalogue::gmail::email_prepare(ctx, request_id, call)
    } else if is_github_issue(&call.name) {
        catalogue::github::issue::prepare(ctx, request_id, call)
    } else {
        catalogue::github::comment::prepare(ctx, request_id, call)
    }
}
pub(crate) fn draft_call(ctx: &Context<'_>, request_id: &str) -> Option<String> {
    catalogue::github::comment::call_id(ctx, request_id)
        .or_else(|| catalogue::gmail::email_call_id(ctx, request_id))
        .or_else(|| catalogue::slack::post::call_id(ctx, request_id))
}
/// What Charles corrected on a draft card: a comment or a Slack message has a text, an issue a title and a body,
/// an email a subject and a body.
pub(crate) fn edit_draft(
    ctx: &Context<'_>,
    request_id: &str,
    tool: &str,
    title: Option<&str>,
    text: Option<&str>,
) -> Result<(), ApiError> {
    if is_slack_post(tool) {
        catalogue::slack::post::edit(ctx, request_id, text.unwrap_or_default())
    } else if is_gmail_email(tool) {
        catalogue::gmail::email_edit(ctx, request_id, title, text)
    } else if is_github_issue(tool) {
        catalogue::github::issue::edit(ctx, request_id, title, text)
    } else {
        catalogue::github::comment::edit(ctx, request_id, text.unwrap_or_default())
    }
}
pub(crate) fn is_lifecycle(name: &str) -> bool {
    name.starts_with("mcp_start_") || name.starts_with("mcp_remove_")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    name: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    /// HTTPS address of a remote server, instead of a local command.
    url: Option<String>,
    /// Optional bearer secret of that server: it goes to the vault, never back.
    #[serde(default)]
    token: String,
    /// The user confirmed that an `http://127.0.0.1` server runs on this Mac.
    #[serde(default)]
    on_this_mac: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    id: String,
    bot_id: String,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    id: String,
}
#[derive(Clone)]
struct Server {
    id: String,
    name: String,
    command: String,
    endpoint: Option<String>,
    arguments: Vec<String>,
    names: Vec<String>,
    tools: Vec<Value>,
    error: Option<String>,
}
fn servers(conn: &Connection) -> Result<Vec<Server>, ApiError> {
    let mut stmt = conn.prepare("SELECT id,name,command,arguments_json,environment_names_json,tools_json,error,endpoint FROM wrapper_mcp_servers ORDER BY name,id")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, name, command, args, names, tools, error, endpoint)| {
            Ok(Server {
                id,
                name,
                command,
                endpoint,
                arguments: serde_json::from_str(&args)
                    .map_err(|_| invalid("Configuration MCP illisible."))?,
                names: serde_json::from_str(&names)
                    .map_err(|_| invalid("Configuration MCP illisible."))?,
                tools: serde_json::from_str(&tools)
                    .map_err(|_| invalid("Catalogue MCP illisible."))?,
                error,
            })
        })
        .collect()
}
fn invalid(message: &str) -> ApiError {
    ApiError::new(400, message)
}
fn secret_name(id: &str) -> String {
    format!("MCP_{id}_ENV").to_uppercase()
}
fn missing_private_value(vault: &grokbot_forms::vault::Vault, message: &str) -> ApiError {
    if vault.storage_status().backend == "file_fallback" {
        invalid("Le trousseau macOS est verrouillé ou inaccessible : impossible de récupérer les variables privées de ce connecteur. Déverrouillez-le, puis utilisez « Vérifier le stockage » dans Paramètres.")
    } else {
        invalid(message)
    }
}
fn environment(ctx: &Context<'_>, server: &Server) -> Result<HashMap<String, String>, ApiError> {
    let vault = super::forms::vault(ctx)?;
    let raw = vault.get(&secret_name(&server.id))?.ok_or_else(|| {
        missing_private_value(
            &vault,
            "Les variables privées du connecteur sont absentes du coffre.",
        )
    })?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|_| invalid("Les variables privées du connecteur sont illisibles."))?;
    serde_json::from_value(value["variables"].clone())
        .map_err(|_| invalid("Les variables privées du connecteur sont illisibles."))
}
fn strong_secret(value: &str) -> bool {
    value.encode_utf16().count() >= grokbot_forms::vault::MIN_SECRET_VALUE_CHARACTERS
}
fn private_values(ctx: &Context<'_>) -> Result<HashMap<String, String>, ApiError> {
    let vault = super::forms::vault(ctx)?;
    let mut values = HashMap::new();
    for secret in vault.list()? {
        if secret.purpose != grokbot_forms::vault::MCP_ENV_PURPOSE
            || !secret.name.starts_with("MCP_")
            || !secret.name.ends_with("_ENV")
        {
            continue;
        }
        let raw = vault.get(&secret.name)?.ok_or_else(|| {
            missing_private_value(&vault, "Une variable privée du coffre MCP a disparu.")
        })?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| invalid("Une enveloppe privée du coffre MCP est illisible."))?;
        let environment: HashMap<String, String> =
            serde_json::from_value(value["variables"].clone())
                .map_err(|_| invalid("Une enveloppe privée du coffre MCP est invalide."))?;
        values.extend(
            environment
                .into_iter()
                .filter(|(_, value)| strong_secret(value))
                .map(|(key, value)| (format!("{}:{key}", secret.name), value)),
        );
    }
    Ok(values)
}

// Values (including short/whitespace-sensitive environment values) are enclosed
// in one vault secret. Sanitize server-controlled metadata/results before any
// journal/database/UI/model boundary; stderr is never retained.
fn sanitize(value: &Value, env: &HashMap<String, String>) -> Result<Value, ApiError> {
    fn text(raw: &str, values: &[&str]) -> String {
        let mut output = String::new();
        let mut at = 0;
        while at < raw.len() {
            if let Some(value) = values.iter().find(|v| raw[at..].starts_with(**v)) {
                output.push_str("[masqué]");
                at += value.len();
            } else {
                let c = raw[at..].chars().next().unwrap();
                output.push(c);
                at += c.len_utf8();
            }
        }
        output
    }
    let mut patterns = BTreeSet::new();
    let short: Vec<_> = env
        .values()
        .filter(|v| !v.is_empty() && !strong_secret(v))
        .map(String::as_str)
        .collect();
    for value in env.values().filter(|v| strong_secret(v)) {
        patterns.insert(value.clone());
        let encoded = json!(value).to_string();
        patterns.insert(encoded[1..encoded.len() - 1].to_owned());
    }
    let mut values: Vec<_> = patterns.iter().map(String::as_str).collect();
    values.sort_by_key(|s| std::cmp::Reverse(s.len()));
    fn walk(value: &Value, values: &[&str], short: &[&str]) -> Result<Value, ApiError> {
        Ok(match value {
            Value::String(s) if short.contains(&s.as_str()) => json!("[masqué]"),
            Value::String(s) => json!(text(s, values)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| walk(v, values, short))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(items) => {
                let mut output = serde_json::Map::new();
                for (key, value) in items {
                    if output
                        .insert(text(key, values), walk(value, values, short)?)
                        .is_some()
                    {
                        return Err(invalid(
                            "Le masquage des variables privées rend le résultat MCP ambigu.",
                        ));
                    }
                }
                Value::Object(output)
            }
            Value::Number(n) if values.contains(&n.to_string().as_str()) => return Err(invalid(
                "Le résultat MCP contient une donnée numérique privée ; il n’a pas été conservé.",
            )),
            _ => value.clone(),
        })
    }
    walk(value, &values, &short)
}
fn safe_catalogue(
    ctx: &Context<'_>,
    tools: Vec<Value>,
    env: &HashMap<String, String>,
) -> Result<Vec<Value>, ApiError> {
    let original = json!(tools);
    let mut private = private_values(ctx)?;
    private.extend(env.iter().map(|(key, value)| (key.clone(), value.clone())));
    let safe = sanitize(&original, &private)?;
    let safe = redact_value(&super::forms::vault(ctx)?, &safe)?;
    if original
        .as_array()
        .unwrap()
        .iter()
        .zip(safe.as_array().unwrap())
        .any(|(a, b)| a["name"] != b["name"] || a["inputSchema"] != b["inputSchema"])
    {
        return Err(invalid("Le catalogue MCP contient une variable privée dans un nom ou un schéma. Corrigez le serveur pour pouvoir proposer ses outils sans divulgation."));
    }
    Ok(safe.as_array().unwrap().clone())
}

fn namespace(server: &Server, tool: &Value) -> String {
    let hash = format!(
        "{:x}",
        Sha256::digest(tool["name"].as_str().unwrap_or("").as_bytes())
    );
    format!("mcp_{}_{}", server.id, &hash[..16])
}
fn enabled(conn: &Connection, server: &str, bot: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_mcp_bots WHERE server_id=?1 AND bot_id=?2)",
        params![server, bot],
        |r| r.get(0),
    )
    .unwrap_or(false)
}
pub fn scoped_tools(ctx: &Context<'_>) -> Vec<ToolSpec> {
    let mut result = Vec::new();
    // Several accounts of one service share one closed list of tools.
    let mut native_offered = BTreeSet::new();
    for server in servers(ctx.connection).unwrap_or_default() {
        for action in ["start", "remove"]
            .into_iter()
            .filter(|_| ctx.run_id.is_some_and(|id| id.starts_with("mcp-host-")))
        {
            result.push(ToolSpec {
                name: format!("mcp_{action}_{}", server.id),
                description: format!(
                    "Connecteur {} : {}",
                    server.name,
                    if action == "start" {
                        "démarrer ou redémarrer le processus local"
                    } else {
                        "supprimer et déconnecter"
                    }
                ),
                schema: json!({"type":"object","properties":{},"additionalProperties":true}),
                requires_approval: true,
            });
        }
        if !ctx
            .bot_id
            .is_some_and(|bot| enabled(ctx.connection, &server.id, bot))
        {
            continue;
        }
        if let Some(service) = catalogue::native_service(ctx, &server.id) {
            let tools = catalogue::native_tools(ctx, &server);
            if !tools.is_empty() && native_offered.insert(service) {
                result.extend(tools);
            }
            continue;
        }
        let handle = ctx
            .provider
            .mcp()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&server.id)
            .cloned();
        let Some(handle) = handle.filter(|c| c.ready.load(Ordering::Acquire)) else {
            continue;
        };
        if handle
            .client
            .try_lock()
            .is_ok_and(|mut client| !client.alive())
        {
            continue;
        }
        for tool in &server.tools {
            result.push(ToolSpec {
                name: namespace(&server, tool),
                description: format!(
                    "Connecteur {} · {} : {}",
                    server.name,
                    tool["name"].as_str().unwrap_or(""),
                    tool["description"].as_str().unwrap_or("")
                ),
                schema: tool["inputSchema"].clone(),
                requires_approval: true,
            });
        }
    }
    result
}
pub fn on_run_event(ctx: &Context<'_>, event: &grokbot_engine::contracts::RunEvent) {
    if event.kind == RunEventKind::ToolCall {
        if let Some(name) = event.payload["name"]
            .as_str()
            .filter(|n| n.starts_with("mcp_"))
        {
            if let Some(label) = display(ctx, name) {
                let _ = ctx.connection.execute(
                    "INSERT OR IGNORE INTO wrapper_mcp_labels(tool,display_json) VALUES (?1,?2)",
                    params![name, label.to_string()],
                );
            }
        }
    }
}

pub fn display(ctx: &Context<'_>, name: &str) -> Option<Value> {
    let saved: Option<String> = ctx
        .connection
        .query_row(
            "SELECT display_json FROM wrapper_mcp_labels WHERE tool=?1",
            [name],
            |r| r.get(0),
        )
        .ok();
    if let Some(label) = saved.and_then(|s| serde_json::from_str(&s).ok()) {
        return Some(label);
    }
    for server in servers(ctx.connection).ok()? {
        for action in ["start", "remove"] {
            if name == format!("mcp_{action}_{}", server.id) {
                return Some(
                    json!({"server":server.name,"tool":if action=="start" {"Démarrer le connecteur"} else {"Supprimer le connecteur"}}),
                );
            }
        }
        if let Some(tool) = server.tools.iter().find(|t| namespace(&server, t) == name) {
            return Some(json!({"server":server.name,"tool":tool["name"]}));
        }
        if name.starts_with(&format!("mcp_{}_", server.id)) {
            return Some(json!({"server":server.name,"tool":"outil inconnu"}));
        }
    }
    None
}
// Recovery is useful for a missing/dead transport, not an ordinary RPC refusal.
// Use current connection state so repaired historical cards stop suggesting it.
fn needs_recovery(ctx: &Context<'_>, name: &str) -> bool {
    let id = name
        .strip_prefix("mcp_start_")
        .or_else(|| name.strip_prefix("mcp_remove_"))
        .or_else(|| name.strip_prefix("mcp_")?.split_once('_').map(|(id, _)| id));
    let Some(id) = id else {
        return false;
    };
    let exists: bool = ctx
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM wrapper_mcp_servers WHERE id=?1)",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !exists {
        return false;
    }
    let handle = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .cloned();
    let Some(handle) = handle else {
        return true;
    };
    handle
        .client
        .try_lock()
        .is_ok_and(|mut client| !client.alive())
}

pub(crate) fn call_display(ctx: &Context<'_>, name: &str, arguments: &Value) -> Option<Value> {
    if let Some(label) = super::recall::tool_label(name) {
        return Some(label);
    }
    let mut label = display(ctx, name)?;
    let recoverable = needs_recovery(ctx, name);
    let server_id = name
        .strip_prefix("mcp_start_")
        .or_else(|| name.strip_prefix("mcp_remove_"))
        .or_else(|| name.strip_prefix("mcp_")?.split_once('_').map(|(id, _)| id));
    let catalogue_id = server_id.and_then(|id| catalogue::id_for_server(ctx, id));
    if let Some(id) = catalogue_id {
        label
            .as_object_mut()?
            .insert("catalogue_id".into(), json!(id));
    }
    if is_lifecycle(name) {
        let title = arguments["action"]
            .as_str()
            .filter(|s| {
                matches!(
                    *s,
                    "Démarrer le connecteur"
                        | "Redémarrer le connecteur"
                        | "Supprimer le connecteur"
                ) || (catalogue_id.is_some()
                    && ["Connecter", "Reconnecter", "Déconnecter"]
                        .iter()
                        .any(|verb| {
                            *s == format!("{verb} {}", label["server"].as_str().unwrap_or(""))
                        }))
            })
            .unwrap_or_else(|| label["tool"].as_str().unwrap_or("Gérer le connecteur"));
        let mut display = json!({"server":label["server"],"tool":title,"origin":"settings","action":if name.starts_with("mcp_start_") {"start"} else {"remove"},"recoverable":recoverable});
        if let Some(id) = catalogue_id {
            display["catalogue_id"] = json!(id);
        }
        return Some(display);
    }
    label
        .as_object_mut()?
        .insert("recoverable".into(), json!(recoverable));
    Some(label)
}
/// Background only: no stdio under a SQLite writer or the process registry.
/// A busy client is skipped; immutable connection identities defeat stale EOFs.
fn refresh_catalogues(ctx: &Context<'_>) -> Result<(), ApiError> {
    let handles = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let private = active_private(ctx);
    let mut deferred = None;
    for (id, handle) in handles {
        if !handle.ready.load(Ordering::Acquire) {
            continue;
        }
        let Ok(mut client) = handle.client.try_lock() else {
            continue;
        };
        let refreshed = match client.catalog_changed() {
            Ok(false) if !handle.dirty.swap(false, Ordering::AcqRel) => continue,
            Ok(_) => {
                let tools = match client.list_tools() {
                    Ok(tools) => Ok(tools),
                    Err(e @ crate::mcp::Error::Rpc { .. }) => {
                        handle.dirty.store(true, Ordering::Release);
                        deferred = Some(ApiError::new(503, e.to_string()));
                        continue;
                    }
                    Err(e) => Err(ApiError::new(503, e.to_string())),
                };
                tools.and_then(|t| {
                    let mut p = private.clone();
                    p.extend(handle.environment.clone());
                    safe_catalogue(ctx, t, &p)
                })
            }
            Err(e) => Err(ApiError::new(503, e.to_string())),
        };
        drop(client);
        let saved = (|| -> Result<(), ApiError> {
            let tx = write_transaction(ctx.connection)?;
            let mut registry = ctx
                .provider
                .mcp()
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !registry
                .get(&id)
                .is_some_and(|c| c.identity == handle.identity)
            {
                return Ok(());
            }
            match &refreshed {
                Ok(tools) => {
                    tx.execute(
                        "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
                        params![id, json!(tools).to_string()],
                    )?;
                }
                Err(e) => {
                    tx.execute(
                        "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                        params![id, e.message],
                    )?;
                }
            }
            tx.commit()?;
            if refreshed.is_err() {
                registry.remove(&id);
            }
            Ok(())
        })();
        if saved.is_err() {
            handle.dirty.store(true, Ordering::Release);
        }
        if let Err(error) = saved {
            deferred = Some(error);
        }
    }
    deferred.map_or(Ok(()), Err)
}
fn active_private(ctx: &Context<'_>) -> HashMap<String, String> {
    ctx.provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .flat_map(|(id, c)| {
            c.environment
                .iter()
                .filter(|(_, value)| strong_secret(value))
                .map(move |(key, value)| (format!("active:{id}:{key}"), value.clone()))
        })
        .collect()
}

fn state_servers(ctx: &Context<'_>) -> Result<Vec<Value>, ApiError> {
    let mut live = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut result = Vec::new();
    for server in servers(ctx.connection)? {
        let status = match live.get_mut(&server.id) {
            Some(handle) if !handle.ready.load(Ordering::Acquire) => "starting",
            Some(handle) => match handle.client.try_lock() {
                Ok(mut client) => {
                    if client.alive() {
                        "connected"
                    } else {
                        "crashed"
                    }
                }
                Err(_) => "busy",
            },
            None if catalogue::native_status(ctx, &server.id).is_some() => {
                catalogue::native_status(ctx, &server.id).unwrap_or("disconnected")
            }
            None if server.error.as_deref() == Some(crate::mcp::DISCONNECTED) => "crashed",
            None if server.error.is_some() => "error",
            None => "disconnected",
        };
        let mut stmt = ctx
            .connection
            .prepare("SELECT bot_id FROM wrapper_mcp_bots WHERE server_id=?1 ORDER BY bot_id")?;
        let bots = stmt
            .query_map([&server.id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let tools: Vec<_> = server.tools.iter().map(|t|json!({"name":t["name"],"description":t["description"],"namespace":namespace(&server,t)})).collect();
        result.push(json!({"id":server.id,"name":server.name,"command":server.command,"url":server.endpoint,"arguments":server.arguments,"environment_names":server.names,"enabled_bots":bots,"status":status,"error":if status=="crashed" {Some("Le serveur MCP s’est arrêté. Redémarrez le connecteur.".to_owned())} else {server.error},"tools":tools}));
    }
    Ok(result)
}
pub fn state(ctx: &Context<'_>) -> Value {
    let load = || -> Result<Value, ApiError> {
        Ok(
            json!({"servers":state_servers(ctx)?,"catalogue":catalogue::state(ctx)?,"error":ctx.provider.mcp().2.load(Ordering::Acquire).then_some("L’actualisation des outils est différée. Le dernier état connu reste affiché.")}),
        )
    };
    load().unwrap_or_else(|_| json!({"error":"Les connecteurs MCP sont indisponibles."}))
}
pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method != "POST"
        || !matches!(
            path,
            "/api/mcp/add"
                | "/api/mcp/select"
                | "/api/mcp/restart"
                | "/api/mcp/remove"
                | "/api/connectors/connect"
                | "/api/connectors/cancel"
                | "/api/connectors/disconnect"
        )
    {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}
/// A server reached by address: the address is the approved origin, the optional
/// token a vault secret that is never shown again nor given to the model.
fn add_remote(ctx: &Context<'_>, add: Add) -> Result<Value, ApiError> {
    let name = add.name.trim();
    if !add.command.trim().is_empty() || !add.arguments.is_empty() || !add.environment.is_empty() {
        return Err(invalid(
            "Choisissez une commande locale ou une adresse https, pas les deux.",
        ));
    }
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(invalid("Nom du connecteur invalide (1 à 60 caractères)."));
    }
    let endpoint = crate::mcp::approved_endpoint(add.url.as_deref().unwrap_or(""), add.on_this_mac)
        .map_err(|message| invalid(&message))?;
    let token = add.token.trim();
    if token.len() > 2048 || !token.bytes().all(|c| c.is_ascii_graphic()) {
        return Err(invalid(
            "Le jeton est invalide : 2 Ko au plus, sans espace ni caractère de contrôle.",
        ));
    }
    // A shorter value is only masked when it is a whole string: it could leak inside a sentence.
    if !token.is_empty() && !strong_secret(token) {
        return Err(invalid(
            "Le jeton est trop court pour être masqué de façon fiable : 8 caractères au moins.",
        ));
    }
    if !token.is_empty() && (name.contains(token) || endpoint.contains(token)) {
        return Err(invalid(
            "Gardez le jeton uniquement dans son champ, pas dans le nom ni dans l’adresse.",
        ));
    }
    let vault = super::forms::vault(ctx)?;
    let public = json!([name, endpoint]);
    if redact_value(&vault, &public)? != public {
        return Err(invalid(
            "La configuration publique contient un secret. Utilisez le champ du jeton.",
        ));
    }
    let tx = write_transaction(ctx.connection)?;
    let count: i64 = tx.query_row("SELECT count(*) FROM wrapper_mcp_servers", [], |r| r.get(0))?;
    if count >= 20 {
        return Err(invalid("La limite est de 20 connecteurs."));
    }
    if servers(&tx)?
        .iter()
        .any(|s| s.name.to_lowercase() == name.to_lowercase())
    {
        return Err(ApiError::new(
            409,
            "Un connecteur porte déjà ce nom. Choisissez un nom distinct pour les approbations.",
        ));
    }
    let id: String = tx.query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))?;
    let names = if token.is_empty() {
        json!([])
    } else {
        json!(["ACCESS_TOKEN"])
    };
    tx.execute("INSERT INTO wrapper_mcp_servers(id,name,command,arguments_json,environment_names_json,endpoint) VALUES (?1,?2,'','[]',?3,?4)",params![id,name,names.to_string(),endpoint])?;
    let variables: BTreeMap<&str, &str> = if token.is_empty() {
        BTreeMap::new()
    } else {
        BTreeMap::from([("ACCESS_TOKEN", token)])
    };
    let raw = json!({"variables":variables}).to_string();
    if raw.len() > grokbot_forms::vault::MAX_SECRET_VALUE_BYTES {
        return Err(invalid(
            "Le jeton dépasse la limite du coffre local (4 Ko au total).",
        ));
    }
    vault.set(
        &secret_name(&id),
        grokbot_forms::vault::MCP_ENV_PURPOSE,
        &raw,
    )?;
    tx.commit()?;
    Ok(json!({"id":id,"mcp":state(ctx)}))
}
fn lifecycle_arguments(
    ctx: &Context<'_>,
    server: &Server,
    action: &str,
    restarting: bool,
) -> Value {
    // The approval shows the exact address the secret may be sent to.
    let command = server.endpoint.as_deref().unwrap_or(&server.command);
    let mut args = json!({"serveur":server.name,"commande":command,"arguments":server.arguments,"variables_privées":server.names,"action":if action=="start" {if restarting {"Redémarrer le connecteur"} else {"Démarrer le connecteur"}} else {"Supprimer le connecteur"}});
    if server.endpoint.is_some() {
        args["transport"] = json!("https");
    }
    if let Some(catalogue) = catalogue::preview(ctx, server, action) {
        args["action"] = catalogue["action"].clone();
        args["variables_privées"] = catalogue["variables_privées"].clone();
        args["catalogue"] = catalogue;
    }
    args
}

fn route(app: &App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    if path.starts_with("/api/connectors/") {
        return catalogue::routes(app, path, body);
    }
    let ctx = app.feature_context();
    if path == "/api/mcp/add" {
        let add: Add = parse(body)?;
        if add.url.is_some() {
            return add_remote(&ctx, add);
        }
        if !add.token.is_empty() || add.on_this_mac {
            return Err(invalid(
                "Le jeton et la confirmation « sur ce Mac » ne concernent qu’un serveur ajouté par adresse.",
            ));
        }
        let name = add.name.trim();
        let command = add.command.trim();
        if name.is_empty()
            || name.chars().count() > 60
            || name.chars().any(char::is_control)
            || command.is_empty()
            || command.len() > 4096
            || command.contains('\0')
            || add.arguments.len() > 64
            || add
                .arguments
                .iter()
                .any(|s| s.len() > 4096 || s.contains('\0'))
            || add.environment.len() > 32
        {
            return Err(invalid(
                "Nom (1 à 60 caractères), commande ou arguments du connecteur invalides.",
            ));
        }
        for (key, value) in &add.environment {
            if key.is_empty()
                || key.len() > 128
                || !key.bytes().enumerate().all(|(i, c)| {
                    c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit())
                })
                || value.len() > 2048
                || value.contains('\0')
            {
                return Err(invalid("Variable invalide : utilisez un nom de type CLE_API et une valeur sans caractère nul (2 Ko maximum)."));
            }
            if !value.is_empty()
                && (if strong_secret(value) {
                    name.contains(value)
                        || command.contains(value)
                        || add.arguments.iter().any(|a| a.contains(value))
                } else {
                    name == value || command == value || add.arguments.iter().any(|a| a == value)
                })
            {
                return Err(invalid("Gardez les valeurs privées uniquement dans les variables d’environnement, pas dans le nom, la commande ou les arguments."));
            }
        }
        let vault = super::forms::vault(&ctx)?;
        let public = json!([name, command, add.arguments]);
        if redact_value(&vault, &public)? != public {
            return Err(invalid("La configuration publique contient un secret. Utilisez une variable d’environnement."));
        }
        let tx = write_transaction(ctx.connection)?;
        let count: i64 =
            tx.query_row("SELECT count(*) FROM wrapper_mcp_servers", [], |r| r.get(0))?;
        if count >= 20 {
            return Err(invalid("La limite est de 20 connecteurs."));
        }
        if servers(&tx)?
            .iter()
            .any(|s| s.name.to_lowercase() == name.to_lowercase())
        {
            return Err(ApiError::new(409,"Un connecteur porte déjà ce nom. Choisissez un nom distinct pour les approbations."));
        }
        let id: String = tx.query_row("SELECT lower(hex(randomblob(8)))", [], |r| r.get(0))?;
        let names: Vec<_> = add.environment.keys().collect();
        tx.execute("INSERT INTO wrapper_mcp_servers(id,name,command,arguments_json,environment_names_json) VALUES (?1,?2,?3,?4,?5)",params![id,name,command,json!(add.arguments).to_string(),json!(names).to_string()])?;
        // JSON envelope preserves whitespace and allows short values without
        // weakening the existing human-secret validation policy.
        let raw = json!({"variables":add.environment}).to_string();
        if raw.len() > grokbot_forms::vault::MAX_SECRET_VALUE_BYTES {
            return Err(invalid(
                "Les variables privées dépassent la limite du coffre local (4 Ko au total).",
            ));
        }
        vault.set(
            &secret_name(&id),
            grokbot_forms::vault::MCP_ENV_PURPOSE,
            &raw,
        )?;
        tx.commit()?;
        return Ok(json!({"id":id,"mcp":state(&ctx)}));
    }
    if path == "/api/mcp/select" {
        let input: Selection = parse(body)?;
        app.require_bot(&input.bot_id)?;
        if !servers(ctx.connection)?.iter().any(|s| s.id == input.id) {
            return Err(invalid("Ce connecteur MCP n’existe plus."));
        }
        if input.enabled {
            ctx.connection.execute(
                "INSERT OR IGNORE INTO wrapper_mcp_bots VALUES (?1,?2)",
                params![input.id, input.bot_id],
            )?;
        } else {
            ctx.connection.execute(
                "DELETE FROM wrapper_mcp_bots WHERE server_id=?1 AND bot_id=?2",
                params![input.id, input.bot_id],
            )?;
        }
        return Ok(json!({"mcp":state(&ctx)}));
    }
    let input: Identity = parse(body)?;
    let server = servers(ctx.connection)?
        .into_iter()
        .find(|s| s.id == input.id)
        .ok_or_else(|| invalid("Ce connecteur MCP n’existe plus."))?;
    let action = if path.ends_with("restart") {
        "start"
    } else {
        "remove"
    };
    let restarting = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&server.id);
    let args = lifecycle_arguments(&ctx, &server, action, restarting);
    super::engine::queue_approved_action(&ctx, &format!("mcp_{action}_{}", server.id), args)
}

pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if !call.name.starts_with("mcp_") {
        return None;
    }
    // Hidden invoke_tool probes and other direct callers cannot bypass the gate.
    let result = (|| -> Result<Value, ApiError> {
        let run_id = ctx
            .run_id
            .ok_or_else(|| invalid("Cet appel MCP exige une approbation dans la conversation."))?;
        let events = grokbot_engine::store::events(ctx.connection, run_id, 0)?;
        if !events.iter().any(|e| {
            e.kind == RunEventKind::ToolCall
                && e.payload["call_id"] == call.call_id
                && e.payload["name"] == call.name
                && e.payload["arguments"] == call.arguments
        }) {
            return Err(invalid("Appel MCP sans liaison d’approbation."));
        }
        // Engine has just consumed the grant or matched a standing permission.
        // Verify that admission again without consuming the single-use grant.
        super::forms::verify_tool_admitted(ctx, call)?;
        execution::execute(ctx, call)
    })();
    Some(match result {
        Ok(output) => ToolOutcome::Done { output },
        Err(e) => ToolOutcome::Failed { reason: e.message },
    })
}
