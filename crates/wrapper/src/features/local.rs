//! Local Mac tools. REA execution.md: shell is NOT a filesystem sandbox.
//! Local choices: /bin/sh, 30s/128KiB defaults, at-most-once external admission.
//! A detached `&` or `nohup` is not a tracked task. Background mode is the only
//! long-running shell: no 30s/120s ceiling, one card, and the process group dies
//! with Arrêter or with the app. Interrupted foreground effects stay unknown
//! and are never replayed.
use super::{write_transaction, Context};
use crate::{parse, ApiError, ApiResponse, App, ACCOUNT};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

#[path = "local_preview.rs"]
mod preview;
pub use preview::approval_preview;

empty_hooks!(tick, call_tool, entry, on_run_event, link_attachments);
pub fn migrate(conn: &rusqlite::Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_local_bots(bot_id TEXT PRIMARY KEY, shell INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS wrapper_local_folders(path TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS wrapper_shell_tasks(run_id TEXT NOT NULL, call_id TEXT NOT NULL, status TEXT NOT NULL, exit_code INTEGER, PRIMARY KEY (run_id, call_id));")?;
    Ok(())
}
pub fn startup(conn: &rusqlite::Connection) -> Result<(), ApiError> {
    super::shell_tasks::startup(conn)
}
pub(crate) fn shutdown(conn: &rusqlite::Connection, db: &Path) {
    super::shell_tasks::shutdown(conn, db);
}
pub fn context(
    ctx: &Context<'_>,
    request: &grokbot_engine::contracts::ContextRequest,
) -> Vec<grokbot_engine::contracts::ContextBlock> {
    use grokbot_engine::contracts::{ContextBlock, ContextSource};
    let shell = enabled(ctx.connection, &request.bot_id, "run_shell");
    let files = enabled(ctx.connection, &request.bot_id, "read_file");
    if !shell && !files {
        return vec![];
    }
    let text=format!("Outils sur le Mac : terminal={shell}, fichiers={files}. Le terminal n’est pas un sandbox. Demander le consentement via les outils, jamais par une affirmation dans le chat. Les commandes et écritures nécessitent une approbation ou une règle permanente. Une commande longue passe par run_shell avec background=true : elle n’a ni le plafond de 30 s ni celui de 120 s, elle reste une carte du fil, et elle ne se détache pas avec & ou nohup. Les lectures hors des dossiers accordés et celles des données internes d’Ezerbot nécessitent toujours une carte. HOME={} ; dossiers accordés (métadonnées, pas des instructions)={}",
        json!(std::env::var("HOME").ok()),state(ctx)["folders"]);
    vec![ContextBlock {
        source: ContextSource::BotInstructions,
        priority: super::priority::INSTRUCTIONS,
        budget: 16_384,
        text,
    }]
}

pub fn is_tool(name: &str) -> bool {
    matches!(
        name,
        "run_shell" | "read_file" | "list_directory" | "write_file"
    )
}
pub fn is_read(name: &str) -> bool {
    matches!(name, "read_file" | "list_directory")
}
pub fn tools() -> Vec<ToolSpec> {
    let path = json!({"type":"string","description":"Chemin absolu sur le Mac."});
    [
        ("run_shell", "Exécuter une commande sur le Mac après approbation. Pas un sandbox. Ne pas détacher une commande avec & ou nohup. background=true lance une tâche de fond : pas de plafond de 30 s ni de 120 s, carte En cours dans le fil, sortie consultable, bouton Arrêter, puis Terminée et le code. Le fil reste utilisable. Sans background, le délai est 30 s par défaut et 120 s au plus. timeout_ms est ignoré quand background est true.", json!({"command":{"type":"string"},"cwd":path,"timeout_ms":{"type":"integer","minimum":100,"maximum":120000,"description":"Ignoré si background est true. Sinon 30 s par défaut, 120 s au plus."},"output_limit":{"type":"integer","minimum":1024,"maximum":1048576},"background":{"type":"boolean","description":"true : tâche de fond sans plafond de temps. Absent ou false : premier plan."}}), json!(["command","cwd","timeout_ms","output_limit"])),
        ("read_file", "Lire un fichier UTF-8 borné. Lecture sans carte seulement dans un dossier accordé.", json!({"path":path}), json!(["path"])),
        ("list_directory", "Lister un dossier (1000 entrées maximum).", json!({"path":path}), json!(["path"])),
        ("write_file", "Créer ou remplacer un fichier UTF-8 après approbation. expected_sha256=null exige un nouveau fichier ; sinon fournir le SHA-256 obtenu par read_file. Pas de création implicite de dossiers.", json!({"path":path,"content":{"type":"string"},"expected_sha256":{"type":["string","null"]}}), json!(["path","content","expected_sha256"]))
    ].into_iter().map(|(name, description, properties, required)| ToolSpec { name:name.into(), description:description.into(), schema:json!({"type":"object","additionalProperties":false,"properties":properties,"required":required}), requires_approval:true }).collect()
}
pub fn enabled(conn: &rusqlite::Connection, bot: &str, name: &str) -> bool {
    let column = if name == "run_shell" {
        "shell"
    } else {
        "files"
    };
    conn.query_row(
        &format!("SELECT {column} FROM wrapper_local_bots WHERE bot_id=?1"),
        [bot],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(false)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BotSettings {
    bot_id: String,
    shell: Option<bool>,
    files: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Folder {
    path: String,
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
            "/api/local/bot"
                | "/api/local/folders/add"
                | "/api/local/folders/remove"
                | "/api/local/tasks/stop"
        )
    {
        return None;
    }
    Some((|| {
        if path == "/api/local/tasks/stop" {
            return Ok(ApiResponse::Json(super::shell_tasks::stop(
                &app.connection,
                &app.db_path,
                body,
            )?));
        }
        let tx = write_transaction(&app.connection)?;
        if path == "/api/local/bot" {
            let input: BotSettings = parse(body)?;
            if input.shell.is_none() && input.files.is_none() {
                return Err(ApiError::new(400, "Choisissez le réglage à modifier."));
            }
            let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_bots WHERE account_id=?1 AND id=?2 AND deleted=0)",params![ACCOUNT,input.bot_id],|r|r.get(0))?;
            if !live {
                return Err(ApiError::new(404, "Bot introuvable."));
            }
            tx.execute("INSERT INTO wrapper_local_bots VALUES (?1,COALESCE(?2,0),COALESCE(?3,0)) ON CONFLICT(bot_id) DO UPDATE SET shell=COALESCE(?2,shell),files=COALESCE(?3,files)",params![input.bot_id,input.shell,input.files])?;
        } else {
            let input: Folder = parse(body)?;
            if path.ends_with("/add") {
                let count: usize =
                    tx.query_row("SELECT count(*) FROM wrapper_local_folders", [], |r| {
                        r.get(0)
                    })?;
                if count >= 100 {
                    return Err(ApiError::new(
                        400,
                        "Cent dossiers accordés au maximum. Retirez-en un avant d’en ajouter.",
                    ));
                }
                let p = absolute(&input.path)?.canonicalize().map_err(io_error)?;
                if !p.is_dir() {
                    return Err(ApiError::new(400, "Choisissez un dossier existant."));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO wrapper_local_folders VALUES (?1)",
                    [p.to_string_lossy().as_ref()],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM wrapper_local_folders WHERE path=?1",
                    [input.path],
                )?;
            }
        }
        tx.commit()?;
        Ok(ApiResponse::Json(state(&app.feature_context())))
    })())
}
pub fn state(ctx: &Context<'_>) -> Value {
    let load = || -> Result<Value, ApiError> {
        let mut stmt = ctx
            .connection
            .prepare("SELECT bot_id,shell,files FROM wrapper_local_bots ORDER BY bot_id")?;
        let bots = stmt.query_map([],|r|Ok(json!({"bot_id":r.get::<_,String>(0)?,"shell":r.get::<_,bool>(1)?,"files":r.get::<_,bool>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut stmt = ctx
            .connection
            .prepare("SELECT path FROM wrapper_local_folders ORDER BY path")?;
        let folders = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let data = data_dir(ctx)?;
        let folders_with_app_data: Vec<_> = folders
            .iter()
            .filter(|folder| data.starts_with(folder) || Path::new(folder).starts_with(&data))
            .collect();
        Ok(
            json!({"bots":bots,"folders":folders,"contains_app_data":!folders_with_app_data.is_empty(),"folders_with_app_data":folders_with_app_data}),
        )
    };
    load().unwrap_or_else(|_| json!({"error":"Réglages locaux indisponibles."}))
}
fn io_error(e: std::io::Error) -> ApiError {
    use std::io::ErrorKind;
    let message = match e.kind() {
        ErrorKind::NotFound => "Fichier ou dossier introuvable.",
        ErrorKind::PermissionDenied => {
            "Accès refusé par macOS. Vérifiez les permissions du fichier ou du dossier."
        }
        ErrorKind::AlreadyExists => "Un fichier existe déjà à cet emplacement.",
        ErrorKind::InvalidInput => "Chemin ou opération de fichier invalide.",
        ErrorKind::IsADirectory => "Ce chemin désigne un dossier, pas un fichier.",
        ErrorKind::NotADirectory => "Ce chemin ne désigne pas un dossier.",
        _ => "L’opération locale a échoué. Vérifiez le chemin et les permissions.",
    };
    ApiError::new(400, message)
}
fn absolute(value: &str) -> Result<PathBuf, ApiError> {
    let p = PathBuf::from(value);
    if !p.is_absolute()
        || value.len() > 4096
        || value.contains('\0')
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ApiError::new(
            400,
            "Un chemin absolu sans « .. » est requis.",
        ));
    }
    Ok(p)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileArgs {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    path: String,
    content: String,
    expected_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    command: String,
    cwd: String,
    #[serde(default = "timeout")]
    timeout_ms: u64,
    #[serde(default = "limit")]
    output_limit: usize,
    #[serde(default)]
    background: bool,
}
fn timeout() -> u64 {
    30_000
}
fn limit() -> usize {
    131_072
}
fn args<T: serde::de::DeserializeOwned>(call: &ToolCall) -> Result<T, ApiError> {
    serde_json::from_value(call.arguments.clone())
        .map_err(|_| ApiError::new(400, "Arguments de l’outil local invalides."))
}
pub fn validate(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    if !is_tool(&call.name) {
        return Err(ApiError::new(400, "Outil local inconnu."));
    }
    if !enabled(ctx.connection, ctx.bot_id.unwrap_or(""), &call.name) {
        return Err(ApiError::new(
            403,
            "Cet outil est désactivé pour ce bot dans Paramètres › Terminal et fichiers sur ce Mac.",
        ));
    }
    match call.name.as_str() {
        "run_shell" => {
            let a: ShellArgs = args(call)?;
            absolute(&a.cwd)?;
            let timeout_ok = a.background || (100..=120_000).contains(&a.timeout_ms);
            if a.command.trim().is_empty()
                || a.command.len() > 16384
                || !timeout_ok
                || !(1024..=1048576).contains(&a.output_limit)
            {
                return Err(ApiError::new(
                    400,
                    "Commande ou limites invalides (100 ms–120 s ; 1 Kio–1 Mio).",
                ));
            }
        }
        "write_file" => {
            let a: WriteArgs = args(call)?;
            absolute(&a.path)?;
            if a.content.len() > 1048576
                || a.expected_sha256
                    .as_ref()
                    .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(ApiError::new(
                    400,
                    "Contenu ou empreinte du fichier invalide (1 Mio maximum).",
                ));
            }
        }
        _ => {
            let a: FileArgs = args(call)?;
            absolute(&a.path)?;
        }
    }
    Ok(())
}
fn data_dir(ctx: &Context<'_>) -> Result<PathBuf, ApiError> {
    ctx.db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(io_error)
}
fn is_app_data(ctx: &Context<'_>, path: &Path) -> Result<bool, ApiError> {
    if path.starts_with(data_dir(ctx)?) {
        return Ok(true);
    }
    match fs::metadata(path) {
        Ok(metadata) => protected_identity(ctx, &metadata),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_error(e)),
    }
}
// A hardlink has no canonical "original" pathname. Match the opened object's
// real (device,inode) against app-private entries, without reading their bytes.
// Only multiply-linked ordinary files need the walk. An incomplete/oversized
// walk fails closed to a human card rather than granting uncertain authority.
fn protected_identity(ctx: &Context<'_>, metadata: &fs::Metadata) -> Result<bool, ApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file() || metadata.nlink() <= 1 {
            return Ok(false);
        }
        let identity = (metadata.dev(), metadata.ino());
        let mut directories = vec![(data_dir(ctx)?, 0)];
        let mut visited = 0;
        while let Some((directory, depth)) = directories.pop() {
            let Ok(entries) = fs::read_dir(directory) else {
                return Ok(true);
            };
            for entry in entries {
                visited += 1;
                if visited > 4096 {
                    return Ok(true);
                }
                let Ok(entry) = entry else {
                    return Ok(true);
                };
                let Ok(other) = fs::symlink_metadata(entry.path()) else {
                    return Ok(true);
                };
                if (other.dev(), other.ino()) == identity {
                    return Ok(true);
                }
                if other.is_dir() {
                    if depth >= 16 {
                        return Ok(true);
                    }
                    directories.push((entry.path(), depth + 1));
                }
            }
        }
    }
    Ok(false)
}
pub fn granted(ctx: &Context<'_>, call: &ToolCall) -> Result<Option<String>, ApiError> {
    if !is_read(&call.name) {
        return Ok(None);
    }
    let a: FileArgs = args(call)?;
    let Ok(resolved) = absolute(&a.path)?.canonicalize() else {
        return Ok(None);
    };
    // A broad home/Library grant is not consent to expose provider tokens,
    // vault storage, history or other app-internal data to the model.
    if is_app_data(ctx, &resolved)? {
        return Ok(None);
    }
    let mut stmt = ctx
        .connection
        .prepare("SELECT path FROM wrapper_local_folders")?;
    for folder in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let folder = folder?;
        // A replaced/symlinked grant does not silently grant a different folder.
        if Path::new(&folder).canonicalize().ok().as_deref() == Some(Path::new(&folder))
            && resolved.starts_with(&folder)
        {
            return Ok(Some(folder));
        }
    }
    Ok(None)
}
fn read_bytes(path: &Path) -> Result<Vec<u8>, ApiError> {
    read_bytes_checked(path, |_| Ok(()))
}
fn read_bytes_checked(
    path: &Path,
    check: impl FnOnce(&fs::Metadata) -> Result<(), ApiError>,
) -> Result<Vec<u8>, ApiError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // Avoid blocking on FIFOs/devices and never follow a swapped final symlink.
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x100 | 0x4);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000 | 0x800);
    }
    let file = options.open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() {
        return Err(ApiError::new(
            400,
            "Seuls les fichiers ordinaires sont lisibles.",
        ));
    }
    check(&metadata)?;
    let mut bytes = Vec::new();
    file.take(1048577)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > 1048576 {
        return Err(ApiError::new(
            400,
            "Fichier trop volumineux (1 Mio maximum).",
        ));
    }
    Ok(bytes)
}
fn authority(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<grokbot_engine::contracts::AuthorizedBy, ApiError> {
    let authority: Option<String> = ctx
        .connection
        .query_row(
            "SELECT authorized_by FROM engine_external_tools WHERE run_id=?1 AND call_id=?2",
            params![ctx.run_id, call.call_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    authority
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or_else(|| {
            ApiError::new(
                403,
                "Une autorisation durable est requise pour cette action.",
            )
        })
}
fn display(ctx: &Context<'_>, request_id: &str) -> Result<Option<Value>, ApiError> {
    let text: Option<String> = ctx
        .connection
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [request_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(text.and_then(|s| serde_json::from_str(&s).ok()))
}
fn require_human_data_read(
    ctx: &Context<'_>,
    authority: &grokbot_engine::contracts::AuthorizedBy,
) -> Result<(), ApiError> {
    let human = if let grokbot_engine::contracts::AuthorizedBy::AllowOnce { request_id } = authority
    {
        ctx.connection.query_row("SELECT EXISTS(SELECT 1 FROM forms_requests r JOIN forms_resolutions s ON s.resolution_id=r.resolution_id AND s.entry_id=r.entry_id WHERE r.id=?1 AND r.run_id=?2 AND r.status='resolved' AND s.submission_id GLOB 'user:*')", params![request_id,ctx.run_id], |r| r.get::<_,bool>(0))?
    } else {
        false
    };
    if !human {
        return Err(ApiError::new(
            403,
            "Les données d’Ezerbot nécessitent votre décision dans une carte individuelle.",
        ));
    }
    Ok(())
}
fn read_path(ctx: &Context<'_>, call: &ToolCall) -> Result<PathBuf, ApiError> {
    use grokbot_engine::contracts::AuthorizedBy;
    let a: FileArgs = args(call)?;
    let path = absolute(&a.path)?.canonicalize().map_err(io_error)?;
    let authority = authority(ctx, call)?;
    if is_app_data(ctx, &path)? {
        require_human_data_read(ctx, &authority)?;
    }
    // A legacy standing read rule is not an individual approval or a bound
    // folder grant. Fail closed even if such an old claim reaches this boundary.
    if matches!(&authority, AuthorizedBy::Rule { .. }) {
        return Err(ApiError::new(
            403,
            "Cette lecture nécessite une nouvelle carte individuelle.",
        ));
    }
    if let AuthorizedBy::AllowOnce { request_id } = &authority {
        let display = display(ctx, request_id)?;
        let matches = match display
            .as_ref()
            .and_then(|v| v["apercu_local"].get("cible_reelle"))
        {
            Some(Value::String(target)) => Path::new(target) == path,
            // An absent target may appear as an ordinary file, but cannot become
            // a link to some unshown target after the individual approval.
            Some(Value::Null) => path == absolute(&a.path)?,
            _ => false,
        };
        if !matches {
            return Err(ApiError::new(
                409,
                "La cible réelle a changé depuis l’approbation. Demandez une nouvelle lecture.",
            ));
        }
    }
    if let AuthorizedBy::FolderGrant { path: root } = authority {
        if is_app_data(ctx, &path)? {
            return Err(ApiError::new(
                403,
                "Les données d’Ezerbot nécessitent une carte individuelle.",
            ));
        }
        let exists: bool = ctx.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM wrapper_local_folders WHERE path=?1)",
            [&root],
            |r| r.get(0),
        )?;
        if !exists
            || Path::new(&root).canonicalize().ok().as_deref() != Some(Path::new(&root))
            || !path.starts_with(&root)
        {
            return Err(ApiError::new(
                403,
                "Le dossier accordé a été retiré ou le chemin a changé.",
            ));
        }
    }
    Ok(path)
}
pub(crate) fn execute(ctx: &Context<'_>, call: &ToolCall, cancel: Arc<AtomicBool>) -> ToolOutcome {
    let result = (|| -> Result<Value, ApiError> {
        validate(ctx, call)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(ApiError::new(409, "Action annulée."));
        }
        match call.name.as_str() {
            "run_shell" => shell(ctx, call, args(call)?, cancel),
            "write_file" => write(ctx, call, args(call)?),
            "read_file" => {
                let path = read_path(ctx, call)?;
                let bytes = read_bytes_checked(&path, |metadata| {
                    if protected_identity(ctx, metadata)? {
                        require_human_data_read(ctx, &authority(ctx, call)?)?;
                    }
                    Ok(())
                })?;
                if read_path(ctx, call)? != path {
                    return Err(ApiError::new(409, "Le chemin a changé pendant la lecture."));
                }
                let text = String::from_utf8(bytes.clone())
                    .map_err(|_| ApiError::new(400, "Ce fichier n’est pas un texte UTF-8."))?;
                Ok(
                    json!({"path":path,"content":text,"sha256":format!("{:x}",Sha256::digest(&bytes))}),
                )
            }
            _ => {
                let path = read_path(ctx, call)?;
                let mut entries = Vec::new();
                let mut truncated = false;
                for entry in fs::read_dir(&path).map_err(io_error)? {
                    let entry = entry.map_err(io_error)?;
                    if entries.len() == 1000 {
                        truncated = true;
                        break;
                    }
                    let kind = entry.file_type().map_err(io_error)?;
                    entries.push(json!({"name":entry.file_name().to_string_lossy(),"kind":if kind.is_dir(){"dossier"}else if kind.is_symlink(){"lien"}else{"fichier"}}));
                }
                if read_path(ctx, call)? != path {
                    return Err(ApiError::new(409, "Le chemin a changé pendant la lecture."));
                }
                entries.sort_by_key(|e| e["name"].as_str().unwrap_or_default().to_owned());
                Ok(json!({"path":path,"entries":entries,"truncated":truncated}))
            }
        }
    })();
    match result {
        Ok(output) => ToolOutcome::Done { output },
        Err(e) => ToolOutcome::Failed { reason: e.message },
    }
}
// Serialize writes to a canonical destination across all runs/bots in this host.
// A competing approved overwrite must re-check its expected content after the
// first writer finishes, rather than both winning a check-then-rename race.
fn destination_lock(path: &Path) -> Result<Arc<Mutex<()>>, ApiError> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| ApiError::new(409, "Verrou d’écriture indisponible."))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}
fn write_destination(value: &str) -> Result<PathBuf, ApiError> {
    let path = absolute(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::new(400, "Chemin de fichier invalide."))?
        .canonicalize()
        .map_err(io_error)?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| ApiError::new(400, "Nom de fichier requis."))?,
    ))
}
fn write(ctx: &Context<'_>, call: &ToolCall, a: WriteArgs) -> Result<Value, ApiError> {
    use std::io::Write;
    let path = write_destination(&a.path)?;
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::new(400, "Chemin de fichier invalide."))?;
    if let grokbot_engine::contracts::AuthorizedBy::AllowOnce { request_id } = authority(ctx, call)?
    {
        let approved = display(ctx, &request_id)?;
        if approved
            .as_ref()
            .and_then(|v| v["apercu_local"]["cible_reelle"].as_str())
            .map(Path::new)
            != Some(path.as_path())
        {
            return Err(ApiError::new(
                409,
                "La destination réelle a changé depuis l’approbation.",
            ));
        }
    }
    let lock = destination_lock(&path)?;
    let _guard = lock
        .lock()
        .map_err(|_| ApiError::new(409, "Verrou d’écriture indisponible."))?;
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(ApiError::new(
            400,
            "Écriture à travers un lien symbolique refusée.",
        ));
    }
    if let Some(expected) = &a.expected_sha256 {
        let bytes = read_bytes(&path)?;
        if format!("{:x}", Sha256::digest(&bytes)) != expected.to_lowercase() {
            return Err(ApiError::new(
                409,
                "Le fichier a changé. Relisez-le avant de proposer une nouvelle écriture.",
            ));
        }
    } else if path.try_exists().map_err(io_error)? {
        return Err(ApiError::new(
            409,
            "Le fichier existe déjà. Relisez-le pour obtenir son empreinte.",
        ));
    }
    // Stage then rename; create_new/hard_link prevents a create from overwriting
    // an independently-created destination. No parent directories are created.
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let tmp = parent.join(format!(
        ".ezerbot-{}-{}-{}.tmp",
        std::process::id(),
        crate::now(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Preserve a private staging mode for replacements. New files follow
        // normal creation permissions (0666 filtered by the process umask).
        options.mode(if a.expected_sha256.is_some() {
            0o600
        } else {
            0o666
        });
    }
    // A failed create must never unlink another writer's temporary file.
    let mut f = options.open(&tmp).map_err(io_error)?;
    let result = (|| {
        f.write_all(a.content.as_bytes()).map_err(io_error)?;
        f.sync_all().map_err(io_error)?;
        if let Some(expected) = &a.expected_sha256 {
            if format!("{:x}", Sha256::digest(read_bytes(&path)?)) != expected.to_lowercase() {
                return Err(ApiError::new(
                    409,
                    "Le fichier a changé pendant la préparation. Écriture annulée.",
                ));
            }
            f.set_permissions(fs::metadata(&path).map_err(io_error)?.permissions())
                .map_err(io_error)?;
            f.sync_all().map_err(io_error)?;
            fs::rename(&tmp, &path).map_err(io_error)?;
        } else {
            fs::hard_link(&tmp, &path).map_err(io_error)?;
        }
        Ok(
            json!({"path":path,"bytes":a.content.len(),"sha256":format!("{:x}",Sha256::digest(a.content.as_bytes()))}),
        )
    })();
    let _ = fs::remove_file(&tmp);
    result
}

#[cfg(unix)]
fn shell(
    ctx: &Context<'_>,
    call: &ToolCall,
    a: ShellArgs,
    cancel: Arc<AtomicBool>,
) -> Result<Value, ApiError> {
    use std::os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    };
    let cwd = absolute(&a.cwd)?.canonicalize().map_err(io_error)?;
    if !cwd.is_dir() {
        return Err(ApiError::new(400, "Le dossier de travail n’existe pas."));
    }
    // Socket pairs allow bounded drain even if a command deliberately detaches
    // into another process group and keeps inherited output descriptors open.
    let (out_reader, out_writer) = UnixStream::pair().map_err(io_error)?;
    let (err_reader, err_writer) = UnixStream::pair().map_err(io_error)?;
    out_reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(io_error)?;
    err_reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(io_error)?;
    let finished = Arc::new(AtomicBool::new(false));
    // A process-local Drop cannot run after SIGTERM/SIGKILL. Make an owned
    // watchdog the process-group leader: it kills this group if the host dies.
    // No signal-handler unsafe code or new dependency is needed. Normal cleanup
    // also kills and reaps the watchdog, so it never outlives a completed job.
    struct Group(std::process::Child, Arc<AtomicBool>);
    impl Drop for Group {
        fn drop(&mut self) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.0.id())])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = self.0.wait();
            self.1.store(true, Ordering::SeqCst);
        }
    }
    let watchdog = Command::new("/bin/sh")
        .args([
            "-c",
            // The parent alone owns the pipe writer (CLOEXEC). EOF also works
            // after SIGKILL, without PID polling or a sleep/fork loop.
            "read -r _; /bin/kill -KILL -- -$$",
        ])
        .current_dir(&cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(io_error)?;
    let group = Group(watchdog, finished.clone());
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(&a.command)
        .current_dir(&cwd)
        .env_clear()
        .env(
            "PATH",
            "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        )
        .env("LANG", "fr_CA.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(out_writer)))
        .stderr(Stdio::from(OwnedFd::from(err_writer)))
        .process_group(group.0.id() as i32);
    for key in ["HOME", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command.spawn().map_err(io_error)?;
    drop(command); // Close the parent's copies of the child-side descriptors.
    let quota = Arc::new(AtomicUsize::new(a.output_limit));
    let stdout_piece = Arc::new(Mutex::new(super::shell_tasks::Piece::default()));
    let stderr_piece = Arc::new(Mutex::new(super::shell_tasks::Piece::default()));
    let reader = |mut pipe: UnixStream,
                  quota: Arc<AtomicUsize>,
                  finished: Arc<AtomicBool>,
                  slot: Arc<Mutex<super::shell_tasks::Piece>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let mut truncated = false;
            let mut allocated = 0usize;
            let mut incomplete = false;
            let mut draining = None;
            loop {
                if finished.load(Ordering::SeqCst) {
                    let start = draining.get_or_insert_with(Instant::now);
                    if start.elapsed() >= Duration::from_millis(250) {
                        incomplete = true;
                        break;
                    }
                }
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = quota
                            .try_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                                Some(remaining.saturating_sub(n))
                            })
                            .expect("quota update always succeeds");
                        let keep = n.min(remaining);
                        allocated += keep;
                        bytes.extend_from_slice(&buffer[..keep]);
                        truncated |= keep < n;
                        if let Ok(mut piece) = slot.lock() {
                            piece.text = String::from_utf8_lossy(&bytes).into_owned();
                            piece.truncated = truncated;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        if finished.load(Ordering::SeqCst) {
                            incomplete = true;
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        incomplete = true;
                        break;
                    }
                }
            }
            let mut text = String::from_utf8_lossy(&bytes).into_owned();
            if text.len() > allocated {
                let mut end = allocated;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
                truncated = true;
            }
            (text, truncated, incomplete)
        })
    };
    let out = reader(
        out_reader,
        quota.clone(),
        finished.clone(),
        stdout_piece.clone(),
    );
    let err = reader(err_reader, quota, finished.clone(), stderr_piece.clone());
    if a.background {
        let run_id = ctx.run_id.unwrap_or("").to_string();
        if run_id.is_empty() {
            finished.store(true, Ordering::SeqCst);
            drop(group);
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(ApiError::new(
                400,
                "Cette tâche n’est pas rattachée à un tour.",
            ));
        }
        let call_id = call.call_id.clone();
        if let Err(error) = super::shell_tasks::insert(ctx.connection, &run_id, &call_id) {
            finished.store(true, Ordering::SeqCst);
            drop(group);
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err(error);
        }
        let pgid = group.0.id();
        let control = super::shell_tasks::register(ctx.db_path, &run_id, &call_id, pgid);
        let db = ctx.db_path.to_path_buf();
        let stop = control.stop.clone();
        std::thread::spawn(move || {
            use std::os::unix::process::ExitStatusExt;
            let _done = super::shell_tasks::DoneFlag(control.done);
            let live = |stdout: &super::shell_tasks::Piece, stderr: &super::shell_tasks::Piece| {
                super::shell_tasks::Snap {
                    status: "en cours",
                    stdout: stdout.text.clone(),
                    stderr: stderr.text.clone(),
                    truncated: stdout.truncated || stderr.truncated,
                    incomplete: stdout.incomplete || stderr.incomplete,
                    exit_code: None,
                    signal: None,
                }
            };
            loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {}
                    Err(_) => break,
                }
                if let (Ok(stdout), Ok(stderr)) = (stdout_piece.lock(), stderr_piece.lock()) {
                    let _ = super::shell_tasks::publish(
                        &db,
                        &run_id,
                        &call_id,
                        &live(&stdout, &stderr),
                    );
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            finished.store(true, Ordering::SeqCst);
            if stop.load(Ordering::SeqCst) {
                super::shell_tasks::kill_group(pgid);
            }
            let status = child.wait().ok();
            drop(group);
            let (stdout, t1, i1) = out.join().unwrap_or_else(|_| (String::new(), false, true));
            let (stderr, t2, i2) = err.join().unwrap_or_else(|_| (String::new(), false, true));
            let stopped = stop.load(Ordering::SeqCst);
            let code = status.as_ref().and_then(|status| status.code());
            let signal = status.as_ref().and_then(|status| status.signal());
            let snap = if stopped {
                super::shell_tasks::Snap {
                    status: "arrêtée",
                    stdout,
                    stderr,
                    truncated: t1 || t2,
                    incomplete: i1 || i2,
                    exit_code: None,
                    signal: None,
                }
            } else {
                super::shell_tasks::Snap {
                    status: if code == Some(0) {
                        "terminé"
                    } else {
                        "échec"
                    },
                    stdout,
                    stderr,
                    truncated: t1 || t2,
                    incomplete: i1 || i2,
                    exit_code: code,
                    signal,
                }
            };
            for _ in 0..100 {
                match super::shell_tasks::publish(&db, &run_id, &call_id, &snap) {
                    Ok(
                        super::shell_tasks::Publish::Updated | super::shell_tasks::Publish::Closed,
                    ) => break,
                    _ => std::thread::sleep(Duration::from_millis(20)),
                }
            }
            super::shell_tasks::finish(&db, &run_id, &call_id);
        });
        return Ok(
            json!({"command":a.command,"cwd":cwd,"status":"en cours","exit_code":Value::Null,"signal":Value::Null,"stdout":"","stderr":"","truncated":false,"output_incomplete":false,"background":true}),
        );
    }
    let start = Instant::now();
    let mut state = "terminé";
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            state = "annulé";
            break None;
        }
        if start.elapsed() >= Duration::from_millis(a.timeout_ms) {
            state = "délai dépassé";
            break None;
        }
        match child.try_wait().map_err(io_error)? {
            Some(status) => {
                if !status.success() {
                    state = "échec";
                }
                break Some(status);
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    drop(group); // Also stop background descendants when their parent exits.
    let status = status.or_else(|| child.wait().ok());
    use std::os::unix::process::ExitStatusExt;
    let signal = status.as_ref().and_then(|s| s.signal());
    let (stdout, t1, i1) = out.join().unwrap_or_else(|_| (String::new(), false, true));
    let (stderr, t2, i2) = err.join().unwrap_or_else(|_| (String::new(), false, true));
    Ok(
        json!({"command":a.command,"cwd":cwd,"status":state,"exit_code":status.and_then(|s|s.code()),"signal":signal,"stdout":stdout,"stderr":stderr,"truncated":t1||t2,"output_incomplete":i1||i2}),
    )
}
#[cfg(not(unix))]
fn shell(
    _: &Context<'_>,
    _: &ToolCall,
    _: ShellArgs,
    _: Arc<AtomicBool>,
) -> Result<Value, ApiError> {
    Err(ApiError::new(
        400,
        "Le terminal local requiert macOS ou Unix.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_engine::{
        contracts::{with_tx, RunBudgets, RunKind, Trigger},
        model::{self, Run, RunStatus},
    };
    #[test]
    fn admitted_non_individual_authority_cannot_read_app_data_at_the_effect_boundary() {
        let root = std::env::temp_dir().join(format!(
            "ezerbot-data-boundary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("data")).unwrap();
        let app = App::open(&root.join("data/workspace.sqlite")).unwrap();
        let protected = root.join("data/identifiants-factices.txt");
        fs::write(&protected, "secret factice").unwrap();
        let ordinary = root.join("document.txt");
        fs::write(&ordinary, "document factice").unwrap();
        let canonical = root.canonicalize().unwrap();
        with_tx(&app.connection, |conn| {
            model::start(
                conn,
                &Run {
                    run_id: "boundary".into(),
                    bot_id: "grokbot".into(),
                    conversation_id: "test".into(),
                    trigger: Trigger::UserMessage {
                        message_id: "test".into(),
                        attempt: 0,
                    },
                    kind: RunKind::Chat,
                    status: RunStatus::Running,
                    task_status: None,
                    turn_end: None,
                    budgets: RunBudgets::default(),
                    started_at: 0,
                },
            )
        })
        .unwrap();
        app.connection
            .execute(
                "INSERT INTO wrapper_local_folders(path) VALUES (?1)",
                [canonical.to_string_lossy().as_ref()],
            )
            .unwrap();
        app.connection.execute("INSERT INTO engine_external_tools(run_id,call_id,authorized_by) VALUES ('boundary','read',?1)",[json!({"kind":"folder_grant","path":canonical}).to_string()]).unwrap();
        let ctx = Context {
            run_id: Some("boundary"),
            ..app.feature_context()
        };
        let mut call = ToolCall {
            call_id: "read".into(),
            name: "read_file".into(),
            arguments: json!({"path":protected}),
        };
        assert!(read_path(&ctx, &call)
            .unwrap_err()
            .message
            .contains("carte individuelle"));
        call.arguments = json!({"path":ordinary});
        assert_eq!(
            read_path(&ctx, &call).unwrap(),
            ordinary.canonicalize().unwrap()
        );
        app.connection.execute("UPDATE engine_external_tools SET authorized_by=?1 WHERE run_id='boundary' AND call_id='read'",[json!({"kind":"rule","rule_id":"legacy"}).to_string()]).unwrap();
        call.arguments = json!({"path":protected});
        assert!(read_path(&ctx, &call)
            .unwrap_err()
            .message
            .contains("carte individuelle"));
        let alias = root.join("lien-dur.txt");
        fs::hard_link(&protected, &alias).unwrap();
        app.connection.execute("UPDATE engine_external_tools SET authorized_by=?1 WHERE run_id='boundary' AND call_id='read'",[json!({"kind":"folder_grant","path":canonical}).to_string()]).unwrap();
        call.arguments = json!({"path":alias});
        assert!(read_path(&ctx, &call)
            .unwrap_err()
            .message
            .contains("carte individuelle"));
        app.connection
            .execute("INSERT INTO wrapper_local_bots VALUES ('grokbot',0,1)", [])
            .unwrap();
        let ctx = Context {
            bot_id: Some("grokbot"),
            ..ctx
        };
        // A committed AllowOnce is insufficient when its durable receipt came
        // from a legacy model/standing reviewer rather than the user endpoint.
        for (index, actor) in ["auto:review:test", "auto:standing:test", "user:manual"]
            .iter()
            .enumerate()
        {
            let id = format!("receipt-{index}");
            with_tx(&app.connection, |conn| -> grokbot_engine::contracts::Result<()> {
                let request = grokbot_forms::requests::HumanInputRequest {
                    id:id.clone(),entry_id:id.clone(),run_id:"boundary".into(),
                    kind:grokbot_forms::requests::InputKind::Approval {tool:call.name.clone(),args_digest:grokbot_forms::approvals::args_digest(&call.name,&call.arguments)?,expires_at:i64::MAX},
                    route:grokbot_engine::contracts::ApprovalRoute {requesting_bot_id:"grokbot".into(),target_bot_id:"grokbot".into(),target_conversation_id:"test".into()},
                    status:grokbot_forms::requests::RequestStatus::Pending,
                };
                grokbot_forms::requests::create(conn,&request)?;
                grokbot_forms::approvals::decide(conn,&id,"test",actor,grokbot_forms::approvals::ApprovalDecision::AllowOnce,0)?;
                grokbot_forms::approvals::consume_allow_once(conn,&id,"boundary",&call.name,&call.arguments,1)?;
                conn.execute("INSERT INTO wrapper_forms_calls VALUES (?1,'binding',?2)",params![id,json!({"apercu_local":{"cible_reelle":alias.canonicalize().unwrap()}}).to_string()])?;
                conn.execute("UPDATE engine_external_tools SET authorized_by=?1 WHERE run_id='boundary' AND call_id='read'",[json!({"kind":"allow_once","request_id":id}).to_string()])?;
                Ok(())
            }).unwrap();
            let outcome = execute(&ctx, &call, Arc::new(AtomicBool::new(false)));
            if actor.starts_with("auto:") {
                assert!(
                    matches!(outcome,ToolOutcome::Failed {reason} if reason.contains("votre décision"))
                );
            } else {
                assert!(
                    matches!(outcome,ToolOutcome::Done {output} if output["content"]=="secret factice")
                );
            }
        }
        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
}
