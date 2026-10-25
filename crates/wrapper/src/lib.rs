//! Adaptation locale du moteur Rust. Les états et les corps proviennent du journal durable.

#![forbid(unsafe_code)]

pub mod deep_links;
mod fluidity;
mod mcp;
mod projection_hints;
mod provider;
mod upstream;

/// Model gateway implementations are added independently of the HTTP host.
#[doc(hidden)]
pub mod gateway;

/// Capability hooks, shared by the HTTP app and future run-thread adapters.
#[doc(hidden)]
pub mod features;

/// Mise à jour locale depuis `main`. Le dossier des conversations n’est pas le dépôt de travail.
pub mod updates;

// The model gateway keeps the root compatibility name while generation belongs
// to its feature module (including the offline simulated reply).
pub(crate) use features::engine::demo_reply;

pub use provider::{database_path_from_home, default_database_path, Endpoints, Provider, ToolCall};

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
/// A template document is 4 MiB. Create and decode wrap that JSON, and decode
/// accepts text up to twice the document size.
pub const TEMPLATE_BODY_LIMIT: usize = 8 * 1024 * 1024;

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
        "/api/documents" | "/api/library" => ATTACHMENT_BODY_LIMIT.div_ceil(3) * 4 + 4096,
        "/api/voice/memos" => features::voice::MAX_WAV_BYTES.div_ceil(3) * 4 + 128 * 1024,
        "/api/meetings/finalize" => features::meetings::MAX_WAV_BYTES.div_ceil(3) * 4 + 256 * 1024,
        "/api/skills" | "/api/skills/update" | "/api/skills/import" => SKILL_IMPORT_BODY_LIMIT,
        // 2 × 20 rules × 1,000 Unicode characters, including JSON escapes.
        "/api/approvals/reviewer" => 256 * 1024,
        "/api/templates/create" | "/api/templates/decode" => TEMPLATE_BODY_LIMIT,
        _ => BODY_LIMIT,
    }
}

/// Shared size error for direct API calls, declared lengths and chunked bodies.
pub fn too_large(path: &str) -> ApiError {
    if path == "/api/documents" || path == "/api/library" {
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
CREATE TABLE IF NOT EXISTS wrapper_reactions (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    emoji TEXT NOT NULL,
    PRIMARY KEY (account_id, conversation_id, message_id)
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
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<GroupAvatar>,
}

/// Pastille d’un groupe : une couleur d’accent et un signe court.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupAvatar {
    pub color: String,
    pub mark: String,
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
    description: Option<String>,
    avatar: Option<GroupAvatar>,
    topic_id: Option<String>,
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
struct UpdateConversation {
    id: String,
    title: Option<String>,
    favorite: Option<bool>,
    description: Option<String>,
    avatar: Option<GroupAvatar>,
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

impl Drop for App {
    fn drop(&mut self) {
        // Fermer l’app avec l’agent désactivé est une absence voulue des routines.
        features::routines::depart(&self.feature_context());
        // Quitter l’app tue les groupes encore suivis. Le redémarrage ne les relance pas.
        // Les fils de fond (tour du moteur, sonde de connexion, suivis, moniteurs des tâches
        // shell) ne doivent plus rien écrire dans le dossier de l'application une fois celle-ci
        // détruite (#463). Seules les versions de développement et de test retiennent leurs
        // fils : la version livrée n'attend rien, l'arrêt de l'application ne change pas.
        // L'attente passe avant l'arrêt des tâches shell : un tour encore en vol peut lancer une
        // tâche pendant la vidange, et son moniteur est alors arrêté et joint comme les autres.
        // Les deux échecs sont explicites : l'attente bornée, puis les tâches non finies.
        #[cfg(any(test, debug_assertions, feature = "test-support"))]
        let waited = self.provider.wait_for_background_work();
        let stopped = features::local::shutdown(&self.connection, &self.db_path);
        #[cfg(any(test, debug_assertions, feature = "test-support"))]
        let stopped = waited.and(stopped);
        if let Err(message) = stopped {
            if cfg!(any(test, debug_assertions, feature = "test-support"))
                && !std::thread::panicking()
            {
                panic!("{message}");
            }
            eprintln!("{message}");
        }
    }
}

pub struct App {
    db_path: PathBuf,
    connection: Connection,
    reader: SessionStore,
    session: SubmissionSession,
    provider: Provider,
    #[cfg(debug_assertions)]
    debug_voice: features::voice::DebugVoice,
    #[cfg(debug_assertions)]
    debug_voice_session: features::voice_session::DebugSession,
    #[cfg(debug_assertions)]
    debug_voice_call: features::voice_call::DebugCall,
    // Host-owned: survives a WebView reload, never persisted across app starts.
    deep_link_seen: BTreeMap<String, u64>,
    // Display fluidity (#255), per polling page; `state_client` is the page of
    // the `/api/state` being served, set by the HTTP host.
    stream_watches: BTreeMap<&'static str, fluidity::StreamWatch>,
    state_client: &'static str,
}

impl App {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        Self::open_with(path, Provider::new(path))
    }

    pub fn open_with(path: &Path, provider: Provider) -> Result<Self, ApiError> {
        let account_id = AccountId::new(ACCOUNT).expect("constant account ID");
        let reader = SessionStore::open(path, &account_id)?;
        let generation = reader
            .latest_generation()?
            .map_or(0, |generation| generation.0)
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
        // A state read repeats about twenty queries for every message of the history
        // (#377): all of them must stay prepared, the default 16 would evict each in turn.
        connection.set_prepared_statement_cache_capacity(64);
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
            (
                "wrapper_conversations",
                "group_description",
                "TEXT NOT NULL DEFAULT ''",
            ),
            ("wrapper_conversations", "group_avatar", "TEXT"),
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
        features::local::startup(&connection)?;
        features::screen::startup(path);
        features::demonstrations::startup(&connection, path)?;
        features::engine::startup(&connection)?;
        features::screen::reconcile(&connection, path)?;
        if let Err(error) = features::storage::startup(&connection, path, now() as i64) {
            // Keep the app usable; the Stockage panel exposes the actionable error.
            eprintln!("Nettoyage automatique interrompu : {error}");
        }
        features::updates::remember_database(path);
        Ok(Self {
            db_path: path.to_path_buf(),
            connection,
            reader,
            session,
            provider,
            #[cfg(debug_assertions)]
            debug_voice: features::voice::DebugVoice::default(),
            #[cfg(debug_assertions)]
            debug_voice_session: features::voice_session::DebugSession::default(),
            #[cfg(debug_assertions)]
            debug_voice_call: features::voice_call::DebugCall::default(),
            deep_link_seen: BTreeMap::new(),
            stream_watches: BTreeMap::new(),
            state_client: "principal",
        })
    }

    /// Periodic host hook, including while no HTTP request is arriving.
    pub fn tick(&mut self, now_ms: u64) {
        let mut ctx = self.feature_context();
        ctx.now_ms = now_ms;
        features::tick(&ctx);
    }

    /// Deliver an approval wake without launching queued background tasks.
    #[doc(hidden)]
    pub fn tick_suspended(&mut self, now_ms: u64) {
        let mut ctx = self.feature_context();
        ctx.now_ms = now_ms;
        features::engine::tick_suspended(&ctx);
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
        self.invoke_tool_in(bot_id, None, name, args)
    }

    /// Comme `invoke_tool`, mais depuis un fil : les outils qui dépendent de la conversation
    /// (mémoire du sujet) y trouvent leur contexte.
    #[doc(hidden)]
    pub fn invoke_tool_in(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        name: &str,
        args: Value,
    ) -> Result<grokbot_engine::contracts::ToolOutcome, ApiError> {
        validate_id(bot_id)?;
        self.require_bot(bot_id)?;
        let tx = features::write_transaction(&self.connection)?;
        let mut ctx = self.feature_context();
        ctx.connection = &tx;
        ctx.bot_id = Some(bot_id);
        ctx.conversation_id = conversation_id;
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
        // A bot with no voice of its own takes one that no other bot uses. The bot exists
        // either way: a failure here only leaves the voice for the next attribution.
        let _ = features::voice_session::assign_missing(&self.connection);
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
        let response = deep_links::route(self, method, path, body)
            .or_else(|| features::routes(self, method, path, body))
            .unwrap_or_else(|| Err(ApiError::new(404, "Cette adresse n'existe pas.")))?;
        match response {
            ApiResponse::Json(value) if path == "/api/state" => Ok(ApiResponse::Json(
                features::disclosure::state(&self.connection, &value)?,
            )),
            other => Ok(other),
        }
    }

    /// JSON-only compatibility entry point; binary consumers use `api_response`.
    pub fn api(&mut self, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
        let started = std::time::Instant::now();
        match self.api_response(method, path, body)? {
            ApiResponse::Json(value) => {
                // In process, nothing is encoded: the HTTP host times its own encoding.
                if (method, path) == ("GET", "/api/state") {
                    self.log_state_served(started.elapsed());
                }
                Ok(value)
            }
            ApiResponse::Bytes { .. } => Err(ApiError::new(
                406,
                "Cette route renvoie des octets, pas du JSON.",
            )),
        }
    }

    /// Display fluidity (#255): the page the next `/api/state` is served to.
    pub fn set_state_client(&mut self, remote: bool, header: Option<&str>) {
        self.state_client = fluidity::client(remote, header);
    }

    /// Display fluidity (#255): the duration of the last served `/api/state`.
    pub fn log_state_served(&mut self, elapsed: Duration) {
        let directory = self.db_path.parent().unwrap_or(Path::new("."));
        let client = self.state_client;
        self.stream_watches.entry(client).or_default().served(
            directory,
            client,
            elapsed,
            std::time::Instant::now(),
        );
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
            .conversation(id)?
            .ok_or_else(|| ApiError::new(404, "Conversation introuvable."))?;
        features::disclosure::state(&self.connection, &json!({"conversation": conversation}))
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

fn valid_group_description(description: &str) -> Result<String, ApiError> {
    let description = description.trim();
    let forbidden = description
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t');
    if description.is_empty() || description.chars().count() > 280 || forbidden {
        return Err(ApiError::new(
            400,
            "La description du groupe doit contenir entre 1 et 280 caractères.",
        ));
    }
    Ok(description.into())
}

fn valid_group_avatar(avatar: &GroupAvatar) -> Result<GroupAvatar, ApiError> {
    let color = valid_bot_color(&avatar.color)?.to_owned();
    let mark = avatar.mark.trim();
    let allowed = (1..=4).contains(&mark.chars().count())
        && !mark.chars().any(|c| c.is_whitespace() || c.is_control());
    if !allowed {
        return Err(ApiError::new(
            400,
            "Le signe de l’avatar contient de un à quatre caractères, sans espace.",
        ));
    }
    Ok(GroupAvatar {
        color,
        mark: mark.into(),
    })
}

fn stored_group_avatar(raw: Option<String>) -> Option<GroupAvatar> {
    raw.and_then(|value| serde_json::from_str(&value).ok())
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
