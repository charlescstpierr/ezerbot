//! Adaptation locale du moteur Rust. Les états et les corps proviennent du journal durable.

#![forbid(unsafe_code)]

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
    theme TEXT NOT NULL CHECK (theme IN ('light', 'dark', 'system'))
);
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateConversation {
    title: Option<String>,
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
    theme: String,
}

pub struct App {
    connection: Connection,
    reader: SessionStore,
    session: SubmissionSession,
}

impl App {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
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
        let has_title_source: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_conversations') WHERE name = 'title_customized')",
            [], |row| row.get(0),
        )?;
        if !has_title_source {
            connection.execute("ALTER TABLE wrapper_conversations ADD COLUMN title_customized INTEGER NOT NULL DEFAULT 0", [])?;
        }
        Ok(Self {
            connection,
            reader,
            session,
        })
    }

    pub fn api(&mut self, method: &str, path: &str, body: &[u8]) -> Result<Value, ApiError> {
        if body.len() > BODY_LIMIT {
            return Err(ApiError::new(413, "La requête dépasse 64 Ko."));
        }
        match (method, path) {
            ("GET", "/api/state") => Ok(json!({
                "account": {"id": ACCOUNT, "name": "Espace local"},
                "mode": "simulated", "conversations": self.conversations()?,
                "preferences": {"theme": self.connection.query_row("SELECT theme FROM wrapper_preferences WHERE account_id = ?1", [ACCOUNT], |r| r.get::<_, String>(0)).optional()?.unwrap_or_else(|| "system".into())},
            })),
            ("POST", "/api/preferences") => {
                let input: Preferences = parse(body)?;
                if !matches!(input.theme.as_str(), "light" | "dark" | "system") {
                    return Err(ApiError::new(
                        400,
                        "Le thème doit être light, dark ou system.",
                    ));
                }
                self.connection.execute("INSERT INTO wrapper_preferences (account_id, theme) VALUES (?1, ?2) ON CONFLICT(account_id) DO UPDATE SET theme = excluded.theme", params![ACCOUNT, input.theme])?;
                Ok(json!({"preferences": {"theme": input.theme}}))
            }
            ("POST", "/api/conversations") => {
                let input: CreateConversation = parse(body)?;
                let title = valid_title(input.title.as_deref().unwrap_or(DEFAULT_TITLE))?;
                let id: String =
                    self.connection
                        .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
                let timestamp = now();
                self.connection.execute("INSERT INTO wrapper_conversations (account_id, id, title, title_customized, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)", params![ACCOUNT, id, title, input.title.is_some(), timestamp])?;
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
                        error: None,
                    };
                    self.session
                        .send_with(&conversation, &message, &mut transport)?;
                    if let Some(error) = transport.error {
                        return Err(error);
                    }
                }
                self.finish_echo(&conversation, &message)?;
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
                        error: None,
                    };
                    self.session
                        .reconcile_with(&conversation, &message, &mut transport)?;
                    if let Some(error) = transport.error {
                        return Err(error);
                    }
                }
                self.finish_echo(&conversation, &message)?;
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
        let mut statement = self.connection.prepare("SELECT id, title, favorite, deleted, created_at, updated_at FROM wrapper_conversations WHERE account_id = ?1")?;
        let rows = statement.query_map([ACCOUNT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
            ))
        })?;
        for row in rows {
            let (id, title, favorite, deleted, created_at, updated_at) = row?;
            if deleted {
                hidden.insert(id);
                continue;
            }
            created.insert(id.clone(), created_at);
            conversations.insert(
                id.clone(),
                Conversation {
                    id,
                    title,
                    favorite,
                    updated_at,
                    messages: Vec::new(),
                },
            );
        }
        let mut ordered: BTreeMap<String, Vec<(i64, Vec<Message>)>> = BTreeMap::new();
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
            }];
            if matches!(
                record.state,
                SubmissionState::AcceptedAwaitingEcho { .. } | SubmissionState::Echoed { .. }
            ) {
                let reply: Option<(String, u64)> = self.connection.query_row("SELECT reply, created_at FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                if let Some((body, created_at)) = reply {
                    pair.push(Message {
                        id: format!("reply-{}", record.message_id.as_str()),
                        role: "assistant",
                        body,
                        state: "echoed",
                        created_at,
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
    error: Option<ApiError>,
}

impl LocalTransport<'_> {
    fn accept(&self, request: &SendRequest) -> Result<SendOutcome, ApiError> {
        let account = request.actor().account_id.as_str();
        let conversation = request.conversation_id().as_str();
        let message = request.message_id().as_str();
        let tx = self.connection.unchecked_transaction()?;
        let existing: Option<String> = tx.query_row("SELECT body FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![account, conversation, message], |r| r.get(0)).optional()?;
        if existing.is_some_and(|body| body != request.body()) {
            return Ok(SendOutcome::Refused(Refusal::Conflict));
        }
        tx.execute("INSERT INTO wrapper_remote (account_id, conversation_id, message_id, body, reply, created_at, send_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1) ON CONFLICT(account_id, conversation_id, message_id) DO UPDATE SET send_count = send_count + 1", params![account, conversation, message, request.body(), DEMO_REPLY, now()])?;
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

pub fn validate_headers(
    port: u16,
    method: &str,
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
        if !content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
        {
            return Err(reject());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
