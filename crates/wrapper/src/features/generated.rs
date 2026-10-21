//! Local generated documents. REA OTH-S30/35 establishes document surfaces, not
//! server generation. Originals and safe SVG previews are attachment BLOBs in
//! SQLite; only an approved Open/Finder action materializes an immutable cache.
//! Save is create-new only. External actions use the engine's at-most-once journal.
use super::Context;
use crate::{parse, ApiError, ApiResponse, App};
use grokbot_attachments::generation::{self, Request};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

empty_hooks!(state, tick, context, entry, on_run_event, link_attachments);
pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments_generated (
        id TEXT PRIMARY KEY, record TEXT NOT NULL, bytes BLOB NOT NULL,
        preview TEXT NOT NULL, created_at INTEGER NOT NULL);",
    )?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct File {
    pub id: String,
    pub name: String,
    pub format: String,
    pub byte_length: usize,
    pub pages: usize,
    pub preview_url: String,
}
pub fn get(conn: &Connection, id: &str) -> Result<File, ApiError> {
    valid_id(id)?;
    let text: Option<String> = conn
        .query_row(
            "SELECT record FROM attachments_generated WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    serde_json::from_str(&text.ok_or_else(|| {
        ApiError::new(
            404,
            "Ce document généré est introuvable. Demandez au bot de le recréer.",
        )
    })?)
    .map_err(|_| ApiError::new(500, "Les métadonnées du document sont illisibles."))
}
fn valid_id(id: &str) -> Result<(), ApiError> {
    if !id.strip_prefix("generated_").is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(ApiError::new(400, "Référence de document généré invalide."));
    }
    Ok(())
}
pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name:"create_document".into(),
        description:"Créer un fichier local dans la conversation : pdf, docx, pptx, md, csv, xlsx ou diagramme svg. name est un simple nom de fichier. content : texte brut (en DOCX, les lignes # titre et - puce sont mises en forme ; en PDF, les marqueurs Markdown restent du texte), diapositives PPTX séparées par une ligne --- (16 lignes par diapositive ; titre coupé à 50 caractères, texte à 80), tableau CSV/XLSX sous forme JSON de lignes de chaînes [[\"Nom\",\"Coût\"],[\"Été\",\"12\"]], ou diagramme SVG en Mermaid flowchart TD/LR (A[Texte] --> B{Choix} -->|oui| C(Fin), liens -->, ---, -.->, ==> ; style, click et sous-graphes ignorés ; 20 nœuds). Autres types Mermaid non pris en charge. Maximum 100 000 octets, 50 diapositives. PDF Courier prend en charge le français mais pas les emoji. Aucun accès extérieur ; la carte permet ensuite d’ouvrir ou enregistrer avec approbation.".into(),
        schema:json!({"type":"object","additionalProperties":false,"properties":{"name":{"type":"string"},"format":{"type":"string","enum":["pdf","docx","pptx","md","csv","xlsx","svg"]},"content":{"type":"string"}},"required":["name","format","content"]}),requires_approval:false,
    },ToolSpec {
        name:"document_action".into(), description:"Ouvrir un document généré, l’afficher dans le Finder ou en enregistrer une copie sur le Mac, après approbation. id vient de create_document. action : open, reveal ou save ; path=null sauf save (chemin absolu d’un NOUVEAU fichier, même extension que le format, aucun remplacement ; nécessite « Écriture de fichier » activée pour ce bot).".into(),
        schema:json!({"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"},"action":{"type":"string","enum":["open","reveal","save"]},"path":{"type":["string","null"]}},"required":["id","action","path"]}),requires_approval:true,
    }]
}
pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if call.name != "create_document" {
        return None;
    }
    let result = (|| -> Result<Value, ApiError> {
        let req: Request = serde_json::from_value(call.arguments.clone()).map_err(|_| {
            ApiError::new(
                400,
                "Arguments de génération invalides : nom, format et contenu sont requis.",
            )
        })?;
        let generated = generation::generate(&req).map_err(|e| match e {
            grokbot_engine::contracts::Error::InvalidInput(s) => ApiError::new(400, s),
            _ => ApiError::new(
                500,
                "La génération du document a échoué. Réessayez avec un contenu plus court.",
            ),
        })?;
        let id = format!(
            "generated_{:x}",
            Sha256::digest([generated.name.as_bytes(), b"\0", &generated.bytes].concat())
        );
        let file = File {
            preview_url: format!("/api/generated/{id}/preview"),
            id,
            name: generated.name,
            format: generated.format,
            byte_length: generated.bytes.len(),
            pages: generated.pages,
        };
        ctx.connection.execute(
            "INSERT INTO attachments_generated VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
            params![
                file.id,
                serde_json::to_string(&file).map_err(|_| ApiError::new(
                    500,
                    "Le document généré n’a pas pu être enregistré."
                ))?,
                generated.bytes,
                generated.preview,
                ctx.now_ms
            ],
        )?;
        Ok(json!(file))
    })();
    Some(outcome(result))
}
fn outcome(result: Result<Value, ApiError>) -> ToolOutcome {
    match result {
        Ok(output) => ToolOutcome::Done { output },
        Err(e) => ToolOutcome::Failed { reason: e.message },
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    id: String,
    action: String,
    path: Option<String>,
}
fn args(call: &ToolCall) -> Result<Action, ApiError> {
    serde_json::from_value(call.arguments.clone())
        .map_err(|_| ApiError::new(400, "Arguments de l’action sur le document invalides."))
}
pub fn is_external(name: &str) -> bool {
    name == "document_action"
}
fn destination(value: &str) -> Result<PathBuf, ApiError> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || value.len() > 4096
        || value.contains('\0')
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(ApiError::new(
            400,
            "Choisissez un chemin absolu sans « .. » pour enregistrer la copie.",
        ));
    }
    // Canonical parent makes aliasing visible before approval. Refuse a change
    // after approval, rather than writing through a replaced directory symlink.
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::new(400, "Le dossier de destination est invalide."))?;
    if parent.canonicalize().map_err(io)? != parent {
        return Err(ApiError::new(
            400,
            "Utilisez le chemin réel du dossier de destination, sans lien symbolique.",
        ));
    }
    Ok(path)
}
/// True when `path` or one of its ancestors is the same directory as `data`,
/// by device and inode: an alias such as /System/Volumes/Data/Users/… cannot
/// slip past a textual prefix check.
fn inside_directory(path: &Path, data: &Path) -> Result<bool, ApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let data = fs::metadata(data).map_err(io)?;
        for ancestor in path.ancestors() {
            match fs::metadata(ancestor) {
                Ok(m) if (m.dev(), m.ino()) == (data.dev(), data.ino()) => return Ok(true),
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
        }
        Ok(false)
    }
    #[cfg(not(unix))]
    Ok(path.starts_with(data))
}
pub fn validate(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    let a = args(call)?;
    let file = get(ctx.connection, &a.id)?;
    match a.action.as_str() {
        "save" => {
            let p = destination(
                a.path
                    .as_deref()
                    .ok_or_else(|| ApiError::new(400, "Le chemin de la copie est requis."))?,
            )?;
            // The copy's type must stay the generated format: free model text
            // saved as .md cannot become a .command, .plist or .sh.
            if p.extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref()
                != Some(file.format.as_str())
            {
                return Err(ApiError::new(
                    400,
                    format!("L’extension de la copie doit être .{}.", file.format),
                ));
            }
            // A save the model asks for is a file write: it honours the same
            // per-bot switch as write_file. A card button is the user's own
            // gesture (host run, or the card route with no run yet).
            let host = ctx.run_id.is_none_or(|id| id.starts_with("mcp-host-"));
            if !host
                && !super::local::enabled(ctx.connection, ctx.bot_id.unwrap_or(""), "write_file")
            {
                return Err(ApiError::new(
                    403,
                    "L’écriture de fichiers est désactivée pour ce bot dans Paramètres › Terminal et fichiers sur ce Mac. Utilisez le bouton « Enregistrer sous… » de la carte.",
                ));
            }
            if p.symlink_metadata().is_ok() {
                return Err(ApiError::new(409,"Un fichier existe déjà à cet emplacement. Choisissez un autre nom ; aucun remplacement n’est effectué."));
            }
            // A model cannot use save to modify app metadata, credentials or blobs.
            let data = ctx.db_path.parent().unwrap_or(Path::new("."));
            if inside_directory(&p, data)? {
                return Err(ApiError::new(
                    403,
                    "Enregistrez la copie en dehors du dossier de données d’Ezerbot.",
                ));
            }
        }
        "open" | "reveal" if a.path.is_none() => {}
        _ => {
            return Err(ApiError::new(
                400,
                "Action invalide : ouvrir, afficher dans le Finder ou enregistrer une copie.",
            ))
        }
    }
    Ok(())
}
pub fn approval_display(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    validate(ctx, call)?;
    let a = args(call)?;
    let file = get(ctx.connection, &a.id)?;
    Ok(
        json!({"document":file.name,"taille":file.byte_length,"action_id":a.action,"action":match a.action.as_str(){"save"=>"Enregistrer une copie (sans remplacer)","open"=>"Ouvrir avec l’application par défaut",_=>"Afficher dans le Finder"},"destination":a.path,"id":a.id}),
    )
}
fn io(e: std::io::Error) -> ApiError {
    ApiError::new(
        400,
        match e.kind() {
            std::io::ErrorKind::AlreadyExists => {
                "Un fichier existe déjà. Aucun remplacement n’a été effectué."
            }
            std::io::ErrorKind::PermissionDenied => {
                "macOS refuse l’accès au fichier ou au dossier."
            }
            std::io::ErrorKind::NotFound => "Le dossier de destination est introuvable.",
            _ => "L’action sur le document a échoué. Vérifiez le chemin et les permissions.",
        },
    )
}
/// Known extension and digest only; never a model-supplied path inside storage.
pub fn cache_path(base: &Path, file: &File) -> Result<PathBuf, ApiError> {
    valid_id(&file.id)?;
    if !["pdf", "docx", "pptx", "md", "csv", "xlsx", "svg"].contains(&file.format.as_str()) {
        return Err(ApiError::new(500, "Format enregistré invalide."));
    }
    let mut directory = base.as_os_str().to_os_string();
    directory.push(".attachments");
    Ok(PathBuf::from(directory).join(format!("{}.{}", &file.id[10..], file.format)))
}
fn bytes(conn: &Connection, id: &str) -> Result<Vec<u8>, ApiError> {
    Ok(conn.query_row(
        "SELECT bytes FROM attachments_generated WHERE id=?1",
        [id],
        |r| r.get(0),
    )?)
}
fn verify_admitted(ctx: &Context<'_>, call: &ToolCall) -> Result<(), ApiError> {
    let denied = || {
        ApiError::new(
            403,
            "Une approbation durable est requise pour cette action sur le document.",
        )
    };
    let run_id = ctx.run_id.ok_or_else(denied)?;
    let run = grokbot_engine::store::required(ctx.connection, run_id)?;
    if !ctx.connection.is_autocommit()
        || run.status != grokbot_engine::model::RunStatus::Running
        || ctx.bot_id != Some(run.bot_id.as_str())
        || ctx.conversation_id != Some(run.conversation_id.as_str())
    {
        return Err(denied());
    }
    let authority:Option<String>=ctx.connection.query_row("SELECT authorized_by FROM engine_external_tools WHERE run_id=?1 AND call_id=?2 AND outcome IS NULL",params![run_id,call.call_id],|r|r.get(0)).optional()?.flatten();
    let authority = authority
        .and_then(|s| serde_json::from_str::<grokbot_engine::contracts::AuthorizedBy>(&s).ok());
    let bound = grokbot_engine::store::events(ctx.connection, run_id, 0)?
        .into_iter()
        .any(|e| {
            e.kind == grokbot_engine::contracts::RunEventKind::ToolCall
                && e.payload["call_id"] == call.call_id
                && e.payload["name"] == call.name
                && e.payload["arguments"] == call.arguments
        });
    if !bound
        || !matches!(
            authority,
            Some(
                grokbot_engine::contracts::AuthorizedBy::AllowOnce { .. }
                    | grokbot_engine::contracts::AuthorizedBy::Rule { .. }
            )
        )
    {
        return Err(denied());
    }
    Ok(())
}
pub fn execute(ctx: &Context<'_>, call: &ToolCall, cancel: Arc<AtomicBool>) -> ToolOutcome {
    outcome((|| -> Result<Value, ApiError> {
        verify_admitted(ctx, call)?;
        validate(ctx, call)?;
        if cancel.load(Ordering::SeqCst) {
            return Err(ApiError::new(409, "Action annulée avant son exécution."));
        }
        let a = args(call)?;
        let file = get(ctx.connection, &a.id)?;
        let data = bytes(ctx.connection, &a.id)?;
        if a.action == "save" {
            let path = destination(a.path.as_deref().unwrap())?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut output = options.open(&path).map_err(io)?;
            output
                .write_all(&data)
                .and_then(|_| output.sync_all())
                .map_err(io)?;
            Ok(json!({"message":"Copie enregistrée.","path":path}))
        } else {
            let path = cache_path(ctx.db_path, &file)?;
            let parent = path.parent().unwrap();
            match fs::symlink_metadata(parent) {
                Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
                    return Err(ApiError::new(
                        409,
                        "Le dossier des pièces jointes n’est pas un dossier local régulier.",
                    ))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(parent).map_err(io)?
                }
                Err(e) => return Err(io(e)),
            }
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut f) => {
                    f.write_all(&data).and_then(|_| f.sync_all()).map_err(io)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let m = fs::symlink_metadata(&path).map_err(io)?;
                    if !m.is_file()
                        || m.file_type().is_symlink()
                        || fs::read(&path).map_err(io)? != data
                    {
                        return Err(ApiError::new(
                            409,
                            "La copie locale du document a changé. Ouverture refusée.",
                        ));
                    }
                }
                Err(e) => return Err(io(e)),
            }
            #[cfg(target_os = "macos")]
            {
                let mut command = Command::new("/usr/bin/open");
                if a.action == "reveal" {
                    command.arg("-R");
                }
                let status = command
                    .arg(&path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map_err(io)?;
                if !status.success() {
                    return Err(ApiError::new(400,"macOS n’a pas pu ouvrir ce document. Enregistrez une copie puis choisissez une application compatible."));
                }
            }
            #[cfg(not(target_os = "macos"))]
            return Err(ApiError::new(
                400,
                "L’ouverture et le Finder sont disponibles uniquement sur macOS.",
            ));
            #[allow(unreachable_code)]
            Ok(
                json!({"message":if a.action=="open"{"Document ouvert."}else{"Document affiché dans le Finder."}}),
            )
        }
    })())
}
pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method == "GET" {
        if let Some(id) = path
            .strip_prefix("/api/generated/")
            .and_then(|s| s.strip_suffix("/save-path"))
        {
            return Some((|| {
                let file = get(&app.connection, id)?;
                // A suggestion only: no directory is created and no write occurs
                // until the separate action is approved.
                let home = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .ok_or_else(|| {
                        ApiError::new(
                            400,
                            "Le dossier personnel est introuvable. Saisissez un chemin complet.",
                        )
                    })?;
                Ok(ApiResponse::Json(
                    json!({"path": home.join("Desktop").join(file.name)}),
                ))
            })());
        }
        if let Some(id) = path
            .strip_prefix("/api/generated/")
            .and_then(|s| s.strip_suffix("/preview"))
        {
            return Some((|| {
                get(&app.connection, id)?;
                let preview: String = app.connection.query_row(
                    "SELECT preview FROM attachments_generated WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )?;
                Ok(ApiResponse::Bytes {
                    content_type: "image/svg+xml".into(),
                    body: preview.into_bytes(),
                })
            })());
        }
    }
    if method != "POST" || path != "/api/generated/action" {
        return None;
    }
    Some((|| {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Input {
            id: String,
            action: String,
            path: Option<String>,
            conversation_id: String,
        }
        let input: Input = parse(body)?;
        app.require_conversation(&input.conversation_id)?;
        // Only permit a card's action in a conversation that actually references
        // this generated file, not an arbitrary guessed ID from another thread.
        let referenced:bool=app.connection.query_row("SELECT EXISTS(SELECT 1 FROM engine_run_events e JOIN engine_runs r USING(run_id) WHERE json_extract(r.record,'$.conversation_id')=?1 AND e.kind='tool_result' AND json_extract(e.payload,'$.outcome.output.id')=?2)",params![input.conversation_id,input.id],|r|r.get(0))?;
        if !referenced {
            return Err(ApiError::new(
                403,
                "Ce document n’appartient pas à cette conversation.",
            ));
        }
        let arguments = json!({"id":input.id,"action":input.action,"path":input.path});
        let mut ctx = app.feature_context();
        ctx.conversation_id = Some(&input.conversation_id);
        validate(
            &ctx,
            &ToolCall {
                call_id: "preview".into(),
                name: "document_action".into(),
                arguments: arguments.clone(),
            },
        )?;
        let conversation = super::forms::principal_conversation(&ctx, &input.conversation_id)?;
        ctx.conversation_id = Some(&conversation);
        Ok(ApiResponse::Json(super::engine::queue_approved_action(
            &ctx,
            "document_action",
            arguments,
        )?))
    })())
}
