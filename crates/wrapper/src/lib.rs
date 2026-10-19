//! Adaptation locale du moteur Rust. Les états et les corps proviennent du journal durable.

#![forbid(unsafe_code)]

pub mod deep_links;
mod mcp;
mod provider;
mod upstream;

/// Model gateway implementations are added independently of the HTTP host.
#[doc(hidden)]
pub mod gateway;

/// Capability hooks, shared by the HTTP app and future run-thread adapters.
#[doc(hidden)]
pub mod features;

// The model gateway keeps the root compatibility name while generation belongs
// to its feature module (including the offline simulated reply).
pub(crate) use features::engine::demo_reply;

pub use provider::{Endpoints, Provider, ToolCall};

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_domain::{AccountGeneration, AccountId, AuthenticatedActor, SubmissionState};
use grokbot_session::{SessionError, SubmissionSession};
use grokbot_store::SessionStore;
use grokbot_submission_journal::JournalError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const BODY_LIMIT: usize = 64 * 1024;
pub const ATTACHMENT_BODY_LIMIT: usize = 20 * 1024 * 1024;
pub const SKILL_IMPORT_BODY_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) struct AttachmentUpload {
    draft_id: String,
    upload_id: Option<String>,
    name: Option<String>,
}

fn decode_query(value: &str) -> Result<String, ApiError> {
    let invalid = || ApiError::new(400, "Les paramètres d’import sont invalides.");
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        bytes.push(match byte {
            b'%' => {
                let high = (input.next().ok_or_else(invalid)? as char)
                    .to_digit(16)
                    .ok_or_else(invalid)?;
                let low = (input.next().ok_or_else(invalid)? as char)
                    .to_digit(16)
                    .ok_or_else(invalid)?;
                (high * 16 + low) as u8
            }
            b'+' => b' ',
            other => other,
        });
    }
    String::from_utf8(bytes).map_err(|_| invalid())
}

pub(crate) fn attachment_upload_query(query: &str) -> Result<AttachmentUpload, ApiError> {
    let mut fields = BTreeMap::new();
    for pair in query.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| ApiError::new(400, "Paramètre d’import invalide."))?;
        if !matches!(key, "draft_id" | "upload_id" | "name")
            || fields.insert(key, decode_query(value)?).is_some()
        {
            return Err(ApiError::new(400, "Paramètre d’import inconnu ou répété."));
        }
    }
    let draft_id = fields
        .remove("draft_id")
        .ok_or_else(|| ApiError::new(400, "Brouillon invalide."))?;
    validate_conversation_id(&draft_id)?;
    let upload_id = fields.remove("upload_id");
    if let Some(token) = &upload_id {
        validate_id(token)?;
    }
    let name = fields.remove("name");
    if let Some(name) = &name {
        if name.is_empty()
            || name.len() > 512
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err(ApiError::new(400, "Le nom de l’image est invalide."));
        }
    }
    Ok(AttachmentUpload {
        draft_id,
        upload_id,
        name,
    })
}

fn attachment_upload_path(path: &str) -> bool {
    path == "/api/attachments"
        || path
            .strip_prefix("/api/attachments?")
            .is_some_and(|query| attachment_upload_query(query).is_ok())
}

/// Only upload routes (including their validated draft query) get a larger budget.
pub fn body_limit(path: &str) -> usize {
    if attachment_upload_path(path) {
        return ATTACHMENT_BODY_LIMIT;
    }
    match path {
        "/api/attachments" => ATTACHMENT_BODY_LIMIT,
        // Base64 upload plus a bounded filename/JSON envelope; decoded bytes
        // retain the same 20 MiB limit as images.
        "/api/documents" => ATTACHMENT_BODY_LIMIT.div_ceil(3) * 4 + 4096,
        "/api/voice/memos" => features::voice::MAX_WAV_BYTES.div_ceil(3) * 4 + 128 * 1024,
        "/api/skills" | "/api/skills/update" | "/api/skills/import" => SKILL_IMPORT_BODY_LIMIT,
        // 2 × 20 rules × 1,000 Unicode characters, including JSON escapes.
        "/api/approvals/reviewer" => 256 * 1024,
        _ => BODY_LIMIT,
    }
}

/// Shared size error for direct API calls, declared lengths and chunked bodies.
pub fn too_large(path: &str) -> ApiError {
    if path == "/api/documents" {
        return ApiError::new(413, "Le document dépasse la limite de 20 Mo.");
    }
    let limit = body_limit(path);
    let (amount, unit) = if limit >= 1024 * 1024 {
        (limit / (1024 * 1024), "Mio")
    } else {
        (limit / 1024, "Ko")
    };
    ApiError::new(
        413,
        format!("Le fichier ou la requête dépasse la taille permise ({amount} {unit})."),
    )
}

/// tiny_http accepts CR/LF in header values: reject controls and non-ASCII before encoding.
pub fn header_safe(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..0x7f).contains(&byte))
}

/// Feature routes can return binary data without JSON encoding or losing their MIME type.
#[derive(Debug)]
pub enum ApiResponse {
    Json(Value),
    Bytes { content_type: String, body: Vec<u8> },
}

impl ApiResponse {
    pub fn into_parts(self) -> (Vec<u8>, String) {
        match self {
            Self::Json(value) => (
                value.to_string().into_bytes(),
                "application/json; charset=utf-8".into(),
            ),
            Self::Bytes { content_type, body } => (body, content_type),
        }
    }
}
const ACCOUNT: &str = "local-workspace";
const DEFAULT_TITLE: &str = "Nouvelle conversation";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS wrapper_migrations (
    account_id TEXT NOT NULL,
    id TEXT NOT NULL,
    PRIMARY KEY (account_id, id)
);
CREATE TABLE IF NOT EXISTS wrapper_conversations (
    account_id TEXT NOT NULL,
    id TEXT NOT NULL,
    title TEXT NOT NULL,
    title_customized INTEGER NOT NULL DEFAULT 0,
    favorite INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    bot_id TEXT NOT NULL DEFAULT 'grokbot',
    PRIMARY KEY (account_id, id)
);
CREATE TABLE IF NOT EXISTS wrapper_bots (
    account_id TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    color TEXT NOT NULL,
    shape TEXT NOT NULL DEFAULT 'galet',
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    template TEXT,
    template_version INTEGER,
    customized INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, id)
);
CREATE TABLE IF NOT EXISTS wrapper_messages (
    ordinal INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (account_id, conversation_id, message_id)
);
CREATE TABLE IF NOT EXISTS wrapper_remote (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    body TEXT NOT NULL,
    reply TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    send_count INTEGER NOT NULL,
    PRIMARY KEY (account_id, conversation_id, message_id)
);
CREATE TABLE IF NOT EXISTS wrapper_preferences (
    account_id TEXT PRIMARY KEY,
    theme TEXT NOT NULL CHECK (theme IN ('light', 'dark', 'system')),
    accent TEXT NOT NULL DEFAULT 'citron',
    model TEXT,
    effort TEXT,
    fast INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS wrapper_generation (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('streaming', 'done', 'failed', 'stopped')),
    error TEXT,
    error_kind TEXT,
    notice TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, conversation_id, message_id)
);
CREATE TABLE IF NOT EXISTS wrapper_proposals (
    account_id TEXT NOT NULL,
    id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('create', 'update')),
    bot_id TEXT,
    payload TEXT NOT NULL,
    reason TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'dismissed', 'stale')),
    result_bot_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, id)
);
CREATE INDEX IF NOT EXISTS wrapper_proposals_message
    ON wrapper_proposals (account_id, conversation_id, message_id);
";

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl ApiError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        Self::new(500, format!("Stockage local : {error}"))
    }
}

impl From<grokbot_store::StoreError> for ApiError {
    fn from(error: grokbot_store::StoreError) -> Self {
        Self::new(500, error.to_string())
    }
}

impl From<grokbot_engine::contracts::Error> for ApiError {
    fn from(error: grokbot_engine::contracts::Error) -> Self {
        let status = match &error {
            grokbot_engine::contracts::Error::InvalidInput(_) => 400,
            grokbot_engine::contracts::Error::NotImplemented(_) => 501,
            _ => 500,
        };
        Self::new(status, error.to_string())
    }
}

impl From<SessionError> for ApiError {
    fn from(error: SessionError) -> Self {
        let status = match &error {
            SessionError::Journal(JournalError::NotFound) => 404,
            SessionError::Journal(_) | SessionError::StaleRequest | SessionError::Revoked => 409,
            _ => 500,
        };
        Self::new(status, error.to_string())
    }
}

#[derive(Debug, Serialize)]
pub struct Conversation {
    pub id: String,
    pub bot_id: String,
    pub member_ids: Vec<String>,
    pub title: String,
    pub favorite: bool,
    pub updated_at: u64,
    pub messages: Vec<Message>,
    pub kind: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Message {
    pub id: String,
    pub kind: String,
    pub payload: Value,
    pub attachments: Vec<Value>,
    pub role: &'static str,
    pub body: String,
    pub state: &'static str,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    /// Les propositions de bot faites par cette réponse ; vide pour un message de l'utilisateur.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<Proposal>,
}

/// Une proposition de bot émise par le modèle. Rien ne change tant que l'utilisateur ne l'a pas
/// acceptée. Son `id` est le `call_id` de l'appel de fonction : rejouer l'événement ne la duplique pas.
#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub id: String,
    pub conversation_id: String,
    pub message_id: String,
    /// `create` ou `update`.
    pub kind: String,
    /// Le bot visé par une modification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    /// Création : les cinq champs du bot. Modification : `changes` et `before`, les valeurs des
    /// champs visés au moment de la proposition.
    pub payload: Value,
    pub reason: String,
    /// `pending`, `accepted`, `dismissed` ou `stale`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_bot_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Les champs modifiables d'un bot, tels qu'ils arrivent de l'API ou du modèle. `None` laisse
/// la valeur en place. `validated` les passe par les mêmes bornes que les routes.
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct BotChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shape: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
}

impl BotChanges {
    fn validated(self) -> Result<Self, ApiError> {
        Ok(Self {
            name: self.name.as_deref().map(valid_bot_name).transpose()?,
            color: self
                .color
                .as_deref()
                .map(valid_bot_color)
                .transpose()?
                .map(str::to_owned),
            shape: self
                .shape
                .as_deref()
                .map(valid_bot_shape)
                .transpose()?
                .map(str::to_owned),
            description: self
                .description
                .as_deref()
                .map(valid_bot_description)
                .transpose()?,
            instructions: self
                .instructions
                .as_deref()
                .map(valid_bot_instructions)
                .transpose()?,
        })
    }

    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.color.is_none()
            && self.shape.is_none()
            && self.description.is_none()
            && self.instructions.is_none()
    }

    /// Les champs qui changeraient vraiment le bot, avec leur valeur actuelle en regard.
    fn against(self, bot: &Bot) -> (Self, Value) {
        let mut before = serde_json::Map::new();
        let mut keep = |key: &str, value: Option<String>, current: &str| {
            let value = value.filter(|value| value != current);
            if value.is_some() {
                before.insert(key.into(), json!(current));
            }
            value
        };
        let changes = Self {
            name: keep("name", self.name, &bot.name),
            color: keep("color", self.color, &bot.color),
            shape: keep("shape", self.shape, &bot.shape),
            description: keep("description", self.description, &bot.description),
            instructions: keep("instructions", self.instructions, &bot.instructions),
        };
        (changes, Value::Object(before))
    }
}

/// Un bot tel que l'interface et la génération le voient.
#[derive(Debug, Clone, Serialize)]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub color: String,
    pub shape: String,
    /// Publique : présentée dans l'interface et donnée au modèle comme contrainte de rôle.
    pub description: String,
    /// Privées : la façon dont le bot se comporte.
    pub instructions: String,
    pub is_default: bool,
    pub template: Option<String>,
    pub customized: bool,
    /// Une pierre tombale : le bot ne démarre plus de conversation, mais les siennes gardent son nom.
    pub deleted: bool,
}

/// Le bot livré avec l'application. Augmenter `version` réécrit les copies que l'utilisateur
/// n'a pas modifiées.
struct BotTemplate {
    id: &'static str,
    version: i64,
    name: &'static str,
    color: &'static str,
    shape: &'static str,
    description: &'static str,
    instructions: &'static str,
}

const DEFAULT_BOT: BotTemplate = BotTemplate {
    id: "grokbot",
    version: 1,
    name: "Ezerbot",
    color: "citron",
    shape: "galet",
    description: "Un petit compagnon curieux et chaleureux, prêt à faire grandir vos idées.",
    instructions: "Tu es chaleureux, curieux et direct. Va à l'essentiel, pose une question quand la demande est ambiguë, et propose un prochain pas concret quand c'est utile.",
};

/// L'identifiant du bot créé par une proposition, dérivé de son `call_id`.
fn proposed_bot_id(proposal: &str) -> String {
    let safe: String = proposal
        .chars()
        .take(100)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    format!("proposition-{safe}")
}

/// Arguments de `propose_bot`, tels que le modèle les émet.
#[derive(Deserialize)]
struct ProposeBot {
    name: String,
    shape: String,
    color: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    reason: String,
}

/// Arguments de `propose_bot_update`.
#[derive(Deserialize)]
struct ProposeBotUpdate {
    bot_id: String,
    changes: BotChanges,
    #[serde(default)]
    reason: String,
}

/// Ce que l'interface montre sous la réponse quand une proposition n'a pas pu être retenue.
fn proposal_notice(detail: &str) -> String {
    format!("Une proposition de bot n'a pas été retenue : {detail}")
}

/// Enregistre un appel de fonction du modèle comme proposition, après l'avoir validé avec les
/// mêmes bornes que l'API. Des arguments invalides ne créent rien : la note explique pourquoi.
/// Le `call_id` sert de clé : un événement rejoué ne crée pas de seconde proposition.
pub(crate) fn record_proposal(
    connection: &Connection,
    conversation: &str,
    message: &str,
    call: &provider::ToolCall,
) -> Result<Option<String>, ApiError> {
    if validate_id(&call.call_id).is_err() {
        return Ok(Some(proposal_notice(
            "son identifiant d'appel est illisible.",
        )));
    }
    let reason = |raw: &str| -> String { raw.trim().chars().take(280).collect() };
    let (kind, bot_id, payload, reason): (&str, Option<String>, Value, String) = match call
        .name
        .as_str()
    {
        "propose_bot" => {
            let Ok(arguments) = serde_json::from_str::<ProposeBot>(&call.arguments) else {
                return Ok(Some(proposal_notice("ses arguments sont illisibles.")));
            };
            let fields = BotChanges {
                name: Some(arguments.name),
                color: Some(arguments.color),
                shape: Some(arguments.shape),
                description: Some(arguments.description),
                instructions: Some(arguments.instructions),
            };
            let fields = match fields.validated() {
                Ok(fields) => fields,
                Err(error) => return Ok(Some(proposal_notice(&error.message))),
            };
            (
                "create",
                None,
                serde_json::to_value(&fields).expect("champs sérialisables"),
                reason(&arguments.reason),
            )
        }
        "propose_bot_update" => {
            let Ok(arguments) = serde_json::from_str::<ProposeBotUpdate>(&call.arguments) else {
                return Ok(Some(proposal_notice("ses arguments sont illisibles.")));
            };
            if validate_id(&arguments.bot_id).is_err() {
                return Ok(Some(proposal_notice("le bot visé est inconnu.")));
            }
            let bot = connection
                .query_row(
                    "SELECT id, name, color, description, instructions, template, customized, deleted, shape, created_at FROM wrapper_bots WHERE account_id = ?1 AND id = ?2",
                    params![ACCOUNT, arguments.bot_id],
                    bot_from_row,
                )
                .optional()?;
            let Some(bot) = bot.filter(|bot| !bot.deleted) else {
                return Ok(Some(proposal_notice("le bot visé est inconnu.")));
            };
            let changes = match arguments.changes.validated() {
                Ok(changes) => changes,
                Err(error) => return Ok(Some(proposal_notice(&error.message))),
            };
            let (changes, before) = changes.against(&bot);
            if changes.is_empty() {
                return Ok(Some(proposal_notice("elle ne changerait rien à ce bot.")));
            }
            (
                "update",
                Some(bot.id),
                json!({"changes": changes, "before": before}),
                reason(&arguments.reason),
            )
        }
        other => {
            return Ok(Some(proposal_notice(&format!(
                "l'outil « {} » n'existe pas.",
                other.chars().take(40).collect::<String>()
            ))));
        }
    };
    let timestamp = now();
    connection.execute(
        "INSERT OR IGNORE INTO wrapper_proposals (account_id, id, conversation_id, message_id, kind, bot_id, payload, reason, status, result_bot_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, ?9, ?9)",
        params![ACCOUNT, call.call_id, conversation, message, kind, bot_id, payload.to_string(), reason, timestamp],
    )?;
    Ok(None)
}

fn proposal_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Proposal> {
    let payload: String = row.get(5)?;
    Ok(Proposal {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        message_id: row.get(2)?,
        kind: row.get(3)?,
        bot_id: row.get(4)?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        reason: row.get(6)?,
        status: row.get(7)?,
        result_bot_id: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

const PROPOSAL_COLUMNS: &str = "id, conversation_id, message_id, kind, bot_id, payload, reason, status, result_bot_id, created_at, updated_at";

/// Migration de nom indépendante du template : personnaliser la couleur ou la personnalité
/// ne doit pas conserver l'ancien nom. Une marque durable laisse ensuite l'utilisateur libre
/// de choisir n'importe quel nom, y compris « Grokbot », sans le réécrire au prochain lancement.
fn migrate_default_bot_name(connection: &Connection) -> Result<(), ApiError> {
    let tx = connection.unchecked_transaction()?;
    tx.execute(
        "UPDATE wrapper_bots SET name = ?3
         WHERE account_id = ?1 AND id = ?2 AND name = 'Grokbot'
         AND NOT EXISTS (SELECT 1 FROM wrapper_migrations WHERE account_id = ?1 AND id = 'ezerbot-name-v1')",
        params![ACCOUNT, DEFAULT_BOT.id, DEFAULT_BOT.name],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO wrapper_migrations (account_id, id) VALUES (?1, 'ezerbot-name-v1')",
        [ACCOUNT],
    )?;
    tx.commit()?;
    Ok(())
}

/// Pose le bot par défaut, ou le met à niveau tant que l'utilisateur ne l'a pas modifié.
fn seed_default_bot(connection: &Connection) -> Result<(), ApiError> {
    connection.execute(
        "INSERT INTO wrapper_bots (account_id, id, name, color, shape, description, instructions, template, template_version, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?9, ?5, ?6, ?2, ?7, ?8, ?8)
         ON CONFLICT(account_id, id) DO UPDATE SET name = excluded.name, description = excluded.description,
             instructions = excluded.instructions, template_version = excluded.template_version, updated_at = excluded.updated_at
         WHERE wrapper_bots.customized = 0 AND COALESCE(wrapper_bots.template_version, 0) < excluded.template_version",
        params![
            ACCOUNT,
            DEFAULT_BOT.id,
            DEFAULT_BOT.name,
            DEFAULT_BOT.color,
            DEFAULT_BOT.description,
            DEFAULT_BOT.instructions,
            DEFAULT_BOT.version,
            now(),
            DEFAULT_BOT.shape
        ],
    )?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversation {
    title: Option<String>,
    bot_id: Option<String>,
    bot_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBot {
    id: String,
    name: String,
    color: String,
    shape: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBot {
    id: String,
    name: Option<String>,
    color: Option<String>,
    shape: Option<String>,
    description: Option<String>,
    instructions: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBot {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptProposal {
    id: String,
    /// « Modifier avant » : les valeurs retenues par l'utilisateur à la place de celles proposées.
    overrides: Option<BotChanges>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DismissProposal {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateConversation {
    id: String,
    title: Option<String>,
    favorite: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteConversation {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    theme: Option<String>,
    accent: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    fast: Option<bool>,
}

const DEFAULT_THEME: &str = "dark";
const DEFAULT_ACCENT: &str = "citron";
const ACCENTS: [&str; 9] = [
    "citron",
    "orange",
    "ambre",
    "menthe",
    "cyan",
    "magenta",
    "vermillon",
    "violet",
    "blanc",
];
const DEFAULT_SHAPE: &str = "galet";
const SHAPES: [&str; 6] = ["galet", "goutte", "nuage", "carre", "fantome", "etoile"];

pub struct App {
    db_path: PathBuf,
    connection: Connection,
    reader: SessionStore,
    session: SubmissionSession,
    provider: Provider,
    #[cfg(debug_assertions)]
    debug_voice: features::voice::DebugVoice,
    // Host-owned: survives a WebView reload, never persisted across app starts.
    deep_link_seen: BTreeMap<String, u64>,
}

impl App {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        Self::open_with(path, Provider::new(path))
    }

    pub fn open_with(path: &Path, provider: Provider) -> Result<Self, ApiError> {
        let account_id = AccountId::new(ACCOUNT).expect("constant account ID");
        let reader = SessionStore::open(path, &account_id)?;
        let generation = reader
            .list_submissions()?
            .iter()
            .map(|item| item.generation.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| ApiError::new(500, "La génération locale est épuisée."))?;
        let session = SubmissionSession::open(
            path,
            AuthenticatedActor {
                account_id,
                team_id: None,
                generation: AccountGeneration(generation),
            },
        )?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(ApiError::new(
                500,
                "Le stockage local n'a pas pu activer le mode WAL.",
            ));
        }
        connection.execute_batch(SCHEMA)?;
        // Colonnes ajoutées après la première version : une ancienne base les reçoit avec leur défaut.
        for (table, column, definition) in [
            (
                "wrapper_conversations",
                "title_customized",
                "INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "wrapper_conversations",
                "bot_id",
                "TEXT NOT NULL DEFAULT 'grokbot'",
            ),
            (
                "wrapper_preferences",
                "accent",
                "TEXT NOT NULL DEFAULT 'citron'",
            ),
            ("wrapper_bots", "shape", "TEXT NOT NULL DEFAULT 'galet'"),
            ("wrapper_preferences", "model", "TEXT"),
            ("wrapper_preferences", "effort", "TEXT"),
            ("wrapper_preferences", "fast", "INTEGER NOT NULL DEFAULT 0"),
            (
                "wrapper_messages",
                "kind",
                "TEXT NOT NULL DEFAULT 'submission'",
            ),
            (
                "wrapper_conversations",
                "kind",
                "TEXT NOT NULL DEFAULT 'chat'",
            ),
            ("wrapper_conversations", "parent_id", "TEXT"),
        ] {
            let present: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
                [table, column],
                |row| row.get(0),
            )?;
            if !present {
                connection.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                    [],
                )?;
            }
        }
        seed_default_bot(&connection)?;
        migrate_default_bot_name(&connection)?;
        features::migrate(&connection)?;
        features::screen::startup(path);
        features::engine::startup(&connection)?;
        if let Err(error) = features::storage::startup(&connection, path, now() as i64) {
            // Keep the app usable; the Stockage panel exposes the actionable error.
            eprintln!("Nettoyage automatique interrompu : {error}");
        }
        Ok(Self {
            db_path: path.to_path_buf(),
            connection,
            reader,
            session,
            provider,
            #[cfg(debug_assertions)]
            debug_voice: features::voice::DebugVoice::default(),
            deep_link_seen: BTreeMap::new(),
        })
    }

    /// Periodic host hook, including while no HTTP request is arriving.
    pub fn tick(&mut self, now_ms: u64) {
        let mut ctx = self.feature_context();
        ctx.now_ms = now_ms;
        features::tick(&ctx);
    }

    #[doc(hidden)]
    pub fn feature_context(&self) -> features::Context<'_> {
        features::Context {
            connection: &self.connection,
            provider: &self.provider,
            db_path: &self.db_path,
            now_ms: now(),
            bot_id: None,
            conversation_id: None,
            run_id: None,
        }
    }

    #[doc(hidden)]
    pub fn context_preview(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<grokbot_engine::contracts::ContextBlock>, ApiError> {
        features::engine::context_preview(self, conversation_id)
    }

    #[doc(hidden)]
    pub fn invoke_tool(
        &self,
        bot_id: &str,
        name: &str,
        args: Value,
    ) -> Result<grokbot_engine::contracts::ToolOutcome, ApiError> {
        validate_id(bot_id)?;
        self.require_bot(bot_id)?;
        let tx = features::write_transaction(&self.connection)?;
        let mut ctx = self.feature_context();
        ctx.connection = &tx;
        ctx.bot_id = Some(bot_id);
        let call_id: String =
            tx.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
        let call = grokbot_engine::contracts::ToolCall {
            call_id,
            name: name.into(),
            arguments: args,
        };
        let outcome = features::call_tool(&ctx, &call)
            .ok_or_else(|| ApiError::new(404, "Cet outil n'existe pas."))?;
        if matches!(
            outcome,
            grokbot_engine::contracts::ToolOutcome::Failed { .. }
        ) {
            tx.rollback()?;
        } else {
            tx.commit()?;
        }
        Ok(outcome)
    }

    fn proposal(&self, id: &str) -> Result<Option<Proposal>, ApiError> {
        let mut proposal = self
            .connection
            .query_row(
                &format!("SELECT {PROPOSAL_COLUMNS} FROM wrapper_proposals WHERE account_id = ?1 AND id = ?2"),
                params![ACCOUNT, id],
                proposal_from_row,
            )
            .optional()?;
        if let Some(proposal) = proposal.as_mut() {
            self.settle_stale(proposal)?;
        }
        Ok(proposal)
    }

    /// Une modification en attente se périme si son bot a disparu, ou si l'un des champs qu'elle
    /// vise a changé depuis la proposition. La comparaison se fait sur `before`, mémorisé avec elle.
    fn settle_stale(&self, proposal: &mut Proposal) -> Result<(), ApiError> {
        if proposal.status != "pending" || proposal.kind != "update" {
            return Ok(());
        }
        let bot = proposal
            .bot_id
            .as_deref()
            .map(|id| self.bot(id))
            .transpose()?
            .flatten()
            .filter(|bot| !bot.deleted);
        let current = bot.map(|bot| {
            json!({
                "name": bot.name, "color": bot.color, "shape": bot.shape,
                "description": bot.description, "instructions": bot.instructions,
            })
        });
        let unchanged = current.as_ref().is_some_and(|current| {
            proposal.payload["before"]
                .as_object()
                .is_some_and(|before| before.iter().all(|(key, value)| current[key] == *value))
        });
        if unchanged {
            return Ok(());
        }
        let timestamp = now();
        self.connection.execute(
            "UPDATE wrapper_proposals SET status = 'stale', updated_at = ?3 WHERE account_id = ?1 AND id = ?2 AND status = 'pending'",
            params![ACCOUNT, proposal.id, timestamp],
        )?;
        proposal.status = "stale".into();
        proposal.updated_at = timestamp;
        Ok(())
    }

    /// Les propositions de chaque réponse, périmées au besoin, indexées par (conversation, message).
    fn proposals_by_message(&self) -> Result<BTreeMap<(String, String), Vec<Proposal>>, ApiError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {PROPOSAL_COLUMNS} FROM wrapper_proposals WHERE account_id = ?1 ORDER BY created_at, id"
        ))?;
        let rows: Vec<Proposal> = statement
            .query_map([ACCOUNT], proposal_from_row)?
            .collect::<Result<_, _>>()?;
        drop(statement);
        let mut grouped: BTreeMap<(String, String), Vec<Proposal>> = BTreeMap::new();
        for mut proposal in rows {
            self.settle_stale(&mut proposal)?;
            grouped
                .entry((
                    proposal.conversation_id.clone(),
                    proposal.message_id.clone(),
                ))
                .or_default()
                .push(proposal);
        }
        Ok(grouped)
    }

    /// Crée un bot avec des valeurs déjà validées. Rejouer la même création rend le même bot.
    fn create_bot(&self, id: &str, fields: &BotChanges) -> Result<Value, ApiError> {
        let name = fields.name.as_deref().ok_or_else(|| {
            ApiError::new(400, "Le nom du bot doit contenir entre 1 et 40 caractères.")
        })?;
        let color = fields
            .color
            .as_deref()
            .ok_or_else(|| ApiError::new(400, "Cette couleur n’existe pas."))?;
        let shape = fields.shape.as_deref().unwrap_or(DEFAULT_SHAPE);
        let description = fields.description.as_deref().unwrap_or_default();
        let instructions = fields.instructions.as_deref().unwrap_or_default();
        if let Some(bot) = self.bot(id)? {
            let same = (
                bot.name.as_str(),
                bot.color.as_str(),
                bot.shape.as_str(),
                bot.description.as_str(),
                bot.instructions.as_str(),
            ) == (name, color, shape, description, instructions);
            if bot.deleted || !same {
                return Err(ApiError::new(
                    409,
                    "Ce bot existe déjà avec un autre contenu.",
                ));
            }
            return Ok(json!({"bot": bot}));
        }
        let timestamp = now();
        self.connection.execute(
            "INSERT INTO wrapper_bots (account_id, id, name, color, shape, description, instructions, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?8, ?5, ?6, ?7, ?7)",
            params![ACCOUNT, id, name, color, description, instructions, timestamp, shape],
        )?;
        self.bot_response(id)
    }

    /// Applique des changements déjà validés à un bot vivant.
    fn update_bot(&self, id: &str, changes: &BotChanges) -> Result<Value, ApiError> {
        if changes.is_empty() {
            return Err(ApiError::new(
                400,
                "Choisis au moins un champ du bot à modifier.",
            ));
        }
        self.require_bot(id)?;
        self.connection.execute(
            "UPDATE wrapper_bots SET name = COALESCE(?3, name), color = COALESCE(?4, color), shape = COALESCE(?8, shape), description = COALESCE(?5, description), instructions = COALESCE(?6, instructions), customized = 1, updated_at = ?7 WHERE account_id = ?1 AND id = ?2",
            params![ACCOUNT, id, changes.name, changes.color, changes.description, changes.instructions, now(), changes.shape],
        )?;
        self.bot_response(id)
    }

    /// `pending → accepted`. Accepter de nouveau rend le même bot ; une proposition écartée ou
    /// périmée répond 409. `overrides` remplace les valeurs proposées (« Modifier avant »).
    fn accept_proposal(&self, id: &str, overrides: Option<BotChanges>) -> Result<Value, ApiError> {
        let proposal = self
            .proposal(id)?
            .ok_or_else(|| ApiError::new(404, "Proposition introuvable."))?;
        let conversation = proposal.conversation_id.clone();
        match proposal.status.as_str() {
            "accepted" => {
                let bot = proposal
                    .result_bot_id
                    .as_deref()
                    .map(|id| self.bot_response(id))
                    .transpose()?
                    .map_or(Value::Null, |response| response["bot"].clone());
                return Ok(
                    json!({"conversation": self.conversation_value(&conversation)?, "bot": bot}),
                );
            }
            "dismissed" => {
                return Err(ApiError::new(409, "Cette proposition a été écartée."));
            }
            "stale" => {
                return Err(ApiError::new(409, "Ce bot a changé depuis la proposition."));
            }
            _ => {}
        }
        let overrides = overrides.map(BotChanges::validated).transpose()?;
        let (bot_id, response) = if proposal.kind == "update" {
            let target = proposal
                .bot_id
                .clone()
                .ok_or_else(|| ApiError::new(500, "La proposition ne vise aucun bot."))?;
            let proposed: BotChanges = serde_json::from_value(proposal.payload["changes"].clone())
                .map_err(|_| ApiError::new(500, "La proposition est illisible."))?;
            let changes = overrides.unwrap_or(proposed);
            let response = self.update_bot(&target, &changes)?;
            (target, response)
        } else {
            let proposed: BotChanges = serde_json::from_value(proposal.payload.clone())
                .map_err(|_| ApiError::new(500, "La proposition est illisible."))?;
            let overrides = overrides.unwrap_or_default();
            let fields = BotChanges {
                name: overrides.name.or(proposed.name),
                color: overrides.color.or(proposed.color),
                shape: overrides.shape.or(proposed.shape),
                description: overrides.description.or(proposed.description),
                instructions: overrides.instructions.or(proposed.instructions),
            };
            let bot_id = proposed_bot_id(&proposal.id);
            let response = self.create_bot(&bot_id, &fields)?;
            (bot_id, response)
        };
        self.connection.execute(
            "UPDATE wrapper_proposals SET status = 'accepted', result_bot_id = ?3, updated_at = ?4 WHERE account_id = ?1 AND id = ?2",
            params![ACCOUNT, proposal.id, bot_id, now()],
        )?;
        Ok(json!({"conversation": self.conversation_value(&conversation)?, "bot": response["bot"]}))
    }

    /// `pending → dismissed`. Écarter de nouveau ne fait rien ; une proposition acceptée répond 409.
    fn dismiss_proposal(&self, id: &str) -> Result<Value, ApiError> {
        let proposal = self
            .proposal(id)?
            .ok_or_else(|| ApiError::new(404, "Proposition introuvable."))?;
        let conversation = proposal.conversation_id.clone();
        if proposal.status == "accepted" {
            return Err(ApiError::new(409, "Cette proposition a déjà été acceptée."));
        }
        self.connection.execute(
            "UPDATE wrapper_proposals SET status = 'dismissed', updated_at = ?3 WHERE account_id = ?1 AND id = ?2 AND status = 'pending'",
            params![ACCOUNT, proposal.id, now()],
        )?;
        Ok(json!({"conversation": self.conversation_value(&conversation)?}))
    }

    /// La conversation d'une proposition, même supprimée : la réponse reste cohérente.
    fn conversation_value(&self, id: &str) -> Result<Value, ApiError> {
        Ok(self
            .conversations()?
            .into_iter()
            .find(|conversation| conversation.id == id)
            .map_or(Value::Null, |conversation| json!(conversation)))
    }

    fn preferences(&self) -> Result<(String, String), ApiError> {
        Ok(self
            .connection
            .query_row(
                "SELECT theme, accent FROM wrapper_preferences WHERE account_id = ?1",
                [ACCOUNT],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or_else(|| (DEFAULT_THEME.into(), DEFAULT_ACCENT.into())))
    }

    /// HTTP dispatcher preserves binary feature responses without JSON encoding.
    pub fn api_response(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<ApiResponse, ApiError> {
        if body.len() > body_limit(path) {
            return Err(too_large(path));
        }
        deep_links::route(self, method, path, body)
            .or_else(|| features::routes(self, method, path, body))
            .unwrap_or_else(|| Err(ApiError::new(404, "Cette adresse n'existe pas.")))
    }

    /// JSON-only compatibility entry point; binary consumers use `api_response`.
    pub fn api(&mut self, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
        match self.api_response(method, path, body)? {
            ApiResponse::Json(value) => Ok(value),
            ApiResponse::Bytes { .. } => Err(ApiError::new(
                406,
                "Cette route renvoie des octets, pas du JSON.",
            )),
        }
    }

    fn require_conversation(&self, id: &str) -> Result<(), ApiError> {
        let deleted = self
            .connection
            .query_row(
                "SELECT deleted FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, id],
                |r| r.get::<_, bool>(0),
            )
            .optional()?;
        match deleted {
            Some(false) => Ok(()),
            Some(true) => Err(ApiError::new(409, "Cette conversation a été supprimée.")),
            None => Err(ApiError::new(404, "Conversation introuvable.")),
        }
    }

    /// Un bot supprimé reste lisible pour ses conversations.
    fn bot(&self, id: &str) -> Result<Option<Bot>, ApiError> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, name, color, description, instructions, template, customized, deleted, shape, created_at FROM wrapper_bots WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, id],
                bot_from_row,
            )
            .optional()?)
    }

    fn bots(&self) -> Result<Vec<Bot>, ApiError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, color, description, instructions, template, customized, deleted, shape, created_at FROM wrapper_bots WHERE account_id = ?1 ORDER BY id = ?2 DESC, created_at, id",
        )?;
        let bots = statement
            .query_map(params![ACCOUNT, DEFAULT_BOT.id], bot_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(bots)
    }

    fn require_bot(&self, id: &str) -> Result<(), ApiError> {
        match self.bot(id)? {
            Some(bot) if !bot.deleted => Ok(()),
            Some(_) => Err(ApiError::new(409, "Ce bot a été supprimé.")),
            None => Err(ApiError::new(404, "Bot introuvable.")),
        }
    }

    fn bot_response(&self, id: &str) -> Result<Value, ApiError> {
        let bot = self
            .bot(id)?
            .ok_or_else(|| ApiError::new(404, "Bot introuvable."))?;
        Ok(json!({"bot": bot}))
    }

    fn touch(&self, id: &str) -> Result<(), ApiError> {
        self.connection.execute("UPDATE wrapper_conversations SET updated_at = MAX(updated_at, ?3) WHERE account_id = ?1 AND id = ?2 AND deleted = 0", params![ACCOUNT, id, now()])?;
        Ok(())
    }

    fn conversation_response(&self, id: &str) -> Result<Value, ApiError> {
        let conversation = self
            .conversations()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| ApiError::new(404, "Conversation introuvable."))?;
        Ok(json!({"conversation": conversation}))
    }
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::new(400, "La requête JSON est invalide."))
}

fn valid_title(title: &str) -> Result<String, ApiError> {
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err(ApiError::new(
            400,
            "Le titre doit contenir entre 1 et 80 caractères.",
        ));
    }
    Ok(title.into())
}

fn bot_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Bot> {
    let id: String = row.get(0)?;
    Ok(Bot {
        is_default: id == DEFAULT_BOT.id,
        id,
        name: row.get(1)?,
        color: row.get(2)?,
        description: row.get(3)?,
        instructions: row.get(4)?,
        template: row.get(5)?,
        customized: row.get(6)?,
        deleted: row.get(7)?,
        shape: row.get(8)?,
        created_at: row.get(9)?,
    })
}

fn valid_bot_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(ApiError::new(
            400,
            "Le nom du bot doit contenir entre 1 et 40 caractères.",
        ));
    }
    Ok(name.into())
}

fn valid_bot_color(color: &str) -> Result<&'static str, ApiError> {
    ACCENTS
        .iter()
        .find(|accent| **accent == color)
        .copied()
        .ok_or_else(|| ApiError::new(400, "Cette couleur n’existe pas."))
}

fn valid_bot_shape(shape: &str) -> Result<&'static str, ApiError> {
    SHAPES
        .iter()
        .find(|known| **known == shape)
        .copied()
        .ok_or_else(|| ApiError::new(400, "Cette forme n’existe pas."))
}

fn valid_bot_description(description: &str) -> Result<String, ApiError> {
    let description = description.trim();
    if description.chars().count() > 280 {
        return Err(ApiError::new(
            400,
            "La description du bot ne doit pas dépasser 280 caractères.",
        ));
    }
    Ok(description.into())
}

fn valid_bot_instructions(instructions: &str) -> Result<String, ApiError> {
    let instructions = instructions.trim();
    if instructions.chars().count() > 8_000 {
        return Err(ApiError::new(
            400,
            "Les instructions du bot ne doivent pas dépasser 8 000 caractères.",
        ));
    }
    Ok(instructions.into())
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    let atom = |value: &str| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    };
    if !atom(id) {
        return Err(ApiError::new(400, "Identifiant invalide."));
    }
    Ok(())
}

fn validate_conversation_id(id: &str) -> Result<(), ApiError> {
    let invalid = || ApiError::new(400, "Identifiant invalide.");
    // DL-S01's task namespace is not a license for arbitrary colon-bearing
    // identifiers. Each component keeps the original ASCII atom restriction;
    // only task:<id>:<parent> may nest, to Q3's maximum delegation depth of 3.
    // The total bound also covers nested task conversations longer than 128.
    if id.len() > 512 {
        return Err(invalid());
    }
    let mut parent = id;
    let mut depth = 0;
    while let Some(task) = parent.strip_prefix("task:") {
        depth += 1;
        if depth > 3 {
            return Err(invalid());
        }
        let (child, rest) = task.split_once(':').ok_or_else(invalid)?;
        validate_id(child)?;
        parent = rest;
    }
    validate_id(parent)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn state_name(state: &SubmissionState) -> &'static str {
    match state {
        SubmissionState::Prepared | SubmissionState::Queued => "queued",
        SubmissionState::Dispatching { .. } => "dispatching",
        SubmissionState::OutcomeUnknown { .. } => "unknown",
        SubmissionState::AcceptedAwaitingEcho { .. } => "accepted",
        SubmissionState::Echoed { .. } => "echoed",
        SubmissionState::Refused { .. } => "refused",
    }
}

/// Compatibility validator for JSON-only callers. The host uses the route-aware variant.
pub fn validate_headers(
    port: u16,
    method: &str,
    headers: &[(String, String)],
) -> Result<(), ApiError> {
    validate_headers_for_path(port, method, "", headers)
}

pub fn validate_headers_for_path(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(String, String)],
) -> Result<(), ApiError> {
    let reject = || {
        ApiError::new(
            403,
            "Cette requête ne provient pas de l'application locale.",
        )
    };
    let one = |name: &str| -> Result<Option<&str>, ApiError> {
        let mut matching = headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name));
        let value = matching.next().map(|(_, value)| value.as_str());
        if matching.next().is_some() {
            return Err(reject());
        }
        Ok(value)
    };
    let host = one("host")?.ok_or_else(reject)?;
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Err(reject());
    }
    if method != "GET" && method != "HEAD" {
        if let Some(origin) = one("origin")? {
            if origin != format!("http://{host}") {
                return Err(reject());
            }
        }
        if one("x-grokbot-request")? != Some("local") {
            return Err(reject());
        }
        let content_type = one("content-type")?.ok_or_else(reject)?;
        let mime = content_type.split(';').next().unwrap_or("").trim();
        let image = mime.split_once('/').is_some_and(|(kind, subtype)| {
            kind.eq_ignore_ascii_case("image")
                && !subtype.is_empty()
                && subtype
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b'.'))
        });
        if !mime.eq_ignore_ascii_case("application/json")
            && !(attachment_upload_path(path) && image)
        {
            return Err(reject());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
