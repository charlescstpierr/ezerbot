//! What the bots produced, read in one place: `GET /api/livrables`. One creation is
//! one livrable. A Page lives in `bot_pages` (one row per Page, last write wins). A
//! document or an image is a row of `wrapper_livrables`, keyed by the thread card of the
//! tool call that made it, because the file itself is addressed by its content: two
//! identical creations share one file and stay two livrables. A new kind (a kept answer)
//! inserts its own row with its own `open_url`; this read does not change.
//! A deleted bot or conversation hides its livrables, as for skills and tasks; no row is
//! erased, so nothing is lost if the bot is restored. The table is a root for the Storage
//! screen: a file that is a livrable, even of unknown origin, is never cleaned as an orphan.
use super::Context;
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

empty_hooks!(
    state,
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

/// `key` is the card (`run_id:seq`) or `inconnu:<ref>` when no run remembers the file.
/// `entry_id`, `bot_id` and `conversation_id` are null in that last case.
pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_livrables (
            key TEXT PRIMARY KEY, entry_id TEXT, kind TEXT NOT NULL, ref TEXT NOT NULL,
            title TEXT NOT NULL, bot_id TEXT, conversation_id TEXT,
            open_url TEXT NOT NULL, created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS wrapper_livrables_ref ON wrapper_livrables(ref);",
    )?;
    let done: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM component_versions WHERE component='wrapper.livrables')",
        [],
        |r| r.get(0),
    )?;
    if done {
        return Ok(());
    }
    // Every earlier creation is still in its run: the tool result names the file, the tool
    // call before it names the card. `OR IGNORE` on the key makes a second pass a no-op.
    for (tool, kind, field, url, title) in [
        (
            "create_document",
            "document",
            "id",
            "'/api/generated/'||f.id||'/preview'",
            "json_extract(f.record,'$.name')",
        ),
        (
            "generate_image",
            "image",
            "attachment_id",
            "'/api/attachments/'||f.id",
            "COALESCE(json_extract(c.payload,'$.arguments.description'),'Image générée')",
        ),
    ] {
        let table = if kind == "document" {
            "attachments_generated"
        } else {
            "attachments_staged"
        };
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO wrapper_livrables
                 SELECT e.run_id||':'||c.seq, e.run_id||':'||c.seq, '{kind}', f.id, {title},
                        r.bot_id, r.conversation_id, {url}, COALESCE(json_extract(r.record,'$.started_at'), 0)
                 FROM engine_run_events e
                 JOIN engine_runs r ON r.run_id=e.run_id
                 JOIN engine_run_events c ON c.run_id=e.run_id AND c.kind='tool_call'
                      AND json_extract(c.payload,'$.call_id')=json_extract(e.payload,'$.call_id')
                 JOIN {table} f ON f.id=json_extract(e.payload,'$.outcome.output.{field}')
                 WHERE e.kind='tool_result' AND json_extract(e.payload,'$.outcome.status')='done'
                   AND json_extract(c.payload,'$.name')=?1"
            ),
            [tool],
        )?;
    }
    // A Page keeps the card of its last good write, if that write is in its conversation.
    conn.execute(
        "UPDATE bot_pages SET entry_id=h.entry FROM (
            SELECT page, entry, conversation FROM (
                SELECT json_extract(e.payload,'$.outcome.output.id') AS page, e.run_id||':'||c.seq AS entry,
                       r.conversation_id AS conversation,
                       ROW_NUMBER() OVER (PARTITION BY json_extract(e.payload,'$.outcome.output.id')
                           ORDER BY COALESCE(json_extract(r.record,'$.started_at'), 0) DESC, c.seq DESC) AS rank
                FROM engine_run_events e
                JOIN engine_runs r ON r.run_id=e.run_id
                JOIN engine_run_events c ON c.run_id=e.run_id AND c.kind='tool_call'
                     AND json_extract(c.payload,'$.call_id')=json_extract(e.payload,'$.call_id')
                WHERE e.kind='tool_result' AND json_extract(e.payload,'$.outcome.status')='done'
                  AND json_extract(c.payload,'$.name') IN ('write_page','edit_page')
            ) WHERE rank=1
         ) h WHERE h.page=bot_pages.id AND h.conversation=bot_pages.conversation_id
           AND bot_pages.entry_id IS NULL",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_livrables
         SELECT 'inconnu:'||g.id, NULL, 'document', g.id, COALESCE(json_extract(g.record,'$.name'), g.id),
                NULL, NULL, '/api/generated/'||g.id||'/preview', g.created_at
         FROM attachments_generated g
         WHERE NOT EXISTS (SELECT 1 FROM wrapper_livrables l WHERE l.ref=g.id)",
        [],
    )?;
    conn.execute(
        "INSERT INTO component_versions(component,version) VALUES ('wrapper.livrables',1)",
        [],
    )?;
    Ok(())
}

/// Registers a creation made by the tool call that has just succeeded. A call with no run
/// (a hidden probe) has no thread card and makes no livrable.
pub fn record(
    conn: &Connection,
    ctx: &Context<'_>,
    call_id: &str,
    kind: &str,
    reference: &str,
    title: &str,
    open_url: &str,
) -> Result<(), ApiError> {
    let Ok(card) = super::images::tool_message_id(ctx, call_id) else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO wrapper_livrables VALUES (?1,?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT DO NOTHING",
        params![
            card,
            kind,
            reference,
            title,
            ctx.bot_id,
            ctx.conversation_id,
            open_url,
            ctx.now_ms
        ],
    )?;
    Ok(())
}

/// One read, most recent first. A Page is listed only when it compiled. `?1` account, `?2`
/// kind and `?3` bot are optional filters; a livrable whose origin is lost is always shown.
pub const LIST: &str = "
SELECT x.type, x.id, x.title, x.bot_id, b.name, x.conversation_id, c.title, x.date, x.open_url, x.entry_id
FROM (
    SELECT 'page' AS type, id, title, bot_id, conversation_id, updated_at AS date,
           '/api/pages/'||id AS open_url, entry_id
    FROM bot_pages WHERE status='ready'
    UNION ALL
    SELECT kind, ref, title, bot_id, conversation_id, created_at, open_url, entry_id
    FROM wrapper_livrables
) x
LEFT JOIN wrapper_bots b ON b.account_id=?1 AND b.id=x.bot_id
LEFT JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=x.conversation_id
WHERE (x.bot_id IS NULL OR b.deleted=0) AND (x.conversation_id IS NULL OR c.deleted=0)
  AND (?2 IS NULL OR x.type=?2) AND (?3 IS NULL OR x.bot_id=?3)
ORDER BY x.date DESC, x.entry_id DESC, x.id";

/// Only `type` and `bot`, each once: any other parameter is refused with a 400 rather than
/// ignored, so a typo never reads as an unfiltered list.
fn filters(query: &str) -> Result<(Option<&str>, Option<&str>), ApiError> {
    let (mut kind, mut bot) = (None, None);
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        crate::validate_id(value)?;
        let slot = match key {
            "type" => &mut kind,
            "bot" => &mut bot,
            _ => {
                return Err(ApiError::new(
                    400,
                    "Paramètres acceptés : type et bot, chacun une seule fois.",
                ))
            }
        };
        if slot.replace(value).is_some() {
            return Err(ApiError::new(
                400,
                "Paramètres acceptés : type et bot, chacun une seule fois.",
            ));
        }
    }
    Ok((kind, bot))
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    _body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    if method != "GET" || route != "/api/livrables" {
        return None;
    }
    Some((|| {
        let (kind, bot) = filters(query)?;
        let mut statement = app.connection.prepare_cached(LIST)?;
        let rows = statement.query_map(params![ACCOUNT, kind, bot], |r| {
            Ok(json!({
                "type": r.get::<_, String>(0)?, "id": r.get::<_, String>(1)?,
                "title": r.get::<_, String>(2)?,
                "bot_id": r.get::<_, Option<String>>(3)?, "bot_name": r.get::<_, Option<String>>(4)?,
                "conversation_id": r.get::<_, Option<String>>(5)?,
                "conversation_title": r.get::<_, Option<String>>(6)?,
                "date": r.get::<_, i64>(7)?, "open_url": r.get::<_, String>(8)?,
                "entry_id": r.get::<_, Option<String>>(9)?,
            }))
        })?;
        let livrables = json!({"livrables": rows.collect::<rusqlite::Result<Vec<Value>>>()?});
        // Titles come from the bot: a secret registered since is masked, as in the thread.
        let mut livrables =
            super::engine::redaction_patterns(&app.connection)?.value(&livrables)?;
        // An image is titled by its whole description, cut only now: a cut made before the
        // mask would show the first characters of a secret.
        for item in livrables["livrables"].as_array_mut().into_iter().flatten() {
            if item["type"] == "image" {
                let title: String = item["title"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect();
                item["title"] = Value::String(title);
            }
        }
        Ok(ApiResponse::Json(livrables))
    })())
}
