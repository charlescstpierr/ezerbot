//! Independent local connector management. REA protocol.md distinguishes config,
//! bot selection and grants; NM04 establishes JSON lines, NM07 EOF cleanup.
//! Command launch/removal are host-generated tool calls through forms.rs too.
use super::{write_transaction, Context};
use crate::{mcp::Client, parse, ApiError, ApiResponse, App};
use grokbot_engine::contracts::{RunEventKind, ToolCall, ToolOutcome, ToolSpec};
use grokbot_forms::redact::redact_value;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

// No global MCP tools or custom transcript entries: Services supplies bot-scoped
// specs, and the engine/forms modules own the tool and approval cards.
empty_hooks!(context, entry, link_attachments, tools, tick);
pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_mcp_servers (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, command TEXT NOT NULL,
        arguments_json TEXT NOT NULL, environment_names_json TEXT NOT NULL,
        tools_json TEXT NOT NULL DEFAULT '[]', error TEXT);
        CREATE TABLE IF NOT EXISTS wrapper_mcp_bots (
        server_id TEXT NOT NULL REFERENCES wrapper_mcp_servers(id) ON DELETE CASCADE,
        bot_id TEXT NOT NULL, PRIMARY KEY(server_id,bot_id));
        CREATE TABLE IF NOT EXISTS wrapper_mcp_labels (tool TEXT PRIMARY KEY,display_json TEXT NOT NULL);",
    )?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Add {
    name: String,
    command: String,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
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
    arguments: Vec<String>,
    names: Vec<String>,
    tools: Vec<Value>,
    error: Option<String>,
}
fn servers(conn: &Connection) -> Result<Vec<Server>, ApiError> {
    let mut stmt = conn.prepare("SELECT id,name,command,arguments_json,environment_names_json,tools_json,error FROM wrapper_mcp_servers ORDER BY name,id")?;
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
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(id, name, command, args, names, tools, error)| {
            Ok(Server {
                id,
                name,
                command,
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
fn environment(ctx: &Context<'_>, server: &Server) -> Result<HashMap<String, String>, ApiError> {
    let raw = super::forms::vault(ctx)?
        .get(&secret_name(&server.id))?
        .ok_or_else(|| invalid("Les variables privées du connecteur sont absentes du coffre."))?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|_| invalid("Les variables privées du connecteur sont illisibles."))?;
    serde_json::from_value(value["variables"].clone())
        .map_err(|_| invalid("Les variables privées du connecteur sont illisibles."))
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
        let raw = vault
            .get(&secret.name)?
            .ok_or_else(|| invalid("Une variable privée du coffre MCP a disparu."))?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| invalid("Une enveloppe privée du coffre MCP est illisible."))?;
        let environment: HashMap<String, String> =
            serde_json::from_value(value["variables"].clone())
                .map_err(|_| invalid("Une enveloppe privée du coffre MCP est invalide."))?;
        values.extend(
            environment
                .into_iter()
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
    for value in env.values().filter(|v| !v.is_empty()) {
        patterns.insert(value.clone());
        let encoded = json!(value).to_string();
        patterns.insert(encoded[1..encoded.len() - 1].to_owned());
    }
    let mut values: Vec<_> = patterns.iter().map(String::as_str).collect();
    values.sort_by_key(|s| std::cmp::Reverse(s.len()));
    fn walk(value: &Value, values: &[&str]) -> Result<Value, ApiError> {
        Ok(match value {
            Value::String(s) => json!(text(s, values)),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| walk(v, values))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(items) => {
                let mut output = serde_json::Map::new();
                for (key, value) in items {
                    if output
                        .insert(text(key, values), walk(value, values)?)
                        .is_some()
                    {
                        return Err(invalid(
                            "Le masquage des variables privées rend le résultat MCP ambigu.",
                        ));
                    }
                }
                Value::Object(output)
            }
            _ => {
                let raw = value.to_string();
                if text(&raw, values) != raw {
                    json!("[masqué]")
                } else {
                    value.clone()
                }
            }
        })
    }
    walk(value, &values)
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
    for server in servers(ctx.connection).unwrap_or_default() {
        for action in ["start", "remove"] {
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
                    json!({"server":server.name,"tool":if action=="start" {"Démarrer / redémarrer"} else {"Supprimer le connecteur"}}),
                );
            }
        }
        if let Some(tool) = server.tools.iter().find(|t| namespace(&server, t) == name) {
            return Some(json!({"server":server.name,"tool":tool["name"]}));
        }
    }
    None
}
/// Catalogue refresh follows the engine's lock order: SQLite writer, then
/// process registry. Never try to acquire a writer while holding the registry;
/// a simultaneous approved call may already own SQLite and need that registry.
fn refresh_catalogues(ctx: &Context<'_>) -> Result<(), ApiError> {
    let pending: Vec<_> = {
        let mut live = ctx
            .provider
            .mcp()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        live.iter_mut()
            .filter_map(|(id, client)| match client.catalog_changed() {
                Ok(true) => Some((id.clone(), client.identity(), None)),
                Ok(false) => None,
                Err(error) => Some((id.clone(), client.identity(), Some(error))),
            })
            .collect()
    };
    for (id, identity, error) in pending {
        let tx = write_transaction(ctx.connection)?;
        let local = Context {
            connection: &tx,
            ..*ctx
        };
        let mut live = ctx
            .provider
            .mcp()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // A queued notification/error belongs to one process, not merely its
        // persisted server ID. A concurrent restart must not lose its NEW child.
        if !live
            .get(&id)
            .is_some_and(|client| client.identity() == identity)
        {
            continue;
        }
        let active_private: HashMap<String, String> = live
            .iter()
            .flat_map(|(server, client)| {
                client
                    .private_environment()
                    .iter()
                    .map(move |(key, value)| (format!("active:{server}:{key}"), value.clone()))
            })
            .collect();
        // The approved connection includes metadata discovery, never tool execution.
        let refreshed = match error {
            Some(error) => Err(ApiError::new(503, error)),
            None => live
                .get_mut(&id)
                .unwrap()
                .list_tools()
                .map_err(|e| ApiError::new(503, e))
                .and_then(|tools| safe_catalogue(&local, tools, &active_private)),
        };
        match refreshed {
            Ok(tools) => {
                if let Err(error) = tx.execute(
                    "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
                    params![id, json!(tools).to_string()],
                ) {
                    live.get_mut(&id).unwrap().mark_catalog_dirty();
                    return Err(error.into());
                }
            }
            Err(error) => {
                live.remove(&id);
                tx.execute(
                    "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                    params![id, error.message],
                )?;
            }
        }
        drop(live);
        if let Err(error) = tx.commit() {
            if let Some(client) = ctx
                .provider
                .mcp()
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&id)
            {
                if client.identity() == identity {
                    client.mark_catalog_dirty();
                }
            }
            return Err(error.into());
        }
    }
    Ok(())
}

pub fn state(ctx: &Context<'_>) -> Value {
    let refresh_error = refresh_catalogues(ctx).err();
    let load = || -> Result<Value, ApiError> {
        let mut live = ctx
            .provider
            .mcp()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut result = Vec::new();
        for server in servers(ctx.connection)? {
            let status = match live.get_mut(&server.id) {
                Some(client) => {
                    if client.alive() {
                        "connected"
                    } else {
                        "crashed"
                    }
                }
                None if server.error.is_some() => "error",
                None => "disconnected",
            };
            let mut stmt = ctx.connection.prepare(
                "SELECT bot_id FROM wrapper_mcp_bots WHERE server_id=?1 ORDER BY bot_id",
            )?;
            let bots = stmt
                .query_map([&server.id], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let tools: Vec<_> = server.tools.iter().map(|t|json!({"name":t["name"],"description":t["description"],"namespace":namespace(&server,t)})).collect();
            result.push(json!({"id":server.id,"name":server.name,"command":server.command,"arguments":server.arguments,"environment_names":server.names,"enabled_bots":bots,"status":status,"error":if status=="crashed" {Some("Le serveur MCP s’est arrêté. Redémarrez le connecteur.".to_owned())} else {server.error},"tools":tools}));
        }
        Ok(
            json!({"servers":result,"error":refresh_error.as_ref().map(|_|"L’actualisation des outils est différée. Le dernier état connu reste affiché.")}),
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
            "/api/mcp/add" | "/api/mcp/select" | "/api/mcp/restart" | "/api/mcp/remove"
        )
    {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}
fn lifecycle_arguments(server: &Server, action: &str) -> Value {
    json!({"serveur":server.name,"commande":server.command,"arguments":server.arguments,"variables_privées":server.names,"action":if action=="start" {"Démarrer / redémarrer un processus local"} else {"Arrêter le processus et supprimer ce connecteur et ses variables privées"}})
}

fn route(app: &App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    let ctx = app.feature_context();
    if path == "/api/mcp/add" {
        let add: Add = parse(body)?;
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
                && (name.contains(value)
                    || command.contains(value)
                    || add.arguments.iter().any(|a| a.contains(value)))
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
    let args = lifecycle_arguments(&server, action);
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
        let mut live = ctx
            .provider
            .mcp()
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for server in servers(ctx.connection)? {
            if call.name == format!("mcp_start_{}", server.id) {
                if call.arguments != lifecycle_arguments(&server, "start") {
                    return Err(invalid(
                        "La commande MCP ne correspond pas à la configuration à approuver.",
                    ));
                }
                live.remove(&server.id);
                let env = environment(ctx, &server)?;
                let mut redaction = env.clone();
                for (id, client) in live.iter() {
                    redaction.extend(
                        client
                            .private_environment()
                            .iter()
                            .map(|(key, value)| (format!("active:{id}:{key}"), value.clone())),
                    );
                }
                let started = Client::spawn(&server.command, &server.arguments, &env)
                    .and_then(|mut c| c.initialize().map(|tools| (c, tools)));
                match started {
                    Ok((client, tools)) => {
                        let tools = match safe_catalogue(ctx, tools, &redaction) {
                            Ok(tools) => json!(tools),
                            Err(error) => {
                                ctx.connection.execute(
                                    "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                                    params![server.id, error.message],
                                )?;
                                return Err(error);
                            }
                        };
                        ctx.connection.execute(
                            "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
                            params![server.id, tools.to_string()],
                        )?;
                        live.insert(server.id, client);
                        return Ok(json!({"message":"Connecteur MCP connecté.","outils":tools}));
                    }
                    Err(error) => {
                        ctx.connection.execute(
                            "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                            params![server.id, error],
                        )?;
                        return Err(ApiError::new(503, error));
                    }
                }
            }
            if call.name == format!("mcp_remove_{}", server.id) {
                if call.arguments != lifecycle_arguments(&server, "remove") {
                    return Err(invalid(
                        "La suppression MCP ne correspond pas à la configuration à approuver.",
                    ));
                }
                live.remove(&server.id);
                ctx.connection.execute(
                    "DELETE FROM wrapper_mcp_bots WHERE server_id=?1",
                    [&server.id],
                )?;
                ctx.connection
                    .execute("DELETE FROM wrapper_mcp_servers WHERE id=?1", [&server.id])?;
                // Remove stale Always permissions; an ID is never reused.
                ctx.connection.execute(
                    "DELETE FROM forms_approval_rules WHERE tool GLOB ?1 OR tool IN (?2,?3)",
                    params![
                        format!("mcp_{}_*", server.id),
                        format!("mcp_start_{}", server.id),
                        format!("mcp_remove_{}", server.id)
                    ],
                )?;
                super::forms::vault(ctx)?.remove(&secret_name(&server.id))?;
                return Ok(json!({"message":"Connecteur MCP supprimé et déconnecté."}));
            }
            if let Some(tool) = server
                .tools
                .iter()
                .find(|t| namespace(&server, t) == call.name)
            {
                if !ctx
                    .bot_id
                    .is_some_and(|bot| enabled(ctx.connection, &server.id, bot))
                {
                    return Err(invalid("Ce connecteur MCP est désactivé pour ce bot."));
                }
                let mut private = private_values(ctx)?;
                private.extend(environment(ctx, &server)?);
                for (id, client) in live.iter() {
                    private.extend(
                        client
                            .private_environment()
                            .iter()
                            .map(|(key, value)| (format!("active:{id}:{key}"), value.clone())),
                    );
                }
                let client = live.get_mut(&server.id).ok_or_else(||invalid("Le serveur MCP est déconnecté. Démarrez le connecteur dans les paramètres."))?;
                let output = client.request(
                    "tools/call",
                    json!({"name":tool["name"],"arguments":call.arguments}),
                );
                match output {
                    Ok(output) => {
                        let valid = output["content"].as_array().is_some_and(|parts| {
                            parts.iter().all(|part| {
                                part.is_object()
                                    && part["type"].is_string()
                                    && (part["type"] != "text" || part["text"].is_string())
                            })
                        });
                        if !valid {
                            let error="Le résultat de l’outil MCP est invalide. Le connecteur a été déconnecté.";
                            live.remove(&server.id);
                            ctx.connection.execute(
                                "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                                params![server.id, error],
                            )?;
                            return Err(ApiError::new(503, error));
                        }
                        return sanitize(&output, &private);
                    }
                    Err(error) => {
                        live.remove(&server.id);
                        ctx.connection.execute(
                            "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                            params![server.id, error],
                        )?;
                        return Err(ApiError::new(503, error));
                    }
                }
            }
        }
        Err(invalid(
            "Cet outil MCP est inconnu ou son connecteur a été supprimé.",
        ))
    })();
    Some(match result {
        Ok(output) => ToolOutcome::Done { output },
        Err(e) => ToolOutcome::Failed { reason: e.message },
    })
}
