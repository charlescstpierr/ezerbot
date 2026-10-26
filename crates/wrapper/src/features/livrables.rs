//! A creation is keyed by its thread card, not by the content-addressed file.
//! Unknown historical documents remain storage roots. Deleted origins are hidden.
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
    // A historical result names the file, and its call names the card.
    // The card key makes migration retries idempotent.
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
    // A masked historical ID leaves the origin of unmatched files ambiguous.
    // Keep those files as storage roots, but never relabel them as public unknowns.
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_livrables
         SELECT CASE WHEN EXISTS (
             SELECT 1 FROM engine_run_events e
             JOIN engine_run_events c ON c.run_id=e.run_id AND c.kind='tool_call'
                  AND json_extract(c.payload,'$.call_id')=json_extract(e.payload,'$.call_id')
             WHERE e.kind='tool_result' AND json_extract(e.payload,'$.outcome.status')='done'
               AND json_extract(c.payload,'$.name')='create_document'
               AND NOT EXISTS (SELECT 1 FROM attachments_generated known WHERE known.id=json_extract(e.payload,'$.outcome.output.id'))
         ) THEN 'nonattribue:' ELSE 'inconnu:' END||g.id,
                NULL, 'document', g.id, COALESCE(json_extract(g.record,'$.name'), g.id),
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
    if ctx.run_id.is_none() {
        return Ok(());
    }
    let card = super::images::tool_message_id(ctx, call_id).map_err(|_| {
        ApiError::new(
            409,
            "La création ne peut pas être reliée à sa conversation.",
        )
    })?;
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
    super::disclosure::stamp(conn, "creation", &card)?;
    Ok(())
}

/// Creation history is never scanned here. Deleted origins hide their creations.
pub const LIST: &str = "
SELECT x.kind, x.ref, x.title, x.bot_id, b.name, x.conversation_id, c.title, x.created_at, x.open_url, x.entry_id, t.epoch, bt.epoch, ct.epoch
FROM wrapper_livrables x
LEFT JOIN wrapper_bots b ON b.account_id=?1 AND b.id=x.bot_id
LEFT JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=x.conversation_id
LEFT JOIN wrapper_disclosure_trust t ON t.kind='creation' AND t.key=x.key
LEFT JOIN wrapper_disclosure_trust bt ON bt.kind='bot' AND bt.key=?1||':'||x.bot_id
LEFT JOIN wrapper_disclosure_trust ct ON ct.kind='conversation' AND ct.key=?1||':'||x.conversation_id
WHERE x.key NOT GLOB 'nonattribue:*' AND x.kind IN ('document', 'image')
  AND (x.bot_id IS NULL OR b.deleted=0) AND (x.conversation_id IS NULL OR c.deleted=0)
  AND (?2 IS NULL OR x.kind=?2) AND (?3 IS NULL OR x.bot_id=?3)
ORDER BY x.created_at DESC, x.entry_id DESC, x.ref";

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
    if kind.is_some_and(|kind| !matches!(kind, "document" | "image")) {
        return Err(ApiError::new(400, "Types acceptés : document et image."));
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
    Some(super::disclosure::read(&app.connection, |patterns| {
        let (kind, bot) = filters(query)?;
        let mut statement = app.connection.prepare_cached(LIST)?;
        let rows = statement.query_map(params![ACCOUNT, kind, bot], |r| {
            let mut item = json!({
                "type": r.get::<_, String>(0)?, "id": r.get::<_, String>(1)?,
                "title": r.get::<_, String>(2)?,
                "bot_id": r.get::<_, Option<String>>(3)?, "bot_name": r.get::<_, Option<String>>(4)?,
                "conversation_id": r.get::<_, Option<String>>(5)?,
                "conversation_title": r.get::<_, Option<String>>(6)?,
                "date": r.get::<_, i64>(7)?, "open_url": r.get::<_, String>(8)?,
                "entry_id": r.get::<_, Option<String>>(9)?,
            });
            if !patterns.covers(r.get::<_, Option<String>>(10)?.as_deref()) {
                let kind = item["type"].clone();
                item = json!({"type":kind,"id":null,"title":if kind=="image" {"Image"} else {"Document"},"bot_id":null,"bot_name":null,"conversation_id":null,"conversation_title":null,"date":0,"open_url":null,"entry_id":null});
            } else {
                if !patterns.covers(r.get::<_, Option<String>>(11)?.as_deref()) {
                    item["bot_id"] = Value::Null;
                    item["bot_name"] = Value::Null;
                }
                if !patterns.covers(r.get::<_, Option<String>>(12)?.as_deref()) {
                    item["conversation_id"] = Value::Null;
                    item["conversation_title"] = Value::Null;
                    item["entry_id"] = Value::Null;
                }
            }
            Ok(item)
        })?;
        let livrables = json!({"livrables": rows.collect::<rusqlite::Result<Vec<Value>>>()?});
        for item in livrables["livrables"]
            .as_array()
            .ok_or_else(super::disclosure::refused)?
        {
            for key in [
                "type",
                "id",
                "bot_id",
                "conversation_id",
                "date",
                "open_url",
                "entry_id",
            ] {
                patterns.identity(&item[key])?;
            }
        }
        let mut output = patterns.value(&livrables)?;
        for item in output
            .get_mut("livrables")
            .and_then(Value::as_array_mut)
            .ok_or_else(super::disclosure::refused)?
        {
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
        Ok(ApiResponse::Json(patterns.value(&output)?))
    }))
}
