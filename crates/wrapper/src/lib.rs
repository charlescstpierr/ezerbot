//! Adaptation locale du moteur Rust. Les états et les corps proviennent du journal durable.

#![forbid(unsafe_code)]

mod provider;

/// Model gateway implementations are added independently of the HTTP host.
#[doc(hidden)]
pub mod gateway;

pub use provider::{Endpoints, Provider, ToolCall};

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grokbot_domain::{
    AccountGeneration, AccountId, AuthenticatedActor, ConversationId, DeliveryReceipt, MessageId,
    Refusal, SubmissionState, TranscriptEntryId,
};
use grokbot_session::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SessionError,
    SubmissionSession, SubmissionTransport,
};
use grokbot_store::SessionStore;
use grokbot_submission_journal::JournalError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const BODY_LIMIT: usize = 64 * 1024;
pub const ATTACHMENT_BODY_LIMIT: usize = 20 * 1024 * 1024;
pub const SKILL_IMPORT_BODY_LIMIT: usize = 16 * 1024 * 1024;

/// Only the exact upload routes receive a larger budget; other routes stay bounded JSON.
pub fn body_limit(path: &str) -> usize {
    match path {
        "/api/attachments" => ATTACHMENT_BODY_LIMIT,
        "/api/skills/import" => SKILL_IMPORT_BODY_LIMIT,
        _ => BODY_LIMIT,
    }
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
const DEMO_REPLY: &str = "Bien reçu. Ton message est conservé dans cette conversation. Le mode local simule les réponses ; aucun service d'intelligence artificielle n'est connecté.";

const SCHEMA: &str = "
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

const DEMO_DIAGRAM: &str = "Voici un diagramme d'exemple. Le mode local simule les réponses.\n\n```mermaid\nflowchart LR\n  A[Votre message] --> B{Grokbot}\n  B -->|texte| C[Réponse]\n  B -->|schéma| D[Diagramme]\n  C & D --> E[(Historique local)]\n```\n\nPassez la souris sur le diagramme pour le copier, le télécharger ou l'agrandir.";
const DEMO_CODE: &str = "Voici un exemple de code. Le mode local simule les réponses.\n\n```rust\nfn saluer(nom: &str) -> String {\n    format!(\"Bonjour, {nom} !\")\n}\n\nfn main() {\n    println!(\"{}\", saluer(\"Grokbot\"));\n}\n```\n\nLe bouton en haut du bloc copie le code.";
const DEMO_TABLE: &str = "Voici un tableau d'exemple. Le mode local simule les réponses.\n\n| Élément | Rôle | État |\n| --- | --- | --- |\n| Moteur Rust | Garde l'historique | Actif |\n| Interface | Affiche la conversation | Active |\n| Service d'IA | Répond aux messages | **Non connecté** |\n\nLa formule $E = mc^2$ s'affiche aussi.";

/// The simulated engine has no model. A few keywords pick a canned reply that shows the
/// rich rendering (diagram, code, table); everything else gets the plain acknowledgement.
fn demo_reply(body: &str) -> &'static str {
    let body = body.to_lowercase();
    let mentions = |words: &[&str]| words.iter().any(|word| body.contains(word));
    if mentions(&["diagramme", "schéma", "schema", "mermaid"]) {
        DEMO_DIAGRAM
    } else if mentions(&["code", "rust", "fonction"]) {
        DEMO_CODE
    } else if mentions(&["tableau", "formule"]) {
        DEMO_TABLE
    } else {
        DEMO_REPLY
    }
}

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
    pub title: String,
    pub favorite: bool,
    pub updated_at: u64,
    pub messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
pub struct Message {
    pub id: String,
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
    name: "Grokbot",
    color: "citron",
    shape: "galet",
    description: "Un petit compagnon curieux et chaleureux, prêt à faire grandir vos idées.",
    instructions: "Tu es chaleureux, curieux et direct. Va à l'essentiel, pose une question quand la demande est ambiguë, et propose un prochain pas concret quand c'est utile.",
};

/// Le contrat d'affichage de l'application, commun à tous les bots.
const BASE_INSTRUCTIONS: &str = "Tu es un assistant dans une application de bureau. Réponds dans la langue de l'utilisateur. Tu peux utiliser le Markdown, des blocs de code avec le nom du langage, des diagrammes dans des blocs ```mermaid et des formules LaTeX entre $...$ ou $$...$$.";

/// Ce que le bot par défaut sait de ses outils de proposition. Les autres bots n'en ont pas.
const PROPOSAL_INSTRUCTIONS: &str = "Tu peux proposer à l'utilisateur de créer un nouveau bot avec l'outil propose_bot, ou de modifier un bot existant avec propose_bot_update, quand il le demande ou que c'est clairement utile. Remplis une vraie personnalité : un nom court, une présentation d'une ou deux phrases et des consignes de comportement précises. Rien ne change sans sa confirmation : il verra ta proposition sous ta réponse et choisira. Pour une modification, bot_id est l'identifiant exact d'un bot de la liste.";

/// `bots` sert au bot par défaut, qui doit connaître les identifiants existants pour en proposer
/// une modification. Les bots supprimés n'y figurent pas.
fn compose_instructions(bot: &Bot, bots: &[Bot]) -> String {
    let mut parts = vec![
        BASE_INSTRUCTIONS.to_owned(),
        format!("Tu t'appelles {}.", bot.name),
    ];
    if !bot.description.is_empty() {
        parts.push(format!(
            "Ton rôle, tel que les autres le voient : {}",
            bot.description
        ));
    }
    if !bot.instructions.is_empty() {
        parts.push(format!(
            "Consignes privées sur ta façon de te comporter :\n{}",
            bot.instructions
        ));
    }
    if bot.is_default && PROPOSALS_ENABLED {
        let roster: Vec<String> = bots
            .iter()
            .filter(|known| !known.deleted)
            .map(|known| {
                if known.description.is_empty() {
                    format!("- {} : {}", known.id, known.name)
                } else {
                    format!("- {} : {} — {}", known.id, known.name, known.description)
                }
            })
            .collect();
        parts.push(format!(
            "{PROPOSAL_INSTRUCTIONS}\n\nBots existants (identifiant : nom — présentation) :\n{}",
            roster.join("\n")
        ));
    }
    parts.join("\n\n")
}

/// Les deux outils de proposition, en mode strict : chaque objet ferme `additionalProperties`,
/// tous les champs sont requis, et les champs facultatifs acceptent `null`.
fn proposal_tools() -> Vec<Value> {
    let text = |description: &str| json!({"type": "string", "description": description});
    let choice = |values: &[&str], description: &str| json!({"type": "string", "enum": values, "description": description});
    let optional = |schema: Value| json!({"anyOf": [schema, {"type": "null"}]});
    let fields = [
        ("name", text("Nom court du bot, 40 caractères au plus.")),
        ("shape", choice(&SHAPES, "Silhouette de la mascotte.")),
        (
            "color",
            choice(&ACCENTS, "Couleur des yeux de la mascotte."),
        ),
        (
            "description",
            text("Présentation publique, une ou deux phrases, 280 caractères au plus."),
        ),
        (
            "instructions",
            text("Personnalité privée : ton, manières, limites. 8 000 caractères au plus."),
        ),
    ];
    let required: Vec<&str> = fields.iter().map(|(name, _)| *name).collect();
    let object = |properties: serde_json::Map<String, Value>, required: Vec<&str>| json!({"type": "object", "additionalProperties": false, "properties": properties, "required": required});
    let mut create: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(name, schema)| ((*name).to_owned(), schema.clone()))
        .collect();
    create.insert(
        "reason".into(),
        text("Pourquoi ce bot serait utile, en une phrase."),
    );
    let changes: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(name, schema)| ((*name).to_owned(), optional(schema.clone())))
        .collect();
    let mut update = serde_json::Map::new();
    update.insert(
        "bot_id".into(),
        text("Identifiant exact d'un bot existant."),
    );
    update.insert("changes".into(), object(changes, required.clone()));
    update.insert(
        "reason".into(),
        text("Pourquoi cette modification serait utile, en une phrase."),
    );
    vec![
        json!({
            "type": "function", "name": "propose_bot", "strict": true,
            "description": "Propose à l'utilisateur de créer un nouveau bot. Rien n'est créé sans sa confirmation.",
            "parameters": object(create, [required.clone(), vec!["reason"]].concat()),
        }),
        json!({
            "type": "function", "name": "propose_bot_update", "strict": true,
            "description": "Propose à l'utilisateur de modifier un bot existant. Les champs à laisser tels quels sont null. Rien ne change sans sa confirmation.",
            "parameters": object(update, vec!["bot_id", "changes", "reason"]),
        }),
    ]
}

/// Seul le bot par défaut peut proposer des bots.
/// Chantier suspendu : le fil n'affiche pas encore les cartes de proposition. Tant que c'est
/// faux, Grokbot ne reçoit ni les outils ni la consigne qui les décrit.
const PROPOSALS_ENABLED: bool = false;

fn tools_for(bot: &Bot) -> Vec<Value> {
    if bot.is_default && PROPOSALS_ENABLED {
        proposal_tools()
    } else {
        Vec::new()
    }
}

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
                    "SELECT id, name, color, description, instructions, template, customized, deleted, shape FROM wrapper_bots WHERE account_id = ?1 AND id = ?2",
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

/// Une ligne de l'historique envoyée au modèle, pour qu'il sache ce qu'il est advenu de sa
/// proposition sans rien stocker chez OpenAI.
fn proposal_transcript(proposal: &Proposal) -> String {
    let status = match proposal.status.as_str() {
        "accepted" => "acceptée",
        "dismissed" => "refusée",
        "stale" => "périmée",
        _ => "en attente",
    };
    if proposal.kind == "update" {
        let target = proposal.bot_id.as_deref().unwrap_or("?");
        format!("[Proposition de modification du bot « {target} » : {status}]")
    } else {
        let name = proposal.payload["name"].as_str().unwrap_or("?");
        format!("[Proposition de bot « {name} » : {status}]")
    }
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
struct SendMessage {
    conversation_id: String,
    message_id: String,
    body: String,
    #[serde(default)]
    simulate_loss: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconcileMessage {
    conversation_id: String,
    message_id: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

/// État, erreur, type d'erreur et note d'une génération.
type GenerationRow = (String, Option<String>, Option<String>, Option<String>);

/// Modèle, effort et palier de vitesse réellement envoyés.
type ResolvedModel = (String, Option<String>, Option<String>);

const INTERRUPTED: &str = "La réponse a été interrompue avant la fin. Vous pouvez la relancer.";

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
    connection: Connection,
    reader: SessionStore,
    session: SubmissionSession,
    provider: Provider,
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
        // Aucun fil d'exécution ne survit au processus : une génération restée ouverte est perdue.
        connection.execute(
            "UPDATE wrapper_generation SET status = 'failed', error = ?1, error_kind = 'other', updated_at = ?2 WHERE status = 'streaming'",
            params![INTERRUPTED, now()],
        )?;
        Ok(Self {
            connection,
            reader,
            session,
            provider,
        })
    }

    /// Periodic host hook, including while no HTTP request is arriving.
    /// Feature dispatch will attach scheduler and run wake delivery here.
    pub fn tick(&mut self, _now_ms: u64) {}

    fn model_preferences(&self) -> Result<(Option<String>, Option<String>, bool), ApiError> {
        Ok(self
            .connection
            .query_row(
                "SELECT model, effort, fast FROM wrapper_preferences WHERE account_id = ?1",
                [ACCOUNT],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .unwrap_or((None, None, false)))
    }

    /// Le modèle et l'effort réellement envoyés : la préférence si le catalogue du compte la
    /// contient, sinon le premier modèle et son réglage par défaut.
    fn resolved_model(&self) -> Result<Option<ResolvedModel>, ApiError> {
        let (model, effort, fast) = self.model_preferences()?;
        let models = self.provider.models();
        let Some(chosen) = models
            .iter()
            .find(|candidate| Some(&candidate.slug) == model.as_ref())
            .or_else(|| models.first())
        else {
            return Ok(None);
        };
        let effort = effort
            .filter(|effort| chosen.efforts.iter().any(|level| level.effort == *effort))
            .or_else(|| chosen.default_effort.clone());
        // « Fast » n'existe que pour un modèle qui annonce un palier de vitesse.
        let tier = chosen
            .tiers
            .first()
            .filter(|_| fast)
            .map(|tier| tier.id.clone());
        Ok(Some((chosen.slug.clone(), effort, tier)))
    }

    fn history(
        &self,
        conversation: &str,
        message: &str,
    ) -> Result<Vec<(&'static str, String)>, ApiError> {
        let mut history = Vec::new();
        let conversations = self.conversations()?;
        let Some(found) = conversations
            .iter()
            .find(|candidate| candidate.id == conversation)
        else {
            return Ok(history);
        };
        for item in &found.messages {
            if item.role == "user" {
                if matches!(item.state, "accepted" | "echoed") {
                    history.push(("user", item.body.clone()));
                }
                if item.id == message {
                    break;
                }
            } else if item.state != "failed" {
                // Une proposition est rejouée en texte court : le modèle sait ce qu'il en est advenu.
                let mut lines: Vec<String> = Vec::new();
                if !item.body.trim().is_empty() {
                    lines.push(item.body.clone());
                }
                lines.extend(item.proposals.iter().map(proposal_transcript));
                if !lines.is_empty() {
                    history.push(("assistant", lines.join("\n\n")));
                }
            }
        }
        Ok(history)
    }

    /// Démarre la génération d'une réponse marquée `streaming` qu'aucun fil ne traite encore.
    fn start_generation(&self, conversation: &str, message: &str) -> Result<(), ApiError> {
        let streaming: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM wrapper_generation WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status = 'streaming')",
            params![ACCOUNT, conversation, message], |row| row.get(0),
        )?;
        if !streaming || self.provider.running(conversation, message) {
            return Ok(());
        }
        let Some((model, effort, service_tier)) = self.resolved_model()? else {
            self.connection.execute(
                "UPDATE wrapper_generation SET status = 'failed', error = ?4, error_kind = 'other', updated_at = ?5 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
                params![ACCOUNT, conversation, message, "Les modèles de votre compte ChatGPT ne sont pas encore chargés. Relancez la réponse dans un instant.", now()],
            )?;
            return Ok(());
        };
        let history = self.history(conversation, message)?;
        // Le sujet se précise souvent après la salutation : le titre est écrit au premier
        // échange, puis revu une fois au troisième, tant que l'utilisateur ne l'a pas choisi.
        let turns = history.iter().filter(|(role, _)| *role == "user").count();
        let (automatic, bot_id): (bool, String) = self.connection.query_row(
            "SELECT title_customized = 0, bot_id FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
            params![ACCOUNT, conversation], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        // Les instructions sont lues à chaque réponse : modifier le bot change ses conversations.
        let bot = self
            .bot(&bot_id)?
            .ok_or_else(|| ApiError::new(500, "Le bot de cette conversation est introuvable."))?;
        let bots = self.bots()?;
        self.provider.generate(provider::Run {
            conversation: conversation.into(),
            message: message.into(),
            instructions: compose_instructions(&bot, &bots),
            tools: tools_for(&bot),
            retitle: automatic && matches!(turns, 1 | 3),
            history,
            model,
            effort,
            service_tier,
        });
        Ok(())
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

    /// HTTP entry point; `api` remains the JSON-only compatibility entry point.
    pub fn api_response(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<ApiResponse, ApiError> {
        self.api(method, path, body).map(ApiResponse::Json)
    }

    pub fn api(&mut self, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
        if body.len() > body_limit(path) {
            return Err(ApiError::new(
                413,
                "La requête dépasse la limite de cette route.",
            ));
        }
        match (method, path) {
            ("GET", "/api/state") => {
                let (theme, accent) = self.preferences()?;
                let provider = self.provider.status();
                let resolved = self.resolved_model()?;
                let fast = self.model_preferences()?.2;
                Ok(json!({
                    "account": {"id": ACCOUNT, "name": "Espace local"},
                    "mode": if provider["connected"] == true { "chatgpt" } else { "simulated" },
                    "conversations": self.conversations()?,
                    "bots": self.bots()?,
                    "preferences": {
                        "theme": theme, "accent": accent, "fast": fast,
                        "model": resolved.as_ref().map(|(model, _, _)| model),
                        "effort": resolved.as_ref().and_then(|(_, effort, _)| effort.as_ref()),
                    },
                    "provider": provider,
                }))
            }
            ("POST", "/api/preferences") => {
                let input: Preferences = parse(body)?;
                if input.model.is_some() || input.effort.is_some() || input.fast.is_some() {
                    for value in [&input.model, &input.effort].into_iter().flatten() {
                        validate_id(value)?;
                    }
                    let (theme, accent) = self.preferences()?;
                    self.connection.execute("INSERT INTO wrapper_preferences (account_id, theme, accent) VALUES (?1, ?2, ?3) ON CONFLICT(account_id) DO NOTHING", params![ACCOUNT, theme, accent])?;
                    self.connection.execute("UPDATE wrapper_preferences SET model = COALESCE(?2, model), effort = COALESCE(?3, effort), fast = COALESCE(?4, fast) WHERE account_id = ?1", params![ACCOUNT, input.model, input.effort, input.fast])?;
                }
                if input.theme.is_none() && input.accent.is_none() {
                    if input.model.is_some() || input.effort.is_some() || input.fast.is_some() {
                        let resolved = self.resolved_model()?;
                        return Ok(json!({"preferences": {
                            "fast": self.model_preferences()?.2,
                            "model": resolved.as_ref().map(|(model, _, _)| model),
                            "effort": resolved.as_ref().and_then(|(_, effort, _)| effort.as_ref()),
                        }}));
                    }
                    return Err(ApiError::new(400, "Indiquez un thème ou un accent."));
                }
                let (stored_theme, stored_accent) = self.preferences()?;
                let theme = input.theme.unwrap_or(stored_theme);
                let accent = input.accent.unwrap_or(stored_accent);
                if !matches!(theme.as_str(), "light" | "dark" | "system") {
                    return Err(ApiError::new(
                        400,
                        "Le thème doit être light, dark ou system.",
                    ));
                }
                if !ACCENTS.contains(&accent.as_str()) {
                    return Err(ApiError::new(400, "Cet accent n’existe pas."));
                }
                self.connection.execute("INSERT INTO wrapper_preferences (account_id, theme, accent) VALUES (?1, ?2, ?3) ON CONFLICT(account_id) DO UPDATE SET theme = excluded.theme, accent = excluded.accent", params![ACCOUNT, theme, accent])?;
                Ok(json!({"preferences": {"theme": theme, "accent": accent}}))
            }
            ("POST", "/api/conversations") => {
                let input: CreateConversation = parse(body)?;
                let title = valid_title(input.title.as_deref().unwrap_or(DEFAULT_TITLE))?;
                let bot_id = input.bot_id.as_deref().unwrap_or(DEFAULT_BOT.id);
                validate_id(bot_id)?;
                self.require_bot(bot_id)?;
                let id: String =
                    self.connection
                        .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
                let timestamp = now();
                self.connection.execute("INSERT INTO wrapper_conversations (account_id, id, title, title_customized, created_at, updated_at, bot_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)", params![ACCOUNT, id, title, input.title.is_some(), timestamp, bot_id])?;
                self.conversation_response(&id)
            }
            ("POST", "/api/send") => {
                let input: SendMessage = parse(body)?;
                validate_id(&input.conversation_id)?;
                validate_id(&input.message_id)?;
                self.require_conversation(&input.conversation_id)?;
                let body = input.body.trim();
                if body.is_empty() || body.len() > 16 * 1024 {
                    return Err(ApiError::new(
                        400,
                        "Le message doit contenir entre 1 octet et 16 Ko de texte.",
                    ));
                }
                let conversation =
                    ConversationId::new(&input.conversation_id).expect("validated ID");
                let message = MessageId::new(&input.message_id).expect("validated ID");
                // Le corps est toujours préparé par le moteur, y compris pour vérifier l'idempotence.
                let state =
                    self.session
                        .prepare(conversation.clone(), message.clone(), body.into())?;
                let has_prior_message = self.reader.list_submissions()?.iter().any(|record| {
                    record.conversation_id == conversation && record.message_id != message
                });
                if !has_prior_message {
                    if let Some(title) = suggested_title(body) {
                        self.connection.execute(
                            "UPDATE wrapper_conversations SET title = ?3 WHERE account_id = ?1 AND id = ?2 AND title = ?4 AND title_customized = 0",
                            params![ACCOUNT, input.conversation_id, title, DEFAULT_TITLE],
                        )?;
                    }
                }
                // Le journal fait autorité si une panne intervient avant cette métadonnée.
                self.connection.execute("INSERT OR IGNORE INTO wrapper_messages (account_id, conversation_id, message_id, created_at) VALUES (?1, ?2, ?3, ?4)", params![ACCOUNT, input.conversation_id, input.message_id, now()])?;
                if matches!(state, SubmissionState::Queued) {
                    let mut transport = LocalTransport {
                        connection: &self.connection,
                        lose_reply: input.simulate_loss,
                        live: self.provider.connected(),
                        error: None,
                    };
                    self.session
                        .send_with(&conversation, &message, &mut transport)?;
                    if let Some(error) = transport.error {
                        return Err(error);
                    }
                }
                self.finish_echo(&conversation, &message)?;
                self.start_generation(&input.conversation_id, &input.message_id)?;
                self.touch(&input.conversation_id)?;
                self.conversation_response(&input.conversation_id)
            }
            ("POST", "/api/reconcile") => {
                let input: ReconcileMessage = parse(body)?;
                validate_id(&input.conversation_id)?;
                validate_id(&input.message_id)?;
                self.require_conversation(&input.conversation_id)?;
                let conversation =
                    ConversationId::new(&input.conversation_id).expect("validated ID");
                let message = MessageId::new(&input.message_id).expect("validated ID");
                let state = self
                    .session
                    .state(&conversation, &message)?
                    .ok_or_else(|| ApiError::new(404, "Message introuvable."))?;
                if matches!(
                    state,
                    SubmissionState::Dispatching { .. } | SubmissionState::OutcomeUnknown { .. }
                ) {
                    let mut transport = LocalTransport {
                        connection: &self.connection,
                        lose_reply: false,
                        live: self.provider.connected(),
                        error: None,
                    };
                    self.session
                        .reconcile_with(&conversation, &message, &mut transport)?;
                    if let Some(error) = transport.error {
                        return Err(error);
                    }
                }
                self.finish_echo(&conversation, &message)?;
                self.start_generation(&input.conversation_id, &input.message_id)?;
                self.touch(&input.conversation_id)?;
                self.conversation_response(&input.conversation_id)
            }
            ("POST", "/api/conversations/update") => {
                let input: UpdateConversation = parse(body)?;
                validate_id(&input.id)?;
                self.require_conversation(&input.id)?;
                let title = input.title.as_deref().map(valid_title).transpose()?;
                if title.is_none() && input.favorite.is_none() {
                    return Err(ApiError::new(
                        400,
                        "Choisis un titre ou un favori à modifier.",
                    ));
                }
                self.connection.execute("UPDATE wrapper_conversations SET title = COALESCE(?3, title), title_customized = CASE WHEN ?3 IS NOT NULL THEN 1 ELSE title_customized END, favorite = COALESCE(?4, favorite), updated_at = ?5 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, title, input.favorite, now()])?;
                self.conversation_response(&input.id)
            }
            ("POST", "/api/conversations/delete") => {
                let input: DeleteConversation = parse(body)?;
                validate_id(&input.id)?;
                // Un tombstone garde l'identité afin qu'aucun ancien écho ne ressuscite la conversation.
                let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2)", params![ACCOUNT, input.id], |r| r.get(0))?;
                if !exists {
                    return Err(ApiError::new(404, "Conversation introuvable."));
                }
                self.connection.execute("UPDATE wrapper_conversations SET deleted = 1, updated_at = ?3 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, now()])?;
                Ok(json!({"ok": true}))
            }
            ("POST", "/api/bots") => {
                let input: CreateBot = parse(body)?;
                validate_id(&input.id)?;
                // L'identifiant vient du client : rejouer la même création rend le même bot.
                let fields = BotChanges {
                    name: Some(input.name),
                    color: Some(input.color),
                    shape: Some(input.shape.unwrap_or_else(|| DEFAULT_SHAPE.into())),
                    description: Some(input.description),
                    instructions: Some(input.instructions),
                }
                .validated()?;
                self.create_bot(&input.id, &fields)
            }
            ("POST", "/api/bots/update") => {
                let input: UpdateBot = parse(body)?;
                validate_id(&input.id)?;
                let changes = BotChanges {
                    name: input.name,
                    color: input.color,
                    shape: input.shape,
                    description: input.description,
                    instructions: input.instructions,
                }
                .validated()?;
                self.update_bot(&input.id, &changes)
            }
            ("POST", "/api/proposals/accept") => {
                let input: AcceptProposal = parse(body)?;
                validate_id(&input.id)?;
                self.accept_proposal(&input.id, input.overrides)
            }
            ("POST", "/api/proposals/dismiss") => {
                let input: DismissProposal = parse(body)?;
                validate_id(&input.id)?;
                self.dismiss_proposal(&input.id)
            }
            ("POST", "/api/bots/delete") => {
                let input: DeleteBot = parse(body)?;
                validate_id(&input.id)?;
                if input.id == DEFAULT_BOT.id {
                    return Err(ApiError::new(
                        409,
                        "Le bot par défaut ne peut pas être supprimé.",
                    ));
                }
                if self.bot(&input.id)?.is_none() {
                    return Err(ApiError::new(404, "Bot introuvable."));
                }
                // Une pierre tombale : ses conversations gardent ses instructions.
                self.connection.execute(
                    "UPDATE wrapper_bots SET deleted = 1, updated_at = ?3 WHERE account_id = ?1 AND id = ?2",
                    params![ACCOUNT, input.id, now()],
                )?;
                Ok(json!({"ok": true}))
            }
            ("POST", "/api/provider/signin") => {
                let _: Empty = parse(body)?;
                self.provider
                    .start_signin()
                    .map_err(|failure| ApiError::new(503, failure.message))?;
                Ok(json!({"provider": self.provider.status()}))
            }
            ("POST", "/api/provider/cancel") => {
                let _: Empty = parse(body)?;
                self.provider.cancel_signin();
                Ok(json!({"provider": self.provider.status()}))
            }
            ("POST", "/api/provider/welcomed") => {
                let _: Empty = parse(body)?;
                self.provider
                    .mark_welcomed()
                    .map_err(|_| ApiError::new(500, "Impossible d’enregistrer ce réglage."))?;
                Ok(json!({"provider": self.provider.status()}))
            }
            ("POST", "/api/provider/signout") => {
                let _: Empty = parse(body)?;
                let revoked = self.provider.signout();
                self.connection.execute(
                    "UPDATE wrapper_generation SET status = 'stopped', updated_at = ?1 WHERE status = 'streaming'",
                    [now()],
                )?;
                Ok(json!({"provider": self.provider.status(), "revoked": revoked}))
            }
            ("POST", "/api/stop") => {
                let input: ReconcileMessage = parse(body)?;
                validate_id(&input.conversation_id)?;
                validate_id(&input.message_id)?;
                self.provider
                    .stop(&input.conversation_id, &input.message_id);
                self.connection.execute(
                    "UPDATE wrapper_generation SET status = 'stopped', updated_at = ?4 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status = 'streaming'",
                    params![ACCOUNT, input.conversation_id, input.message_id, now()],
                )?;
                self.conversation_response(&input.conversation_id)
            }
            ("POST", "/api/regenerate") => {
                let input: ReconcileMessage = parse(body)?;
                validate_id(&input.conversation_id)?;
                validate_id(&input.message_id)?;
                self.require_conversation(&input.conversation_id)?;
                if !self.provider.connected() {
                    return Err(ApiError::new(
                        409,
                        "Connectez-vous à ChatGPT pour relancer cette réponse.",
                    ));
                }
                if self
                    .provider
                    .running(&input.conversation_id, &input.message_id)
                {
                    return Err(ApiError::new(409, "Cette réponse est déjà en cours."));
                }
                let restarted = self.connection.execute(
                    "UPDATE wrapper_generation SET status = 'streaming', error = NULL, error_kind = NULL, notice = NULL, updated_at = ?4 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status IN ('failed', 'stopped')",
                    params![ACCOUNT, input.conversation_id, input.message_id, now()],
                )?;
                if restarted == 0 {
                    return Err(ApiError::new(
                        409,
                        "Cette réponse ne peut pas être relancée.",
                    ));
                }
                self.connection.execute(
                    "UPDATE wrapper_remote SET reply = '' WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
                    params![ACCOUNT, input.conversation_id, input.message_id],
                )?;
                self.start_generation(&input.conversation_id, &input.message_id)?;
                self.conversation_response(&input.conversation_id)
            }
            _ => Err(ApiError::new(404, "Cette adresse n'existe pas.")),
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
                "SELECT id, name, color, description, instructions, template, customized, deleted, shape FROM wrapper_bots WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, id],
                bot_from_row,
            )
            .optional()?)
    }

    fn bots(&self) -> Result<Vec<Bot>, ApiError> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, color, description, instructions, template, customized, deleted, shape FROM wrapper_bots WHERE account_id = ?1 ORDER BY id = ?2 DESC, created_at, id",
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

    fn finish_echo(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<(), ApiError> {
        if matches!(
            self.session.state(conversation, message)?,
            Some(SubmissionState::AcceptedAwaitingEcho { .. })
        ) {
            // Une acceptation connue termine son écho sans second envoi ni recherche distante.
            let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3)", params![ACCOUNT, conversation.as_str(), message.as_str()], |r| r.get(0))?;
            if exists {
                self.session.observe_echo(
                    conversation,
                    message,
                    TranscriptEntryId::new(format!("local-{}", message.as_str()))
                        .expect("non-empty local echo ID"),
                )?;
            }
        }
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

    fn conversations(&self) -> Result<Vec<Conversation>, ApiError> {
        let mut conversations = BTreeMap::new();
        let mut hidden = HashSet::new();
        let mut created = BTreeMap::new();
        let mut statement = self.connection.prepare("SELECT id, title, favorite, deleted, created_at, updated_at, bot_id FROM wrapper_conversations WHERE account_id = ?1")?;
        let rows = statement.query_map([ACCOUNT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;
        for row in rows {
            let (id, title, favorite, deleted, created_at, updated_at, bot_id) = row?;
            if deleted {
                hidden.insert(id);
                continue;
            }
            created.insert(id.clone(), created_at);
            conversations.insert(
                id.clone(),
                Conversation {
                    id,
                    bot_id,
                    title,
                    favorite,
                    updated_at,
                    messages: Vec::new(),
                },
            );
        }
        let mut ordered: BTreeMap<String, Vec<(i64, Vec<Message>)>> = BTreeMap::new();
        let mut proposals = self.proposals_by_message()?;
        for record in self.reader.list_submissions()? {
            let id = record.conversation_id.as_str();
            if hidden.contains(id) {
                continue;
            }
            let metadata: Option<(i64, u64)> = self.connection.query_row("SELECT ordinal, created_at FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let (ordinal, created_at) =
                metadata.unwrap_or((i64::MAX, *created.get(id).unwrap_or(&0)));
            let conversation = conversations
                .entry(id.to_owned())
                .or_insert_with(|| Conversation {
                    id: id.to_owned(),
                    bot_id: DEFAULT_BOT.id.into(),
                    title: "Conversation récupérée".into(),
                    favorite: false,
                    updated_at: created_at,
                    messages: Vec::new(),
                });
            conversation.updated_at = conversation.updated_at.max(created_at);
            let mut pair = vec![Message {
                id: record.message_id.as_str().into(),
                role: "user",
                body: record.body,
                state: state_name(&record.state),
                created_at,
                error: None,
                error_kind: None,
                notice: None,
                proposals: Vec::new(),
            }];
            if matches!(
                record.state,
                SubmissionState::AcceptedAwaitingEcho { .. } | SubmissionState::Echoed { .. }
            ) {
                let reply: Option<(String, u64)> = self.connection.query_row("SELECT reply, created_at FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                if let Some((body, created_at)) = reply {
                    let generation: Option<GenerationRow> = self.connection.query_row("SELECT status, error, error_kind, notice FROM wrapper_generation WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
                    let (state, error, error_kind, notice) = match generation {
                        Some((status, error, kind, notice)) => (
                            match status.as_str() {
                                "streaming" => "streaming",
                                "failed" => "failed",
                                "stopped" => "stopped",
                                _ => "echoed",
                            },
                            error,
                            kind,
                            notice,
                        ),
                        None => ("echoed", None, None, None),
                    };
                    pair.push(Message {
                        id: format!("reply-{}", record.message_id.as_str()),
                        role: "assistant",
                        body,
                        state,
                        created_at,
                        error,
                        error_kind,
                        notice,
                        proposals: proposals
                            .remove(&(id.to_owned(), record.message_id.as_str().to_owned()))
                            .unwrap_or_default(),
                    });
                }
            }
            ordered.entry(id.into()).or_default().push((ordinal, pair));
        }
        for (id, mut messages) in ordered {
            messages.sort_by_key(|(ordinal, _)| *ordinal);
            if let Some(conversation) = conversations.get_mut(&id) {
                conversation.messages = messages.into_iter().flat_map(|(_, pair)| pair).collect();
            }
        }
        let mut result: Vec<_> = conversations.into_values().collect();
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(result)
    }
}

struct LocalTransport<'a> {
    connection: &'a Connection,
    lose_reply: bool,
    /// Un fournisseur est connecté : la réponse sera écrite par sa génération, pas par la démo.
    live: bool,
    error: Option<ApiError>,
}

impl LocalTransport<'_> {
    fn accept(&self, request: &SendRequest) -> Result<SendOutcome, ApiError> {
        let account = request.actor().account_id.as_str();
        let conversation = request.conversation_id().as_str();
        let message = request.message_id().as_str();
        let tx = self.connection.unchecked_transaction()?;
        let existing: Option<String> = tx.query_row("SELECT body FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![account, conversation, message], |r| r.get(0)).optional()?;
        let existing_reply_absent = existing.is_none();
        if existing.is_some_and(|body| body != request.body()) {
            return Ok(SendOutcome::Refused(Refusal::Conflict));
        }
        tx.execute("INSERT INTO wrapper_remote (account_id, conversation_id, message_id, body, reply, created_at, send_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) ON CONFLICT(account_id, conversation_id, message_id) DO UPDATE SET send_count = send_count + 1", params![account, conversation, message, request.body(), if self.live { "" } else { demo_reply(request.body()) }, now()])?;
        if self.live && existing_reply_absent {
            tx.execute("INSERT OR IGNORE INTO wrapper_generation (account_id, conversation_id, message_id, status, updated_at) VALUES (?1, ?2, ?3, 'streaming', ?4)", params![account, conversation, message, now()])?;
        }
        tx.commit()?;
        Ok(if self.lose_reply {
            SendOutcome::OutcomeUnknown
        } else {
            SendOutcome::Accepted(DeliveryReceipt {
                account_id: request.actor().account_id.clone(),
                conversation_id: request.conversation_id().clone(),
                message_id: request.message_id().clone(),
            })
        })
    }
}

impl SubmissionTransport for LocalTransport<'_> {
    fn send(&mut self, request: &SendRequest) -> SendOutcome {
        match self.accept(request) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.error = Some(error);
                SendOutcome::OutcomeUnknown
            }
        }
    }

    fn lookup(&mut self, request: &ReconciliationRequest) -> LookupOutcome {
        let found: rusqlite::Result<bool> = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3)", params![request.actor().account_id.as_str(), request.conversation_id().as_str(), request.message_id().as_str()], |r| r.get(0));
        match found {
            Ok(true) => LookupOutcome::Found(DeliveryReceipt {
                account_id: request.actor().account_id.clone(),
                conversation_id: request.conversation_id().clone(),
                message_id: request.message_id().clone(),
            }),
            Ok(false) => LookupOutcome::NotFound,
            Err(error) => {
                self.error = Some(error.into());
                LookupOutcome::Unknown
            }
        }
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

fn suggested_title(body: &str) -> Option<String> {
    let line: String = body
        .lines()
        .next()?
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let title = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    if title.chars().count() <= 80 {
        return Some(title);
    }
    let prefix: String = title.chars().take(79).collect();
    Some(format!("{}…", prefix.trim_end()))
}

fn validate_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err(ApiError::new(400, "Identifiant invalide."));
    }
    Ok(())
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
    if method == "POST" {
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
        if !mime.eq_ignore_ascii_case("application/json") && !(path == "/api/attachments" && image)
        {
            return Err(reject());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
