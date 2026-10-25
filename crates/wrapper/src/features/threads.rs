//! « Répondre en fil » : un sous-fil lié à un message. Le sous-fil est une
//! conversation ordinaire. Le lien garde le message parent tel qu'il était au
//! moment de l'ouverture, et le bot le reçoit comme contexte de chaque réponse.
//! Le fil principal n'est jamais modifié.
use super::Context;
use crate::*;
use grokbot_engine::contracts::{ContextBlock, ContextRequest, ContextSource};

empty_hooks!(
    tick,
    tools,
    call_tool,
    entry,
    on_run_event,
    link_attachments
);

/// Plafond du texte conservé et envoyé au bot : un parent très long est coupé.
const PARENT_LIMIT: usize = 8_000;
const EXCERPT_LIMIT: usize = 160;
const TITLE_EXCERPT_LIMIT: usize = 48;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenThread {
    conversation_id: String,
    message_id: String,
}

struct Link {
    parent_conversation_id: String,
    parent_message_id: String,
    parent_role: String,
    parent_body: String,
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_threads (
            account_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            parent_conversation_id TEXT NOT NULL,
            parent_message_id TEXT NOT NULL,
            parent_role TEXT NOT NULL,
            parent_body TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (account_id, conversation_id),
            UNIQUE (account_id, parent_conversation_id, parent_message_id)
        );",
    )?;
    Ok(())
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if (method, path) != ("POST", "/api/messages/thread") {
        return None;
    }
    Some(open(app, body).map(ApiResponse::Json))
}

pub fn state(ctx: &Context<'_>) -> Value {
    let links = links(ctx.connection).unwrap_or_default();
    let threads: serde_json::Map<String, Value> = links
        .into_iter()
        .map(|(conversation, link)| (conversation, link_json(&link)))
        .collect();
    Value::Object(threads)
}

pub fn context(ctx: &Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    let Ok(Some(link)) = link(ctx.connection, &request.conversation_id) else {
        return Vec::new();
    };
    let role = if link.parent_role == "user" {
        "écrit par Charles"
    } else {
        "écrit par le bot"
    };
    let text = format!(
        "Cette conversation est un fil ouvert sur un message précis d'une autre conversation. \
         Message parent ({role}), à prendre en compte pour répondre :\n{}",
        link.parent_body
    );
    let Ok(vault) = super::forms::vault(ctx) else {
        eprintln!("Fil indisponible : coffre illisible, le message parent n'est pas transmis.");
        return Vec::new();
    };
    let Ok(text) = grokbot_forms::redact::redact(&vault, &text) else {
        return Vec::new();
    };
    vec![ContextBlock {
        source: ContextSource::DelegatedContext,
        priority: super::priority::THREAD,
        budget: PARENT_LIMIT * 2,
        text,
    }]
}

fn open(app: &mut App, body: &[u8]) -> Result<Value, ApiError> {
    let input: OpenThread = parse(body)?;
    validate_conversation_id(&input.conversation_id)?;
    validate_id(&input.message_id)?;
    app.require_conversation(&input.conversation_id)?;
    let parent = app
        .conversation(&input.conversation_id)?
        .ok_or_else(|| ApiError::new(404, "Conversation introuvable."))?;
    if parent.kind != "chat" {
        return Err(ApiError::new(
            409,
            "Répondre en fil n’est pas encore disponible dans ce type de conversation.",
        ));
    }
    let message = parent
        .messages
        .iter()
        .find(|item| item.id == input.message_id)
        .ok_or_else(|| ApiError::new(404, "Message introuvable."))?;
    let threadable = message.kind == "message"
        && !message.body.trim().is_empty()
        && match message.role {
            "user" => matches!(message.state, "accepted" | "echoed"),
            _ => message.state == "echoed",
        };
    if !threadable {
        return Err(ApiError::new(409, "Ce message ne peut pas ouvrir de fil."));
    }
    app.require_bot(&parent.bot_id)?;
    let tx = super::write_transaction(&app.connection)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT t.conversation_id FROM wrapper_threads t JOIN wrapper_conversations c ON c.account_id=t.account_id AND c.id=t.conversation_id
             WHERE t.account_id=?1 AND t.parent_conversation_id=?2 AND t.parent_message_id=?3 AND c.deleted=0",
            params![ACCOUNT, input.conversation_id, input.message_id],
            |row| row.get(0),
        )
        .optional()?;
    let id = match existing {
        Some(id) => id,
        None => {
            // Un sous-fil supprimé laisse sa place : un nouveau fil le remplace.
            tx.execute(
                "DELETE FROM wrapper_threads WHERE account_id=?1 AND parent_conversation_id=?2 AND parent_message_id=?3",
                params![ACCOUNT, input.conversation_id, input.message_id],
            )?;
            let id: String = tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            let timestamp = now();
            let title = format!("Fil : {}", excerpt(&message.body, TITLE_EXCERPT_LIMIT));
            tx.execute("INSERT INTO wrapper_conversations (account_id, id, title, title_customized, created_at, updated_at, bot_id, group_description) VALUES (?1, ?2, ?3, 1, ?4, ?4, ?5, '')", params![ACCOUNT, id, title, timestamp, parent.bot_id])?;
            super::multibot::initialize(&tx, &id, std::slice::from_ref(&parent.bot_id))?;
            tx.execute(
                "INSERT INTO wrapper_threads (account_id, conversation_id, parent_conversation_id, parent_message_id, parent_role, parent_body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    ACCOUNT,
                    id,
                    input.conversation_id,
                    input.message_id,
                    message.role,
                    message.body.chars().take(PARENT_LIMIT).collect::<String>(),
                    timestamp
                ],
            )?;
            id
        }
    };
    tx.commit()?;
    let mut response = app.conversation_response(&id)?;
    let link = link(&app.connection, &id)?
        .ok_or_else(|| ApiError::new(500, "Le fil n’a pas pu être enregistré."))?;
    response["thread"] = link_json(&link);
    super::disclosure::state(&app.connection, &response)
}

fn link_json(link: &Link) -> Value {
    json!({
        "parent_conversation_id": link.parent_conversation_id,
        "parent_message_id": link.parent_message_id,
        "parent_role": link.parent_role,
        "parent_excerpt": excerpt(&link.parent_body, EXCERPT_LIMIT),
    })
}

fn link(conn: &Connection, conversation: &str) -> Result<Option<Link>, ApiError> {
    Ok(conn
        .query_row(
            "SELECT parent_conversation_id, parent_message_id, parent_role, parent_body FROM wrapper_threads WHERE account_id=?1 AND conversation_id=?2",
            params![ACCOUNT, conversation],
            |row| {
                Ok(Link {
                    parent_conversation_id: row.get(0)?,
                    parent_message_id: row.get(1)?,
                    parent_role: row.get(2)?,
                    parent_body: row.get(3)?,
                })
            },
        )
        .optional()?)
}

fn links(conn: &Connection) -> Result<Vec<(String, Link)>, ApiError> {
    let mut statement = conn.prepare(
        "SELECT t.conversation_id, t.parent_conversation_id, t.parent_message_id, t.parent_role, t.parent_body
         FROM wrapper_threads t JOIN wrapper_conversations c ON c.account_id=t.account_id AND c.id=t.conversation_id
         WHERE t.account_id=?1 AND c.deleted=0",
    )?;
    let rows = statement.query_map([ACCOUNT], |row| {
        Ok((
            row.get::<_, String>(0)?,
            Link {
                parent_conversation_id: row.get(1)?,
                parent_message_id: row.get(2)?,
                parent_role: row.get(3)?,
                parent_body: row.get(4)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Première ligne utile, espaces ramenés à un, coupée proprement.
fn excerpt(body: &str, limit: usize) -> String {
    let flat = body
        .split_whitespace()
        .map(|word| word.chars().filter(|c| !c.is_control()).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    if flat.chars().count() <= limit {
        return flat;
    }
    let cut: String = flat.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::excerpt;

    #[test]
    fn extrait_court_sur_une_ligne() {
        assert_eq!(excerpt("  Bonjour \n  le   monde ", 40), "Bonjour le monde");
        assert_eq!(excerpt("abcdefghij", 5), "abcd…");
    }
}
