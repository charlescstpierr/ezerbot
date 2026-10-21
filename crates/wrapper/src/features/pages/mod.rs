//! Local Page: the bot writes a `.page.tsx`, Ezerbot compiles it, and the
//! result stays on this Mac. A compile error names the line and character so
//! the bot can rewrite the file. There is no public address.
mod compile;

use super::{priority, write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App};
use grokbot_engine::contracts::{
    ContextBlock, ContextRequest, ContextSource, ToolCall, ToolOutcome, ToolSpec,
};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

empty_hooks!(tick, entry, on_run_event, link_attachments);

const REWRITE: &str = "Réécrivez le fichier .page.tsx ; Charles ne l\u{2019}édite pas.";
const SOURCE_LIMIT: usize = 100_000;

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bot_pages (
            id TEXT PRIMARY KEY,
            bot_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL,
            title TEXT NOT NULL,
            filename TEXT NOT NULL,
            source TEXT NOT NULL,
            status TEXT NOT NULL,
            diagnostic TEXT,
            document TEXT,
            preview TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE (bot_id, filename)
        );",
    )?;
    Ok(())
}

pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "write_page".into(),
        description: "Écrire une Page locale .page.tsx, compilée sur ce Mac et affichée à côté du fil. À utiliser pour une analyse autonome (tableau interactif, graphique simple, outil réutilisable), jamais un tableau Markdown. name se termine par .page.tsx. title : 1 à 80 caractères. source : export default function Nom() { const [etat, setEtat] = useState(\"texte\" ou nombre); const lignes = [[\"A\", \"1\"], [\"B\", \"2\"]]; return ( <article><h1>Titre</h1><p>{etat}</p><table><tbody>{lignes.map((ligne) => (<tr><td>{ligne[0]}</td><td><button onClick={() => setEtat(ligne[0])}>Choisir</button></td></tr>))}</tbody></table></article> ); }. Balises autorisées : article, button, div, em, h1, h2, h3, li, ol, p, span, strong, table, tbody, td, th, thead, tr, ul. Seul attribut : onClick={() => setEtat(ligne[n])} ou un texte/nombre littéral. 100 000 octets maximum. Pas de JavaScript libre, pas de lien, pas de partage. Si la compilation échoue, le message donne la ligne et le caractère : réécrire tout le fichier.".into(),
        schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {"type": "string"},
                "title": {"type": "string"},
                "source": {"type": "string", "maxLength": SOURCE_LIMIT}
            },
            "required": ["name", "title", "source"]
        }),
        requires_approval: false,
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WritePage {
    name: String,
    title: String,
    source: String,
}

pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if call.name != "write_page" {
        return None;
    }
    // The engine already holds the write transaction. Insert on this connection
    // and do not begin another one. The file is outside SQLite and idempotent.
    Some(match persist(ctx, call) {
        Ok(output) => ToolOutcome::Done { output },
        Err(error) => ToolOutcome::Failed {
            reason: error.message,
        },
    })
}

fn persist(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let args: WritePage = serde_json::from_value(call.arguments.clone()).map_err(|_| {
        ApiError::new(
            400,
            "Arguments de la Page invalides : nom, titre et source sont requis.",
        )
    })?;
    let filename = page_filename(&args.name)?;
    let title = page_title(&args.title)?;
    let bot = ctx
        .bot_id
        .filter(|id| page_bot(id).is_ok())
        .ok_or_else(|| ApiError::new(400, "Cette Page n’est pas rattachée à un bot."))?;
    let conversation = ctx
        .conversation_id
        .filter(|id| !id.is_empty() && !id.contains('\0'))
        .ok_or_else(|| ApiError::new(400, "Cette Page n’est pas rattachée à une conversation."))?;
    let id = format!(
        "page_{:x}",
        Sha256::digest([bot.as_bytes(), b"\0", filename.as_bytes()].concat())
    );
    let compiled = compile::compile(&args.source);
    let status = if compiled.is_ok() { "ready" } else { "error" };
    let diagnostic = compiled.as_ref().err().map(compile::Diagnostic::text);
    let document = match compiled.as_ref() {
        Ok(page) => Some(
            serde_json::to_string(&page.document)
                .map_err(|_| ApiError::new(500, "La Page compilée n’a pas pu être enregistrée."))?,
        ),
        Err(_) => None,
    };
    let preview = compiled
        .as_ref()
        .map(|page| page.preview.clone())
        .unwrap_or_default();
    if args.source.len() <= SOURCE_LIMIT {
        write_source(&pages_dir(ctx, bot)?, &filename, &args.source)?;
    }
    ctx.connection.execute(
        "INSERT INTO bot_pages (
            id, bot_id, conversation_id, title, filename, source, status, diagnostic, document, preview, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT(id) DO UPDATE SET
            conversation_id = excluded.conversation_id,
            title = excluded.title,
            source = excluded.source,
            status = excluded.status,
            diagnostic = excluded.diagnostic,
            document = excluded.document,
            preview = excluded.preview,
            updated_at = excluded.updated_at",
        params![
            id,
            bot,
            conversation,
            title,
            filename,
            args.source,
            status,
            diagnostic,
            document,
            preview,
            ctx.now_ms
        ],
    )?;
    match compiled {
        Ok(_) => Ok(json!({
            "id": id,
            "title": title,
            "filename": filename,
            "status": "ready"
        })),
        Err(error) => Err(ApiError::new(400, format!("{}. {REWRITE}", error.text()))),
    }
}

pub fn state(ctx: &Context<'_>) -> Value {
    let mut by_bot: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let Ok(mut statement) = ctx.connection.prepare(
        "SELECT id, bot_id, conversation_id, title, filename, status
         FROM bot_pages ORDER BY updated_at DESC, id",
    ) else {
        return json!({"by_bot": {}});
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    }) else {
        return json!({"by_bot": {}});
    };
    for row in rows.flatten() {
        let (id, bot_id, conversation_id, title, filename, status) = row;
        by_bot.entry(bot_id.clone()).or_default().push(json!({
            "id": id,
            "bot_id": bot_id,
            "conversation_id": conversation_id,
            "title": title,
            "filename": filename,
            "status": status
        }));
    }
    json!({"by_bot": by_bot})
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method == "POST" && path == "/api/pages/rename" {
        return Some(rename(app, body).map(ApiResponse::Json));
    }
    if method == "GET" {
        if let Some(id) = path.strip_prefix("/api/pages/") {
            if valid_page_id(id) {
                return Some(read(app, id).map(ApiResponse::Json));
            }
        }
    }
    None
}

pub fn context(ctx: &Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    if ctx.bot_id.is_some_and(|bot| bot != request.bot_id) {
        return Vec::new();
    }
    let text = "Quand Charles demande une analyse autonome (tableau interactif, graphique ou outil réutilisable), appelle write_page et écris un fichier .page.tsx. N’envoie pas un tableau Markdown à la place : la Page s’affiche à côté du fil, sur ce Mac. Si la compilation échoue, le message indique la ligne et le caractère ; réécris le fichier, Charles ne l’édite pas. Il n’existe aucun lien public et aucun partage.".to_string();
    // The instruction category cap is the minimum budget of its blocks. A short
    // budget would drop the bot's private instructions. Priority stays just
    // ahead of those instructions so they remain the last block.
    vec![ContextBlock {
        source: ContextSource::BotInstructions,
        priority: priority::INSTRUCTIONS - 1,
        budget: 16_384,
        text,
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    id: String,
    title: String,
}

fn rename(app: &mut App, body: &[u8]) -> Result<Value, ApiError> {
    let input: Rename = parse(body)?;
    if !valid_page_id(&input.id) {
        return Err(ApiError::new(400, "Référence de Page invalide."));
    }
    let title = page_title(&input.title)?;
    let tx = write_transaction(&app.connection)?;
    let changed = tx.execute(
        "UPDATE bot_pages SET title = ?2, updated_at = ?3 WHERE id = ?1",
        params![input.id, title, crate::now()],
    )?;
    if changed == 0 {
        return Err(ApiError::new(404, "Cette Page est introuvable."));
    }
    let page = load(&tx, &input.id)?;
    tx.commit()?;
    Ok(json!({"page": page}))
}

fn read(app: &App, id: &str) -> Result<Value, ApiError> {
    Ok(json!({"page": load(&app.connection, id)?}))
}

struct PageRecord {
    id: String,
    bot_id: String,
    conversation_id: String,
    title: String,
    filename: String,
    status: String,
    diagnostic: Option<String>,
    document: Option<String>,
    preview: String,
}

fn load(conn: &Connection, id: &str) -> Result<Value, ApiError> {
    let record = conn
        .query_row(
            "SELECT id, bot_id, conversation_id, title, filename, status, diagnostic, document, preview
             FROM bot_pages WHERE id = ?1",
            [id],
            |row| {
                Ok(PageRecord {
                    id: row.get(0)?,
                    bot_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                    title: row.get(3)?,
                    filename: row.get(4)?,
                    status: row.get(5)?,
                    diagnostic: row.get(6)?,
                    document: row.get(7)?,
                    preview: row.get(8)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                ApiError::new(404, "Cette Page est introuvable.")
            }
            other => other.into(),
        })?;
    let document = match record.document {
        Some(text) => serde_json::from_str(&text)
            .map_err(|_| ApiError::new(500, "Le document de la Page est illisible."))?,
        None => Value::Null,
    };
    Ok(json!({
        "id": record.id,
        "bot_id": record.bot_id,
        "conversation_id": record.conversation_id,
        "title": record.title,
        "filename": record.filename,
        "status": record.status,
        "diagnostic": record.diagnostic,
        "preview": record.preview,
        "document": document
    }))
}

fn valid_page_id(id: &str) -> bool {
    id.strip_prefix("page_").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn page_filename(name: &str) -> Result<String, ApiError> {
    let Some(stem) = name.strip_suffix(".page.tsx") else {
        return Err(ApiError::new(
            400,
            "Le fichier doit se terminer par .page.tsx.",
        ));
    };
    let length = stem.chars().count();
    let plain = stem
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '-' | '_'));
    if !(1..=60).contains(&length)
        || !plain
        || stem.starts_with(' ')
        || stem.ends_with(' ')
        || stem.contains("  ")
    {
        return Err(ApiError::new(
            400,
            "Le nom du fichier doit contenir 1 à 60 caractères : lettres, chiffres, espace, tiret ou underscore.",
        ));
    }
    Ok(name.to_string())
}

fn page_title(title: &str) -> Result<String, ApiError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err(ApiError::new(
            400,
            "Le titre doit contenir entre 1 et 80 caractères.",
        ));
    }
    Ok(title.to_string())
}

fn page_bot(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 80
        || id.contains("..")
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::new(400, "Ce bot ne peut pas recevoir de Page."));
    }
    Ok(())
}

fn pages_dir(ctx: &Context<'_>, bot: &str) -> Result<PathBuf, ApiError> {
    let data = ctx
        .db_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(data.join("pages").join(bot))
}

fn write_source(directory: &Path, filename: &str, source: &str) -> Result<(), ApiError> {
    let failed = || ApiError::new(500, "La Page n’a pas pu être écrite sur ce Mac.");
    fs::create_dir_all(directory).map_err(|_| failed())?;
    let directory = directory.canonicalize().map_err(|_| failed())?;
    let temporary = directory.join(format!(".{filename}.tmp"));
    fs::write(&temporary, source).map_err(|_| failed())?;
    fs::rename(&temporary, directory.join(filename)).map_err(|_| failed())?;
    Ok(())
}
