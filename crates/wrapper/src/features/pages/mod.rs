//! Local Page: the bot writes a `.page.tsx`, Ezerbot compiles it, and the
//! result stays on this Mac. A compile error names the line and character so
//! the bot can rewrite the file. There is no public address.
pub(super) mod anchors;
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
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
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
    anchors::migrate(conn)
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
        write_source(data_dir(ctx), bot, &filename, &args.source)?;
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
    if compiled.is_ok() {
        anchors::settle(ctx, &id, &args.source)?;
    }
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
    let mut blocks = vec![ContextBlock {
        source: ContextSource::BotInstructions,
        priority: priority::INSTRUCTIONS - 1,
        budget: 16_384,
        text,
    }];
    blocks.extend(anchors::context(ctx, request));
    blocks
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
    source: String,
    status: String,
    diagnostic: Option<String>,
    document: Option<String>,
    preview: String,
}

pub(super) fn load(conn: &Connection, id: &str) -> Result<Value, ApiError> {
    let record = conn
        .query_row(
            "SELECT id, bot_id, conversation_id, title, filename, source, status, diagnostic, document, preview
             FROM bot_pages WHERE id = ?1",
            [id],
            |row| {
                Ok(PageRecord {
                    id: row.get(0)?,
                    bot_id: row.get(1)?,
                    conversation_id: row.get(2)?,
                    title: row.get(3)?,
                    filename: row.get(4)?,
                    source: row.get(5)?,
                    status: row.get(6)?,
                    diagnostic: row.get(7)?,
                    document: row.get(8)?,
                    preview: row.get(9)?,
                })
            },
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                ApiError::new(404, "Cette Page est introuvable.")
            }
            other => other.into(),
        })?;
    let (status, diagnostic, document) = present_document(&record)?;
    Ok(json!({
        "id": record.id,
        "bot_id": record.bot_id,
        "conversation_id": record.conversation_id,
        "title": record.title,
        "filename": record.filename,
        "status": status,
        "diagnostic": diagnostic,
        "preview": record.preview,
        "source_version": source_version(&record.source),
        "anchors": anchors::list(conn, &record.id)?,
        "document": document
    }))
}

fn source_version(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn recursion_limit(error: &serde_json::Error) -> bool {
    // serde_json classifies this ceiling as a syntax error, not data.
    error.is_syntax() && error.to_string().contains("recursion limit exceeded")
}

/// Page-side render of a saved document. A tree past the cap, including one
/// already stored, is refused before it is walked or sent to the DOM builder.
fn present_document(record: &PageRecord) -> Result<(String, Option<String>, Value), ApiError> {
    let refused = (
        "error".to_string(),
        Some("imbrication trop profonde".to_string()),
        Value::Null,
    );
    if record.status != "ready" {
        return Ok((
            record.status.clone(),
            record.diagnostic.clone(),
            Value::Null,
        ));
    }
    if let Some(text) = record.document.as_deref() {
        if compile::stored_document_too_deep(text) {
            return Ok(refused);
        }
        match serde_json::from_str::<Value>(text) {
            Ok(value) if !compile::exceeds_depth(&value) => {
                return Ok(("ready".to_string(), record.diagnostic.clone(), value));
            }
            Ok(_) => return Ok(refused),
            Err(error) if recursion_limit(&error) => {}
            Err(_) => {
                return Err(ApiError::new(500, "Le document de la Page est illisible."));
            }
        }
    }
    match compile::compile(&record.source) {
        Ok(page) if !compile::exceeds_depth(&page.document) => {
            Ok(("ready".to_string(), None, page.document))
        }
        Ok(_) => Ok(refused),
        Err(error) => Ok(("error".to_string(), Some(error.text()), Value::Null)),
    }
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

fn data_dir<'a>(ctx: &Context<'a>) -> &'a Path {
    ctx.db_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn outside() -> ApiError {
    ApiError::new(
        400,
        "La Page ne peut pas être écrite en dehors de son dossier.",
    )
}

fn unwritten() -> ApiError {
    ApiError::new(500, "La Page n’a pas pu être écrite sur ce Mac.")
}

fn single_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// `parent` is already canonical. Any symlink at this component is refused,
/// including one whose target stays inside the data folder.
fn anchored_dir(parent: &Path, name: &str) -> Result<PathBuf, ApiError> {
    if !single_component(name) {
        return Err(outside());
    }
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => return Err(outside()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(error) = fs::create_dir(&path) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(unwritten());
                }
            }
        }
        Err(_) => return Err(unwritten()),
    }
    let meta = fs::symlink_metadata(&path).map_err(|_| unwritten())?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(outside());
    }
    let resolved = path.canonicalize().map_err(|_| unwritten())?;
    let same_parent = resolved.parent() == Some(parent);
    let same_name = resolved.file_name() == Some(std::ffi::OsStr::new(name));
    if !same_parent || !same_name {
        return Err(outside());
    }
    Ok(resolved)
}

fn page_directory(data: &Path, bot: &str) -> Result<PathBuf, ApiError> {
    let pages = anchored_dir(data, "pages")?;
    anchored_dir(&pages, bot)
}

fn write_source(data: &Path, bot: &str, filename: &str, source: &str) -> Result<(), ApiError> {
    let data = data.canonicalize().map_err(|_| unwritten())?;
    page_directory(&data, bot)?;
    if !single_component(filename) || filename.contains("..") {
        return Err(outside());
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    // Read pages and the bot directory again immediately before the exclusive create.
    let directory = page_directory(&data, bot)?;
    let temporary = directory.join(format!(".{filename}.{}.{nanos}.tmp", std::process::id()));
    let output = directory.join(filename);
    if temporary.parent() != Some(directory.as_path())
        || output.parent() != Some(directory.as_path())
    {
        return Err(outside());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| unwritten())?;
    let wrote = file
        .write_all(source.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    if wrote.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(unwritten());
    }
    if fs::rename(&temporary, &output).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(unwritten());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ezerbot-pages-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn write_refuses_a_bot_directory_symlink_that_leaves_pages() {
        let root = Temp(scratch("symlink"));
        let pages = root.0.join("data/pages");
        let outside = root.0.join("outside");
        fs::create_dir_all(&pages).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel.txt"), "rester").unwrap();
        let bot = pages.join("grokbot");
        symlink(&outside, &bot).unwrap();

        let error = write_source(
            &root.0.join("data"),
            "grokbot",
            "forfaits.page.tsx",
            "contenu secret",
        )
        .unwrap_err();
        assert!(error.message.contains("en dehors"), "{}", error.message);
        assert!(!outside.join("forfaits.page.tsx").exists());
        assert!(!outside.join(".forfaits.page.tsx.tmp").exists());
        assert_eq!(
            fs::read_to_string(outside.join("sentinel.txt")).unwrap(),
            "rester"
        );
        let entries: Vec<_> = fs::read_dir(&outside)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("sentinel.txt")]);
    }

    #[test]
    fn write_refuses_a_pages_symlink_outside_the_canonical_data_folder() {
        let root = Temp(scratch("pages-link"));
        let data = root.0.join("data");
        let outside = root.0.join("outside");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel.txt"), "rester").unwrap();
        symlink(&outside, data.join("pages")).unwrap();

        let error =
            write_source(&data, "grokbot", "forfaits.page.tsx", "contenu secret").unwrap_err();
        assert!(error.message.contains("en dehors"), "{}", error.message);
        assert!(fs::symlink_metadata(data.join("pages"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!outside.join("grokbot").exists());
        assert!(!outside.join("forfaits.page.tsx").exists());
        assert_eq!(
            fs::read_to_string(outside.join("sentinel.txt")).unwrap(),
            "rester"
        );
        let entries: Vec<_> = fs::read_dir(&outside)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("sentinel.txt")]);
    }

    #[test]
    fn write_refuses_a_symlink_whose_target_stays_inside_pages() {
        let root = Temp(scratch("internal"));
        let data = root.0.join("data");
        let pages = data.join("pages");
        let real = pages.join("reel");
        fs::create_dir_all(&real).unwrap();
        symlink(&real, pages.join("alias")).unwrap();

        let error = write_source(&data, "alias", "lien.page.tsx", "lien").unwrap_err();
        assert!(error.message.contains("en dehors"), "{}", error.message);
        assert!(!real.join("lien.page.tsx").exists());
        write_source(&data, "reel", "direct.page.tsx", "direct").unwrap();
        assert_eq!(
            fs::read_to_string(real.join("direct.page.tsx")).unwrap(),
            "direct"
        );
    }

    #[test]
    fn write_leaves_an_existing_temp_hard_link_untouched() {
        let root = Temp(scratch("hardlink"));
        let data = root.0.join("data");
        let bot = data.join("pages").join("grokbot");
        fs::create_dir_all(&bot).unwrap();
        let secret = root.0.join("secret.txt");
        fs::write(&secret, "SECRET").unwrap();
        let planted = bot.join(".forfaits.page.tsx.tmp");
        fs::hard_link(&secret, &planted).unwrap();

        write_source(&data, "grokbot", "forfaits.page.tsx", "contenu secret").unwrap();
        assert_eq!(fs::read_to_string(&secret).unwrap(), "SECRET");
        assert_eq!(fs::read_to_string(&planted).unwrap(), "SECRET");
        assert_eq!(
            fs::read_to_string(bot.join("forfaits.page.tsx")).unwrap(),
            "contenu secret"
        );
    }

    fn deep_document(depth: usize) -> String {
        let mut view = r#"{"type":"text","text":"fin"}"#.to_string();
        for _ in 0..depth {
            view = format!(r#"{{"type":"element","tag":"div","children":[{view}]}}"#);
        }
        format!(r#"{{"states":[],"view":{view}}}"#)
    }

    fn insert_page(conn: &Connection, id: &str, source: &str, document: &str) {
        conn.execute(
            "INSERT INTO bot_pages (
                id, bot_id, conversation_id, title, filename, source, status, diagnostic, document, preview, created_at, updated_at
             ) VALUES (?1, 'grokbot', 'fil', 'Profonde', 'profonde.page.tsx', ?2, 'ready', NULL, ?3, 'fin', 1, 1)",
            params![id, source, document],
        )
        .unwrap();
    }

    #[test]
    fn capped_page_reloads_when_serde_cannot_parse_the_stored_json() {
        let mut open = String::new();
        let mut close = String::new();
        for _ in 0..64 {
            open.push_str("<div>");
            close.push_str("</div>");
        }
        let source = format!("export default function Page(){{return ({open}ok{close});}}");
        let compiled = compile::compile(&source).unwrap();
        let stored = serde_json::to_string(&compiled.document).unwrap();
        assert!(
            serde_json::from_str::<Value>(&stored).is_err(),
            "64 éléments dépassent la récursion serde_json"
        );
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_page(&conn, "page_cap", &source, &stored);
        let page = load(&conn, "page_cap").unwrap();
        assert_eq!(page["status"], "ready", "{page}");
        assert_eq!(page["document"]["view"]["tag"], "div");
    }

    #[test]
    fn already_saved_deep_document_is_refused_without_crashing() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let source = "export default function Page(){return (<p>court</p>);}\n";
        insert_page(&conn, "page_deep", source, &deep_document(9_000));
        let page = load(&conn, "page_deep")
            .expect("un document profond déjà enregistré reste lisible comme refus");
        assert!(page["document"].is_null(), "{page}");
        let diagnostic = page["diagnostic"].as_str().unwrap_or("");
        assert!(
            diagnostic.contains("imbrication trop profonde"),
            "{diagnostic}"
        );
        assert_eq!(page["status"], "error");
    }
}
