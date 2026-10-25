//! Local housekeeping, not a recovered Grok server retention policy (REA CM07,
//! CM11–13). Roots are conservative: every durable text field, including skill
//! history, delegated selections and run snapshots. Only attachment
//! metadata/cache, our own housekeeping journal and the rows of a deleted
//! conversation, which nothing restores or shows, are not roots.
//!
//! Commit metadata removal + a deletion intent BEFORE unlinking an orphan blob.
//! Restart replays intents under the same SQLite writer lock as staging/linking;
//! a re-staged digest cancels the unlink. A crash can leave excess bytes, never a
//! live message pointing at a file removed by housekeeping. No recursive delete.
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App, ACCOUNT};
use grokbot_attachments::{documents, staging};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const DAY: i64 = 86_400_000;
empty_hooks!(
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_storage_drafts (
        draft_id TEXT NOT NULL, attachment_id TEXT NOT NULL, name TEXT,
        PRIMARY KEY(draft_id, attachment_id));
        CREATE TABLE IF NOT EXISTS wrapper_storage_uploads (
        token TEXT PRIMARY KEY, draft_id TEXT NOT NULL, attachment_id TEXT,
        cancelled INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_storage_staged (
        id TEXT PRIMARY KEY, staged_at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_storage_previews (
        token TEXT PRIMARY KEY, payload TEXT NOT NULL, created_at INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS wrapper_storage_deletions (
        seq INTEGER PRIMARY KEY, id TEXT NOT NULL, kind TEXT NOT NULL,
        bytes INTEGER NOT NULL, reason TEXT NOT NULL, state TEXT NOT NULL,
        created_at INTEGER NOT NULL);",
    )?;
    let has_name:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_storage_drafts') WHERE name='name')",[],|r|r.get(0))?;
    if !has_name {
        conn.execute(
            "ALTER TABLE wrapper_storage_drafts ADD COLUMN name TEXT",
            [],
        )?;
    }
    // Old documents have no staging timestamp: start a grace period, never
    // pretend their age is known. Restaging refreshes this timestamp.
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_storage_staged SELECT id, ?1 FROM attachments_documents",
        [crate::now() as i64],
    )?;
    Ok(())
}

fn io(error: std::io::Error) -> ApiError {
    ApiError::new(
        500,
        format!("Le stockage local n’a pas pu être nettoyé : {error}"),
    )
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest_id(id: &str) -> Option<&str> {
    id.strip_prefix("attachment_")
        .or_else(|| id.strip_prefix("document_"))
        .or_else(|| id.strip_prefix("generated_"))
        .filter(|s| valid_digest(s))
}
fn directory(base: &Path) -> Result<PathBuf, ApiError> {
    let parent = base
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent).map_err(io)?;
    let mut name = base
        .file_name()
        .ok_or_else(|| ApiError::new(400, "Dossier de données invalide."))?
        .to_os_string();
    name.push(".attachments");
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => return Err(ApiError::new(
            409,
            "Nettoyage refusé : le dossier des pièces jointes n’est pas un dossier local régulier.",
        )),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io(e)),
    }
    Ok(path)
}

// Scan hex runs, not just structured attachment lists: a skill's Markdown URL,
// an engine snapshot, or a future delegated-child field must also retain bytes.
fn collect_digests(text: &[u8], roots: &mut HashSet<String>) {
    for run in text.split(|b| !b.is_ascii_digit() && !(b'a'..=b'f').contains(b)) {
        if run.len() == 64 {
            roots.insert(String::from_utf8_lossy(run).into_owned());
        }
    }
}
fn roots(conn: &Connection) -> Result<HashSet<String>, ApiError> {
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // Naming the account lets SQLite use the conversation key for every row.
    let deleted = format!("EXISTS(SELECT 1 FROM wrapper_conversations c WHERE c.account_id='{ACCOUNT}' AND c.id=t.conversation_id AND c.deleted=1)");
    let deleted_run = format!("EXISTS(SELECT 1 FROM engine_runs r JOIN wrapper_conversations c ON c.account_id='{ACCOUNT}' AND c.id=r.conversation_id WHERE c.deleted=1 AND r.run_id=t.run_id)");
    let mut roots = HashSet::new();
    for table in tables {
        if matches!(
            table.as_str(),
            "attachments_staged"
                | "attachments_documents"
                | "attachments_generated"
                | "attachments_caption_cache"
                | "attachments_caption_uses"
                | "wrapper_attachment_retry"
                | "wrapper_attachment_jobs"
        ) || table.starts_with("wrapper_storage_") && table != "wrapper_storage_drafts"
        {
            continue;
        }
        // Quote identifiers from SQLite, never interpolate user input as SQL.
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        // A deleted conversation's own row, transcript and run journal keep no
        // file alive. Tables that only name a conversation (a page, a topic
        // fact, a routine) stay roots.
        let sql = match table.as_str() {
            "wrapper_conversations" => format!("SELECT * FROM {quoted} t WHERE t.deleted=0"),
            "wrapper_messages" | "submissions" | "wrapper_remote" | "engine_runs"
            | "engine_summaries" => format!("SELECT * FROM {quoted} t WHERE NOT {deleted}"),
            "engine_run_events" | "wrapper_engine_runs" | "engine_external_tools" => {
                format!("SELECT * FROM {quoted} t WHERE NOT {deleted_run}")
            }
            // The parent card still shows a deleted thread's result, never its captures.
            "delegation_graph" => format!("SELECT t.selected_context, CASE WHEN {deleted} THEN json_remove(t.status,'$.captures') ELSE t.status END FROM {quoted} t"),
            _ => format!("SELECT * FROM {quoted}"),
        };
        let mut statement = conn.prepare(&sql)?;
        let columns = statement.column_count();
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            for index in 0..columns {
                match row.get_ref(index)? {
                    rusqlite::types::ValueRef::Text(bytes)
                    | rusqlite::types::ValueRef::Blob(bytes) => collect_digests(bytes, &mut roots),
                    _ => {}
                }
            }
        }
    }
    // A document may itself contain Markdown links to other local attachments.
    // Exclude only its own metadata ID, not references in its extracted text.
    let mut statement = conn.prepare("SELECT record FROM attachments_documents")?;
    let records = statement.query_map([], |r| r.get::<_, String>(0))?;
    for record in records {
        let mut record: Value = serde_json::from_str(&record?).map_err(|_| {
            ApiError::new(
                409,
                "Nettoyage refusé : une extraction de document est illisible.",
            )
        })?;
        let object = record.as_object_mut().ok_or_else(|| {
            ApiError::new(
                409,
                "Nettoyage refusé : une extraction de document est invalide.",
            )
        })?;
        object.remove("id");
        collect_digests(record.to_string().as_bytes(), &mut roots);
    }
    let mut statement = conn.prepare("SELECT bytes FROM attachments_generated")?;
    for bytes in statement.query_map([], |r| r.get::<_, Vec<u8>>(0))? {
        collect_digests(&bytes?, &mut roots);
    }
    Ok(roots)
}

fn generated_extension(kind: &str) -> Option<&str> {
    kind.strip_prefix("generated:")
        .filter(|ext| ["pdf", "docx", "pptx", "md", "csv", "xlsx", "svg"].contains(ext))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Candidate {
    id: String,
    kind: String,
    bytes: u64,
}
#[derive(Default, Serialize)]
struct Usage {
    images: u64,
    documents: u64,
    screen: u64,
    screen_count: usize,
    memory: u64,
    image_count: usize,
    document_count: usize,
    fact_count: usize,
    forget_marker_count: usize,
}

fn inventory(
    conn: &Connection,
    base: &Path,
    before: Option<i64>,
) -> Result<(Usage, Vec<Candidate>), ApiError> {
    let mut usage = Usage::default();
    let mut candidates = Vec::new();
    (usage.screen, usage.screen_count) = conn.query_row(
        "SELECT COALESCE(SUM(length(CAST(image AS BLOB))),0),COUNT(*) FROM wrapper_screen_shots",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let directory = directory(base)?;
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(e)),
    };
    if let Some(entries) = entries {
        for entry in entries {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            // Never follow symlinks, descend into a directory or touch unknown files.
            if !entry.file_type().map_err(io)?.is_file() {
                continue;
            }
            let temporary = name
                .strip_prefix('.')
                .and_then(|s| s.strip_suffix(".tmp"))
                .is_some_and(valid_digest);
            if !valid_digest(&name) && !temporary {
                continue;
            }
            let meta = entry.metadata().map_err(io)?;
            usage.images += meta.len();
            usage.image_count += 1;
            let id = if temporary {
                name.clone()
            } else {
                format!("attachment_{name}")
            };
            let staged: Option<i64> = if temporary {
                None
            } else {
                conn.query_row(
                    "SELECT staged_at FROM attachments_staged WHERE sha256=?1",
                    [&name],
                    |r| r.get(0),
                )
                .optional()?
            };
            let modified = meta
                .modified()
                .map_err(io)?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(i64::MAX as u128) as i64;
            if before.is_none_or(|cutoff| staged.unwrap_or(modified) < cutoff) {
                candidates.push(Candidate {
                    id,
                    kind: if temporary { "temporary" } else { "image" }.into(),
                    bytes: meta.len(),
                });
            }
        }
    }
    let mut statement = conn.prepare("SELECT d.id, length(CAST(d.record AS BLOB)), s.staged_at FROM attachments_documents d LEFT JOIN wrapper_storage_staged s ON s.id=d.id")?;
    let docs = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, Option<i64>>(2)?,
        ))
    })?;
    for doc in docs {
        let (id, bytes, staged) = doc?;
        usage.documents += bytes;
        usage.document_count += 1;
        if digest_id(&id).is_some()
            && before.is_none_or(|cutoff| staged.is_some_and(|at| at < cutoff))
        {
            candidates.push(Candidate {
                id,
                kind: "document".into(),
                bytes,
            });
        }
    }
    let mut statement = conn.prepare("SELECT id, length(bytes)+length(CAST(record AS BLOB))+length(CAST(preview AS BLOB)), created_at FROM attachments_generated")?;
    for row in statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (id, mut bytes, created) = row?;
        let file = super::generated::get(conn, &id)?;
        let cache = super::generated::cache_path(base, &file)?;
        match fs::symlink_metadata(cache) {
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => bytes += m.len(),
            Ok(_) => {
                return Err(ApiError::new(
                    409,
                    "Le cache d’un document généré n’est pas un fichier local régulier.",
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        usage.documents += bytes;
        usage.document_count += 1;
        if before.is_none_or(|cutoff| created < cutoff) {
            candidates.push(Candidate {
                id,
                kind: format!("generated:{}", file.format),
                bytes,
            });
        }
    }
    (usage.memory, usage.fact_count, usage.forget_marker_count) = conn.query_row("SELECT COALESCE(SUM(length(CAST(text AS BLOB)) + length(CAST(provenance AS BLOB))),0), COALESCE(SUM(text!=''),0), COALESCE(SUM(status='forgotten'),0) FROM memory_facts", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((usage, candidates))
}

fn orphan_candidates(
    conn: &Connection,
    mut candidates: Vec<Candidate>,
) -> Result<Vec<Candidate>, ApiError> {
    if candidates.is_empty() {
        return Ok(candidates);
    }
    // Cheap relational roots avoid scanning run/skill history at startup when
    // every old object is still sent or belongs to a live draft.
    let mut statement=conn.prepare("SELECT attachment_id FROM wrapper_storage_drafts UNION SELECT attachment_id FROM attachments_message_links UNION SELECT document_id FROM attachments_document_links UNION SELECT attachment_id FROM wrapper_attachment_order")?;
    let known = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<HashSet<_>>>()?;
    candidates.retain(|item| !known.contains(&item.id));
    if !candidates.is_empty() {
        let roots = roots(conn)?;
        candidates.retain(|item| {
            let sha = if item.kind == "temporary" {
                item.id
                    .strip_prefix('.')
                    .and_then(|s| s.strip_suffix(".tmp"))
            } else {
                digest_id(&item.id)
            };
            !sha.is_some_and(|sha| roots.contains(sha))
        });
    }
    Ok(candidates)
}

fn candidate_exists(conn: &Connection, base: &Path, item: &Candidate) -> Result<bool, ApiError> {
    if generated_extension(&item.kind).is_some() {
        return Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM attachments_generated WHERE id=?1)",
            [&item.id],
            |r| r.get(0),
        )?);
    }
    if item.kind == "document" {
        return Ok(conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM attachments_documents WHERE id=?1)",
            [&item.id],
            |r| r.get(0),
        )?);
    }
    let digest = match item.kind.as_str() {
        "image" => item.id.strip_prefix("attachment_"),
        "temporary" => item
            .id
            .strip_prefix('.')
            .and_then(|s| s.strip_suffix(".tmp")),
        _ => None,
    }
    .filter(|sha| valid_digest(sha))
    .ok_or_else(|| ApiError::new(409, "Aperçu de nettoyage invalide."))?;
    let path = directory(base)?.join(if item.kind == "temporary" {
        item.id.as_str()
    } else {
        digest
    });
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io(error)),
    }
}

pub(super) fn require_draft(conn: &Connection, draft: &str) -> Result<(), ApiError> {
    crate::validate_conversation_id(draft)?;
    let exists:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id=?1 AND id=?2 AND deleted=0)",params![ACCOUNT,draft],|r|r.get(0))?;
    if !exists {
        return Err(ApiError::new(
            404,
            "Le brouillon appartient à une conversation absente ou supprimée.",
        ));
    }
    Ok(())
}

pub(super) fn discard_conversation_draft(conn: &Connection, draft: &str) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE wrapper_storage_uploads SET cancelled=1,created_at=?2 WHERE draft_id=?1",
        params![draft, crate::now() as i64],
    )?;
    conn.execute(
        "DELETE FROM wrapper_storage_drafts WHERE draft_id=?1",
        [draft],
    )?;
    conn.execute(
        "DELETE FROM wrapper_voice_memos WHERE conversation_id=?1 AND message_id IS NULL",
        [draft],
    )?;
    Ok(())
}

/// A deleted conversation is never restored. Its images are erased now unless
/// another conversation, a draft or a durable text still refers to them.
pub(super) fn discard_conversation_images(
    conn: &Connection,
    conversation: &str,
) -> Result<(), ApiError> {
    let candidates = conn
        .prepare("SELECT DISTINCT a.id, a.byte_length FROM attachments_message_links l JOIN attachments_staged a ON a.id=l.attachment_id WHERE l.conversation_id=?1")?
        .query_map([conversation], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                kind: "image".into(),
                bytes: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    conn.execute(
        "DELETE FROM attachments_message_links WHERE conversation_id=?1",
        [conversation],
    )?;
    conn.execute(
        "DELETE FROM wrapper_attachment_order WHERE conversation_id=?1",
        [conversation],
    )?;
    let orphans = orphan_candidates(conn, candidates)?;
    queue(conn, &orphans, "conversation_deleted")?;
    Ok(())
}

fn prune_invalid_drafts(conn: &Connection) -> Result<(), ApiError> {
    conn.execute("UPDATE wrapper_storage_uploads SET cancelled=1,created_at=?2 WHERE cancelled=0 AND NOT EXISTS(SELECT 1 FROM wrapper_conversations c WHERE c.account_id=?1 AND c.id=wrapper_storage_uploads.draft_id AND c.deleted=0)",params![ACCOUNT,crate::now() as i64])?;
    conn.execute("DELETE FROM wrapper_storage_drafts WHERE NOT EXISTS(SELECT 1 FROM wrapper_conversations c WHERE c.account_id=?1 AND c.id=wrapper_storage_drafts.draft_id AND c.deleted=0)",[ACCOUNT])?;
    Ok(())
}

pub(super) fn retain_draft(conn: &Connection, draft: &str, id: &str) -> Result<(), ApiError> {
    require_draft(conn, draft)?;
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_storage_drafts(draft_id,attachment_id) VALUES (?1,?2)",
        params![draft, id],
    )?;
    Ok(())
}
fn retain_named_draft(
    conn: &Connection,
    draft: &str,
    id: &str,
    name: Option<&str>,
) -> Result<(), ApiError> {
    retain_draft(conn, draft, id)?;
    if let Some(name) = name {
        // Preserve the first selection's label when duplicate bytes are imported
        // under a different name; names belong to drafts, not global blob IDs.
        conn.execute("UPDATE wrapper_storage_drafts SET name=COALESCE(name,?3) WHERE draft_id=?1 AND attachment_id=?2",params![draft,id,name])?;
    }
    Ok(())
}
/// A cancellation can arrive before or after the upload publishes its bytes.
/// Its stable token prevents late completion from resurrecting a removed draft.
pub(super) fn retain_upload(
    conn: &Connection,
    draft: &str,
    id: &str,
    token: Option<&str>,
    name: Option<&str>,
) -> Result<(), ApiError> {
    require_draft(conn, draft)?;
    let Some(token) = token else {
        return retain_named_draft(conn, draft, id, name);
    };
    crate::validate_id(token)?;
    conn.execute("INSERT OR IGNORE INTO wrapper_storage_uploads(token,draft_id,created_at) VALUES (?1,?2,?3)",params![token,draft,crate::now() as i64])?;
    let (owner, previous, cancelled): (String, Option<String>, bool) = conn.query_row(
        "SELECT draft_id,attachment_id,cancelled FROM wrapper_storage_uploads WHERE token=?1",
        [token],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if owner != draft || previous.is_some_and(|p| p != id) {
        return Err(ApiError::new(
            409,
            "Cet import appartient à un autre brouillon ou fichier.",
        ));
    }
    conn.execute(
        "UPDATE wrapper_storage_uploads SET attachment_id=?2 WHERE token=?1",
        params![token, id],
    )?;
    if !cancelled {
        retain_named_draft(conn, draft, id, name)?;
    }
    Ok(())
}
fn cancel_upload(conn: &Connection, draft: &str, token: &str) -> Result<(), ApiError> {
    crate::validate_id(token)?;
    conn.execute("INSERT INTO wrapper_storage_uploads(token,draft_id,cancelled,created_at) VALUES (?1,?2,1,?3) ON CONFLICT(token) DO UPDATE SET cancelled=1,created_at=excluded.created_at WHERE draft_id=excluded.draft_id",params![token,draft,crate::now() as i64])?;
    conn.execute("DELETE FROM wrapper_storage_drafts WHERE draft_id=?1 AND attachment_id=(SELECT attachment_id FROM wrapper_storage_uploads WHERE token=?2 AND draft_id=?1) AND NOT EXISTS(SELECT 1 FROM wrapper_storage_uploads u WHERE u.draft_id=?1 AND u.attachment_id=wrapper_storage_drafts.attachment_id AND u.cancelled=0)",params![draft,token])?;
    Ok(())
}
pub(super) fn staged_document(conn: &Connection, id: &str) -> Result<(), ApiError> {
    conn.execute("INSERT INTO wrapper_storage_staged VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET staged_at=excluded.staged_at", params![id,crate::now() as i64])?;
    Ok(())
}

fn drafts(conn: &Connection) -> Result<Value, ApiError> {
    let mut statement = conn.prepare("SELECT draft_id, attachment_id, name FROM wrapper_storage_drafts ORDER BY draft_id, attachment_id")?;
    let rows = statement.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut output = serde_json::Map::new();
    for row in rows {
        let (draft, id, name) = row?;
        let item = if id.starts_with("document_") {
            documents::get(conn, &id)?.map(|d| super::attachments::document_dto(&d))
        } else {
            staging::get(conn, &id)?.map(|a| super::attachments::dto(&a))
        };
        if let Some(mut item) = item {
            if id.starts_with("attachment_") {
                if let Some(name) = name {
                    item["name"] = json!(name);
                }
            }
            output
                .entry(draft)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .expect("array")
                .push(item);
        }
    }
    Ok(Value::Object(output))
}
pub fn state(ctx: &Context<'_>) -> Value {
    // Usage and orphan facts are fetched on demand, not in every chat poll.
    match drafts(ctx.connection) {
        Ok(drafts) => json!({"drafts":drafts}),
        Err(_) => json!({}),
    }
}
fn orphan_facts(conn: &Connection) -> Result<Value, ApiError> {
    let mut statement = conn.prepare("SELECT f.fact_id, f.bot_id, COALESCE(b.name,'Bot supprimé'), f.text, f.status FROM memory_facts f LEFT JOIN wrapper_bots b ON b.account_id=?1 AND b.id=f.bot_id WHERE f.scope='private' AND f.text!='' AND (b.id IS NULL OR b.deleted=1) ORDER BY f.learned_at DESC")?;
    let rows = statement.query_map([ACCOUNT], |r| Ok(json!({"id":r.get::<_,String>(0)?, "bot_id":r.get::<_,String>(1)?, "bot_name":r.get::<_,String>(2)?, "text":r.get::<_,String>(3)?, "status":r.get::<_,String>(4)?})))?;
    Ok(json!(rows.collect::<rusqlite::Result<Vec<_>>>()?))
}
fn log(conn: &Connection, item: &Candidate, reason: &str, state: &str) -> Result<(), ApiError> {
    conn.execute("INSERT INTO wrapper_storage_deletions(id,kind,bytes,reason,state,created_at) VALUES (?1,?2,?3,?4,?5,?6)", params![item.id,item.kind,item.bytes,reason,state,crate::now() as i64])?;
    Ok(())
}
fn queue(conn: &Connection, items: &[Candidate], reason: &str) -> Result<Vec<i64>, ApiError> {
    let mut sequences = Vec::new();
    for item in items {
        if item.kind == "image" {
            conn.execute(
                "DELETE FROM attachments_caption_cache WHERE sha256=?1",
                [digest_id(&item.id).unwrap()],
            )?;
            conn.execute(
                "DELETE FROM attachments_caption_uses WHERE sha256=?1",
                [digest_id(&item.id).unwrap()],
            )?;
            conn.execute(
                "DELETE FROM wrapper_attachment_retry WHERE attachment_id=?1",
                [&item.id],
            )?;
            conn.execute("DELETE FROM attachments_staged WHERE id=?1", [&item.id])?;
        } else if generated_extension(&item.kind).is_some() {
            conn.execute("DELETE FROM attachments_generated WHERE id=?1", [&item.id])?;
        } else if item.kind == "document" {
            conn.execute("DELETE FROM attachments_documents WHERE id=?1", [&item.id])?;
            conn.execute("DELETE FROM wrapper_storage_staged WHERE id=?1", [&item.id])?;
        }
        log(
            conn,
            item,
            reason,
            if item.kind == "document" {
                "deleted"
            } else {
                "pending"
            },
        )?;
        sequences.push(conn.last_insert_rowid());
    }
    Ok(sequences)
}
pub(super) fn replay(conn: &Connection, base: &Path) -> Result<usize, ApiError> {
    let tx = write_transaction(conn)?;
    let rows = tx
        .prepare("SELECT seq,id,kind FROM wrapper_storage_deletions WHERE state='pending'")?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        tx.commit()?;
        return Ok(0);
    }
    let directory = directory(base)?;
    let roots = roots(&tx)?;
    let mut deleted = 0;
    for (seq, id, kind) in rows {
        let sha = if kind == "temporary" {
            id.strip_prefix('.').and_then(|s| s.strip_suffix(".tmp"))
        } else {
            digest_id(&id)
        }
        .filter(|s| valid_digest(s))
        .ok_or_else(|| ApiError::new(409, "Journal de nettoyage invalide."))?;
        let restaged: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM attachments_staged WHERE sha256=?1) OR EXISTS(SELECT 1 FROM attachments_generated WHERE id='generated_' || ?1)",
            [sha],
            |r| r.get(0),
        )?;
        let state = if roots.contains(sha) || restaged {
            "retained"
        } else {
            let filename = if let Some(ext) = generated_extension(&kind) {
                format!("{sha}.{ext}")
            } else if kind == "temporary" {
                id.clone()
            } else {
                sha.to_owned()
            };
            let path = directory.join(filename);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    fs::remove_file(&path).map_err(io)?;
                    fs::File::open(&directory).and_then(|f|f.sync_all()).map_err(io)?;
                    deleted += 1;
                },
                Ok(_) => return Err(ApiError::new(409,"Nettoyage refusé : une pièce jointe a été remplacée par un lien ou un dossier.")),
                Err(e) if e.kind()==std::io::ErrorKind::NotFound => {},
                Err(e) => return Err(io(e)),
            }
            "deleted"
        };
        tx.execute(
            "UPDATE wrapper_storage_deletions SET state=?2 WHERE seq=?1",
            params![seq, state],
        )?;
    }
    tx.commit()?;
    Ok(deleted)
}

pub fn startup(conn: &Connection, base: &Path, now: i64) -> Result<(), ApiError> {
    let tx = write_transaction(conn)?;
    prune_invalid_drafts(&tx)?;
    tx.commit()?;
    replay(conn, base)?;
    let tx = write_transaction(conn)?;
    let (_, candidates) = inventory(&tx, base, Some(now.saturating_sub(DAY)))?;
    let candidates = orphan_candidates(&tx, candidates)?;
    tx.execute(
        "DELETE FROM wrapper_storage_uploads WHERE cancelled=1 AND created_at<?1",
        [now.saturating_sub(DAY)],
    )?;
    queue(&tx, &candidates, "startup_24h")?;
    tx.commit()?;
    replay(conn, base)?;
    Ok(())
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !matches!(
        (method, path),
        ("GET", "/api/storage")
            | (
                "POST",
                "/api/storage/preview"
                    | "/api/storage/clean"
                    | "/api/storage/facts/delete"
                    | "/api/storage/drafts"
            )
    ) {
        return None;
    }
    Some((|| {
        let conn = &app.connection;
        let output = match path {
            "/api/storage" => {
                let tx = conn.unchecked_transaction()?;
                let (usage, _) = inventory(&tx, &app.db_path, None)?;
                let facts = orphan_facts(&tx)?;
                tx.commit()?;
                json!({"usage":usage,"orphan_facts":facts})
            }
            "/api/storage/preview" => {
                let tx = write_transaction(conn)?;
                let (_, candidates) = inventory(&tx, &app.db_path, None)?;
                let candidates = orphan_candidates(&tx, candidates)?;
                let mut random = [0u8; 16];
                getrandom::getrandom(&mut random)
                    .map_err(|_| ApiError::new(500, "Aperçu indisponible."))?;
                let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
                let bytes: u64 = candidates.iter().map(|c| c.bytes).sum();
                tx.execute(
                    "DELETE FROM wrapper_storage_previews WHERE created_at<?1",
                    [crate::now() as i64 - 600_000],
                )?;
                tx.execute(
                    "INSERT INTO wrapper_storage_previews VALUES (?1,?2,?3)",
                    params![
                        token,
                        serde_json::to_string(&candidates).expect("candidates"),
                        crate::now() as i64
                    ],
                )?;
                tx.commit()?;
                json!({"token":token,"count":candidates.len(),"bytes":bytes})
            }
            "/api/storage/clean" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    token: String,
                    confirm: bool,
                }
                let input: Input = parse(body)?;
                if !input.confirm {
                    return Err(ApiError::new(403, "Le nettoyage doit être confirmé."));
                }
                let tx = write_transaction(conn)?;
                let raw:Option<String> = tx.query_row("SELECT payload FROM wrapper_storage_previews WHERE token=?1 AND created_at>=?2", params![input.token,crate::now() as i64-600_000], |r|r.get(0)).optional()?;
                let approved: Vec<Candidate> =
                    serde_json::from_str(&raw.ok_or_else(|| {
                        ApiError::new(409, "L’aperçu a expiré. Relancez Nettoyer.")
                    })?)
                    .map_err(|_| ApiError::new(500, "Aperçu illisible."))?;
                let (_, current) = inventory(&tx, &app.db_path, None)?;
                let current = orphan_candidates(&tx, current)?;
                let selected: Vec<_> = current
                    .into_iter()
                    .filter(|c| {
                        approved
                            .iter()
                            .any(|a| a.id == c.id && a.kind == c.kind && a.bytes == c.bytes)
                    })
                    .collect();
                let mut already_deleted = 0usize;
                for item in &approved {
                    if !selected
                        .iter()
                        .any(|c| c.id == item.id && c.kind == item.kind)
                        && !candidate_exists(&tx, &app.db_path, item)?
                    {
                        already_deleted += 1;
                    }
                }
                let sequences = queue(&tx, &selected, "user_confirmed")?;
                tx.execute(
                    "DELETE FROM wrapper_storage_previews WHERE token=?1",
                    [&input.token],
                )?;
                tx.commit()?;
                replay(conn, &app.db_path)?;
                let mut deleted = 0;
                let mut bytes = 0u64;
                for (seq, item) in sequences.iter().zip(&selected) {
                    let state: String = conn.query_row(
                        "SELECT state FROM wrapper_storage_deletions WHERE seq=?1",
                        [seq],
                        |r| r.get(0),
                    )?;
                    if state == "deleted" {
                        deleted += 1;
                        bytes += item.bytes;
                    }
                }
                json!({"deleted":deleted,"bytes":bytes,"retained":approved.len()-deleted-already_deleted,"already_deleted":already_deleted})
            }
            "/api/storage/drafts" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    draft_id: String,
                    attachments: Vec<String>,
                    #[serde(default)]
                    removed: Vec<String>,
                    #[serde(default)]
                    cancelled_uploads: Vec<String>,
                }
                let input: Input = parse(body)?;
                crate::validate_conversation_id(&input.draft_id)?;
                if input.attachments.len() > 10 {
                    return Err(ApiError::new(400, "Dix fichiers au maximum par brouillon."));
                }
                let tx = write_transaction(conn)?;
                for id in &input.attachments {
                    if digest_id(id).is_none()
                        || (staging::get(&tx, id)?.is_none() && documents::get(&tx, id)?.is_none())
                    {
                        return Err(ApiError::new(
                            404,
                            "Pièce jointe introuvable. Joignez-la à nouveau.",
                        ));
                    }
                }
                for token in &input.cancelled_uploads {
                    cancel_upload(&tx, &input.draft_id, token)?;
                }
                for id in &input.removed {
                    tx.execute("UPDATE wrapper_storage_uploads SET cancelled=1,created_at=?3 WHERE draft_id=?1 AND attachment_id=?2",params![input.draft_id,id,crate::now() as i64])?;
                    tx.execute(
                        "DELETE FROM wrapper_storage_drafts WHERE draft_id=?1 AND attachment_id=?2",
                        params![input.draft_id, id],
                    )?;
                }
                for id in &input.attachments {
                    retain_draft(&tx, &input.draft_id, id)?;
                }
                tx.commit()?;
                json!({"ok":true})
            }
            "/api/storage/facts/delete" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Input {
                    fact_id: String,
                    confirm: bool,
                }
                let input: Input = parse(body)?;
                if !input.confirm {
                    return Err(ApiError::new(403, "La suppression doit être confirmée."));
                }
                let tx = write_transaction(conn)?;
                let deleted = super::memory::purge_deleted_private(&tx, &input.fact_id)?;
                log(
                    &tx,
                    &Candidate {
                        id: input.fact_id,
                        kind: "fact".into(),
                        bytes: 0,
                    },
                    "user_confirmed",
                    "deleted",
                )?;
                tx.commit()?;
                json!({"deleted":deleted})
            }
            _ => unreachable!(),
        };
        Ok(ApiResponse::Json(output))
    })())
}
