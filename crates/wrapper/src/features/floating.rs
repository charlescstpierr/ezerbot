//! Desktop-mount is a bot composer, not a filesystem mount (REA desktop §7,
//! execution §8). The panel has an explicit target; a main-window navigation
//! cannot retarget an in-flight send. All generation/tools use the ordinary gate.
use super::Context;
use crate::*;

empty_hooks!(
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

const SHORTCUTS: &[&str] = &[
    "disabled",
    "cmd+option+space",
    "ctrl+option+space",
    "cmd+shift+space",
    "cmd+option+j",
];

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_floating (
        account_id TEXT PRIMARY KEY, shortcut TEXT NOT NULL,
        conversation_id TEXT, bot_id TEXT NOT NULL);",
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_floating VALUES (?1,'cmd+option+j',NULL,'grokbot')",
        [ACCOUNT],
    )?;
    Ok(())
}

fn snapshot(conn: &Connection) -> Result<Value, ApiError> {
    let (shortcut, conversation, bot): (String, Option<String>, String) = conn.query_row(
        "SELECT shortcut,conversation_id,bot_id FROM wrapper_floating WHERE account_id=?1",
        [ACCOUNT],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    // Never silently redirect a deleted target to some other conversation.
    let live = conversation.filter(|id| eligible(conn, id).is_ok());
    let bot = conn
        .query_row(
            "SELECT id FROM wrapper_bots WHERE account_id=?1 AND id=?2 AND deleted=0",
            params![ACCOUNT, bot],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "grokbot".into());
    Ok(json!({"shortcut":shortcut,"conversation_id":live,"bot_id":bot}))
}

fn eligible(conn: &Connection, id: &str) -> Result<(), ApiError> {
    validate_conversation_id(id)?;
    let available: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations c JOIN wrapper_bots b ON b.account_id=c.account_id AND b.id=c.bot_id WHERE c.account_id=?1 AND c.id=?2 AND c.deleted=0 AND c.kind IN ('chat','group') AND b.deleted=0)", params![ACCOUNT,id], |r|r.get(0))?;
    if !available {
        return Err(ApiError::new(404,"Cette conversation n’est pas disponible dans le panneau. Ouvrez un fil direct ou un groupe."));
    }
    Ok(())
}

pub fn state(ctx: &Context<'_>) -> Value {
    // Route reads report storage errors; the namespace remains small in /state.
    snapshot(ctx.connection).unwrap_or_else(|_| json!({}))
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !path.starts_with("/api/floating") {
        return None;
    }
    Some((|| {
        let value = match (method, path) {
            ("GET", "/api/floating") => snapshot(&app.connection)?,
            ("POST", "/api/floating/settings") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Settings {
                    shortcut: String,
                }
                let input: Settings = parse(body)?;
                if !SHORTCUTS.contains(&input.shortcut.as_str()) {
                    return Err(ApiError::new(400, "Ce raccourci n’est pas proposé."));
                }
                app.connection.execute(
                    "UPDATE wrapper_floating SET shortcut=?2 WHERE account_id=?1",
                    params![ACCOUNT, input.shortcut],
                )?;
                snapshot(&app.connection)?
            }
            ("POST", "/api/floating/context") => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Selection {
                    conversation_id: Option<String>,
                    bot_id: String,
                }
                let input: Selection = parse(body)?;
                app.require_bot(&input.bot_id)?;
                if let Some(id) = &input.conversation_id {
                    eligible(&app.connection, id)?;
                }
                app.connection.execute(
                    "UPDATE wrapper_floating SET conversation_id=?2,bot_id=?3 WHERE account_id=?1",
                    params![ACCOUNT, input.conversation_id, input.bot_id],
                )?;
                snapshot(&app.connection)?
            }
            ("POST", "/api/floating/send") => {
                #[derive(Deserialize, Serialize)]
                #[serde(deny_unknown_fields)]
                struct Send {
                    conversation_id: String,
                    message_id: String,
                    body: String,
                }
                let input: Send = parse(body)?;
                eligible(&app.connection, &input.conversation_id)?;
                // Same journal nonce, refusal/error semantics, context, approval
                // cards and « Toujours » rules as the full conversation.
                app.api(
                    "POST",
                    "/api/send",
                    &serde_json::to_vec(&input).expect("send JSON"),
                )?
            }
            _ => return Err(ApiError::new(404, "Cette action du panneau n’existe pas.")),
        };
        Ok(ApiResponse::Json(value))
    })())
}
