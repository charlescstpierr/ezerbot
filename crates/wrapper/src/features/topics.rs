//! Sujets : un dossier nommé qui regroupe plusieurs fils. Une conversation (bot
//! seul ou groupe) appartient à zéro ou un sujet. Le sujet ne copie rien d'une
//! conversation : supprimer un sujet les laisse intactes, sans sujet.
//! `wrapper_conversations.parent_id` désigne le parent d'une sous-tâche, pas un
//! sujet : le lien vit dans sa propre table.
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

const MAX_NAME: usize = 80;
const MAX_DESCRIPTION: usize = 280;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTopic {
    name: String,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateTopic {
    id: String,
    name: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteTopic {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignTopic {
    conversation_id: String,
    /// Absent ou `null` : la conversation quitte son sujet.
    topic_id: Option<String>,
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // Une conversation n'a qu'une ligne : la clé primaire garantit un seul sujet.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_topics (
            account_id TEXT NOT NULL,
            id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (account_id, id)
        );
        CREATE TABLE IF NOT EXISTS wrapper_topic_conversations (
            account_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            topic_id TEXT NOT NULL,
            assigned_at INTEGER NOT NULL,
            PRIMARY KEY (account_id, conversation_id)
        );
        CREATE INDEX IF NOT EXISTS wrapper_topic_conversations_topic
            ON wrapper_topic_conversations (account_id, topic_id);",
    )?;
    Ok(())
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
            "/api/topics" | "/api/topics/update" | "/api/topics/delete" | "/api/topics/assign"
        )
    {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}

/// `items` suit l'ordre de création ; `membership` associe chaque conversation
/// visible à son sujet. Une conversation supprimée n'y figure plus.
pub fn state(ctx: &Context<'_>) -> Value {
    json!({
        "items": topics(ctx.connection).unwrap_or_default(),
        "membership": membership(ctx.connection).unwrap_or_default(),
    })
}

fn route(app: &mut App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    match path {
        "/api/topics" => {
            let input: CreateTopic = parse(body)?;
            let name = valid_name(&input.name)?;
            let description = input
                .description
                .as_deref()
                .map(valid_description)
                .transpose()?
                .unwrap_or_default();
            let id: String =
                app.connection
                    .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            let timestamp = now();
            app.connection.execute(
                "INSERT INTO wrapper_topics (account_id, id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![ACCOUNT, id, name, description, timestamp],
            )?;
            topic_response(&app.connection, &id)
        }
        "/api/topics/update" => {
            let input: UpdateTopic = parse(body)?;
            validate_id(&input.id)?;
            let name = input.name.as_deref().map(valid_name).transpose()?;
            let description = input
                .description
                .as_deref()
                .map(valid_description)
                .transpose()?;
            if name.is_none() && description.is_none() {
                return Err(ApiError::new(400, "Indiquez un nom ou une description."));
            }
            let changed = app.connection.execute(
                "UPDATE wrapper_topics SET name = COALESCE(?3, name), description = COALESCE(?4, description), updated_at = ?5 WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, input.id, name, description, now()],
            )?;
            if changed == 0 {
                return Err(not_found());
            }
            topic_response(&app.connection, &input.id)
        }
        "/api/topics/delete" => {
            let input: DeleteTopic = parse(body)?;
            validate_id(&input.id)?;
            let tx = super::write_transaction(&app.connection)?;
            // Seuls le sujet et ses liens partent : les conversations ne sont pas touchées.
            let removed = tx.execute(
                "DELETE FROM wrapper_topics WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, input.id],
            )?;
            if removed == 0 {
                return Err(not_found());
            }
            tx.execute(
                "DELETE FROM wrapper_topic_conversations WHERE account_id = ?1 AND topic_id = ?2",
                params![ACCOUNT, input.id],
            )?;
            tx.commit()?;
            Ok(json!({"ok": true}))
        }
        "/api/topics/assign" => {
            let input: AssignTopic = parse(body)?;
            validate_conversation_id(&input.conversation_id)?;
            let tx = super::write_transaction(&app.connection)?;
            assign(&tx, &input.conversation_id, input.topic_id.as_deref())?;
            tx.commit()?;
            Ok(json!({"ok": true, "topic_id": input.topic_id}))
        }
        _ => unreachable!("route filtered above"),
    }
}

/// Classe une conversation dans un sujet, ou la retire du sien (`None`). Placer
/// une conversation ailleurs la retire de l'ancien sujet : une seule ligne par
/// conversation. Appelé dans la transaction de l'appelant.
pub(crate) fn assign(
    conn: &Connection,
    conversation: &str,
    topic: Option<&str>,
) -> Result<(), ApiError> {
    let kind: Option<(String, bool)> = conn
        .query_row(
            "SELECT kind, deleted FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
            params![ACCOUNT, conversation],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match kind {
        None => return Err(ApiError::new(404, "Conversation introuvable.")),
        Some((_, true)) => return Err(ApiError::new(409, "Cette conversation a été supprimée.")),
        Some((kind, false)) if !matches!(kind.as_str(), "chat" | "group") => {
            return Err(ApiError::new(
                409,
                "Seuls un fil de bot et un groupe peuvent rejoindre un sujet.",
            ));
        }
        Some(_) => {}
    }
    let Some(topic) = topic else {
        conn.execute(
            "DELETE FROM wrapper_topic_conversations WHERE account_id = ?1 AND conversation_id = ?2",
            params![ACCOUNT, conversation],
        )?;
        return Ok(());
    };
    validate_id(topic)?;
    require_topic(conn, topic)?;
    conn.execute(
        "INSERT INTO wrapper_topic_conversations (account_id, conversation_id, topic_id, assigned_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, conversation_id) DO UPDATE SET topic_id = excluded.topic_id, assigned_at = excluded.assigned_at",
        params![ACCOUNT, conversation, topic, now()],
    )?;
    Ok(())
}

/// Un sujet doit exister avant d'y créer un fil.
pub(crate) fn require_topic(conn: &Connection, topic: &str) -> Result<(), ApiError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_topics WHERE account_id = ?1 AND id = ?2)",
        params![ACCOUNT, topic],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(not_found())
    }
}

fn not_found() -> ApiError {
    ApiError::new(404, "Sujet introuvable.")
}

fn topic_response(conn: &Connection, id: &str) -> Result<Value, ApiError> {
    let topic = topics(conn)?
        .into_iter()
        .find(|topic| topic["id"] == id)
        .ok_or_else(not_found)?;
    Ok(json!({"topic": topic}))
}

fn topics(conn: &Connection) -> Result<Vec<Value>, ApiError> {
    let mut statement = conn.prepare(
        "SELECT id, name, description, created_at, updated_at FROM wrapper_topics WHERE account_id = ?1 ORDER BY created_at, id",
    )?;
    let rows = statement.query_map([ACCOUNT], |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "name": row.get::<_, String>(1)?,
            "description": row.get::<_, String>(2)?,
            "created_at": row.get::<_, u64>(3)?,
            "updated_at": row.get::<_, u64>(4)?,
        }))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn membership(conn: &Connection) -> Result<serde_json::Map<String, Value>, ApiError> {
    let mut statement = conn.prepare(
        "SELECT m.conversation_id, m.topic_id
         FROM wrapper_topic_conversations m
         JOIN wrapper_topics t ON t.account_id = m.account_id AND t.id = m.topic_id
         JOIN wrapper_conversations c ON c.account_id = m.account_id AND c.id = m.conversation_id
         WHERE m.account_id = ?1 AND c.deleted = 0",
    )?;
    let rows = statement.query_map([ACCOUNT], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = serde_json::Map::new();
    for row in rows {
        let (conversation, topic) = row?;
        result.insert(conversation, Value::String(topic));
    }
    Ok(result)
}

fn valid_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(ApiError::new(
            400,
            "Le nom du sujet doit contenir entre 1 et 80 caractères.",
        ));
    }
    Ok(name.into())
}

/// Facultative : une description vide efface la précédente.
fn valid_description(description: &str) -> Result<String, ApiError> {
    let description = description.trim();
    let forbidden = description
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t');
    if description.chars().count() > MAX_DESCRIPTION || forbidden {
        return Err(ApiError::new(
            400,
            "La description du sujet ne peut pas dépasser 280 caractères.",
        ));
    }
    Ok(description.into())
}
