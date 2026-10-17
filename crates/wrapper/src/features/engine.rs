//! Legacy text generation, deliberately kept intact until gb-engine-cutover.
//! A plain chat reply keeps the `reply-<message_id>` ID and `streaming` state:
//! app.js polling, stop/regenerate and the wrapper verification rely on them.

use crate::*;
use grokbot_domain::{ConversationId, DeliveryReceipt, MessageId, Refusal, TranscriptEntryId};
use grokbot_session::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SubmissionTransport,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendMessage {
    conversation_id: String,
    message_id: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    attachments: Vec<String>,
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
struct Empty {}

/// Kept disabled just as before: this slice does not reactivate bot proposals.
const PROPOSALS_ENABLED: bool = false;

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

pub(crate) fn tools_for(bot: &Bot) -> Vec<Value> {
    if bot.is_default && PROPOSALS_ENABLED {
        proposal_tools()
    } else {
        Vec::new()
    }
}

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
use grokbot_engine::contracts::{
    ContextBlock, ContextRequest, ContextSource, HistoryItem, Role, Trigger,
};

empty_hooks!(
    migrate,
    state,
    tick,
    tools,
    call_tool,
    entry,
    on_run_event,
    link_attachments
);

const INTERRUPTED: &str = "La réponse a été interrompue avant la fin. Vous pouvez la relancer.";
type GenerationRow = (String, Option<String>, Option<String>, Option<String>);

const DEMO_REPLY: &str = "Bien reçu. Ton message est conservé dans cette conversation. Le mode local simule les réponses ; aucun service d'intelligence artificielle n'est connecté.";
const DEMO_DIAGRAM: &str = "Voici un diagramme d'exemple. Le mode local simule les réponses.\n\n```mermaid\nflowchart LR\n  A[Votre message] --> B{Grokbot}\n  B -->|texte| C[Réponse]\n  B -->|schéma| D[Diagramme]\n  C & D --> E[(Historique local)]\n```\n\nPassez la souris sur le diagramme pour le copier, le télécharger ou l'agrandir.";
const DEMO_CODE: &str = "Voici un exemple de code. Le mode local simule les réponses.\n\n```rust\nfn saluer(nom: &str) -> String {\n    format!(\"Bonjour, {nom} !\")\n}\n\nfn main() {\n    println!(\"{}\", saluer(\"Grokbot\"));\n}\n```\n\nLe bouton en haut du bloc copie le code.";
const DEMO_TABLE: &str = "Voici un tableau d'exemple. Le mode local simule les réponses.\n\n| Élément | Rôle | État |\n| --- | --- | --- |\n| Moteur Rust | Garde l'historique | Actif |\n| Interface | Affiche la conversation | Active |\n| Service d'IA | Répond aux messages | **Non connecté** |\n\nLa formule $E = mc^2$ s'affiche aussi.";

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

const BASE_INSTRUCTIONS: &str = "Tu es un assistant dans une application de bureau. Réponds dans la langue de l'utilisateur. Tu peux utiliser le Markdown, des blocs de code avec le nom du langage, des diagrammes dans des blocs ```mermaid et des formules LaTeX entre $...$ ou $$...$$.";
const PROPOSAL_INSTRUCTIONS: &str = "Tu peux proposer à l'utilisateur de créer un nouveau bot avec l'outil propose_bot, ou de modifier un bot existant avec propose_bot_update, quand il le demande ou que c'est clairement utile. Remplis une vraie personnalité : un nom court, une présentation d'une ou deux phrases et des consignes de comportement précises. Rien ne change sans sa confirmation : il verra ta proposition sous ta réponse et choisira. Pour une modification, bot_id est l'identifiant exact d'un bot de la liste.";

pub(crate) fn compose_instructions(bot: &Bot, bots: &[Bot]) -> String {
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

/// No generation thread survives a process restart. The run engine replaces this
/// sweep with durable Interrupted runs at cutover, not in the host or dispatcher.
pub fn startup(conn: &Connection) -> Result<(), ApiError> {
    conn.execute(
        "UPDATE wrapper_generation SET status = 'failed', error = ?1, error_kind = 'other', updated_at = ?2 WHERE status = 'streaming'",
        params![INTERRUPTED, now()],
    )?;
    Ok(())
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
            "/api/send"
                | "/api/reconcile"
                | "/api/provider/signin"
                | "/api/provider/cancel"
                | "/api/provider/welcomed"
                | "/api/provider/signout"
                | "/api/stop"
                | "/api/regenerate"
        )
    {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}

fn route(app: &mut App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    match path {
        "/api/send" => {
            let input: SendMessage = parse(body)?;
            validate_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.require_conversation(&input.conversation_id)?;
            for id in &input.attachments {
                validate_id(id)?;
            }
            let text = input.body.trim();
            let conversation = ConversationId::new(&input.conversation_id).expect("validated ID");
            let message = MessageId::new(&input.message_id).expect("validated ID");
            let previous = app.session.state(&conversation, &message)?;
            if (previous.is_none() && text.is_empty() && input.attachments.is_empty())
                || text.len() > 16 * 1024
            {
                return Err(ApiError::new(
                    400,
                    "Ajoutez du texte (16 Ko au plus) ou une image.",
                ));
            }
            // Existing submissions check their immutable body before any link write.
            // A body-only UI retry must preserve the links of an image-only message.
            let prepared = if previous.is_some() {
                Some(
                    app.session
                        .prepare(conversation.clone(), message.clone(), text.into())?,
                )
            } else {
                None
            };
            // Validate links and reserve the ordinal BEFORE journaling a new message.
            // Failure rolls both back. A crash after commit but before prepare leaves
            // only invisible orphan metadata/links that the same request can reuse.
            let tx = super::write_transaction(&app.connection)?;
            super::conversations::append_entry(
                &tx,
                &input.conversation_id,
                "submission",
                &input.message_id,
                now(),
            )?;
            if previous.is_none() || !input.attachments.is_empty() {
                let mut ctx = app.feature_context();
                ctx.connection = &tx;
                ctx.conversation_id = Some(&input.conversation_id);
                super::link_attachments(
                    &ctx,
                    &input.conversation_id,
                    &input.message_id,
                    &input.attachments,
                )?;
            }
            tx.commit()?;
            let state = match prepared {
                Some(state) => state,
                None => app
                    .session
                    .prepare(conversation.clone(), message.clone(), text.into())?,
            };
            let has_prior_message = app.reader.list_submissions()?.iter().any(|record| {
                record.conversation_id == conversation && record.message_id != message
            });
            if !has_prior_message {
                if let Some(title) = suggested_title(text) {
                    app.connection.execute(
                        "UPDATE wrapper_conversations SET title = ?3 WHERE account_id = ?1 AND id = ?2 AND title = ?4 AND title_customized = 0",
                        params![ACCOUNT, input.conversation_id, title, DEFAULT_TITLE],
                    )?;
                }
            }
            if matches!(state, SubmissionState::Queued) {
                let mut transport = LocalTransport {
                    connection: &app.connection,
                    lose_reply: input.simulate_loss,
                    live: app.provider.connected(),
                    error: None,
                };
                app.session
                    .send_with(&conversation, &message, &mut transport)?;
                if let Some(error) = transport.error {
                    return Err(error);
                }
            }
            app.finish_echo(&conversation, &message)?;
            app.start_generation(&input.conversation_id, &input.message_id)?;
            app.touch(&input.conversation_id)?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/reconcile" => {
            let input: ReconcileMessage = parse(body)?;
            validate_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.require_conversation(&input.conversation_id)?;
            let conversation = ConversationId::new(&input.conversation_id).expect("validated ID");
            let message = MessageId::new(&input.message_id).expect("validated ID");
            let state = app
                .session
                .state(&conversation, &message)?
                .ok_or_else(|| ApiError::new(404, "Message introuvable."))?;
            if matches!(
                state,
                SubmissionState::Dispatching { .. } | SubmissionState::OutcomeUnknown { .. }
            ) {
                let mut transport = LocalTransport {
                    connection: &app.connection,
                    lose_reply: false,
                    live: app.provider.connected(),
                    error: None,
                };
                app.session
                    .reconcile_with(&conversation, &message, &mut transport)?;
                if let Some(error) = transport.error {
                    return Err(error);
                }
            }
            app.finish_echo(&conversation, &message)?;
            app.start_generation(&input.conversation_id, &input.message_id)?;
            app.touch(&input.conversation_id)?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/provider/signin" => {
            let _: Empty = parse(body)?;
            app.provider
                .start_signin()
                .map_err(|failure| ApiError::new(503, failure.message))?;
            Ok(json!({"provider": app.provider.status()}))
        }
        "/api/provider/cancel" => {
            let _: Empty = parse(body)?;
            app.provider.cancel_signin();
            Ok(json!({"provider": app.provider.status()}))
        }
        "/api/provider/welcomed" => {
            let _: Empty = parse(body)?;
            app.provider
                .mark_welcomed()
                .map_err(|_| ApiError::new(500, "Impossible d’enregistrer ce réglage."))?;
            Ok(json!({"provider": app.provider.status()}))
        }
        "/api/provider/signout" => {
            let _: Empty = parse(body)?;
            let revoked = app.provider.signout();
            app.connection.execute("UPDATE wrapper_generation SET status = 'stopped', updated_at = ?1 WHERE status = 'streaming'", [now()])?;
            Ok(json!({"provider": app.provider.status(), "revoked": revoked}))
        }
        "/api/stop" => {
            let input: ReconcileMessage = parse(body)?;
            validate_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.provider.stop(&input.conversation_id, &input.message_id);
            app.connection.execute(
                "UPDATE wrapper_generation SET status = 'stopped', updated_at = ?4 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status = 'streaming'",
                params![ACCOUNT, input.conversation_id, input.message_id, now()],
            )?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/regenerate" => {
            let input: ReconcileMessage = parse(body)?;
            validate_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.require_conversation(&input.conversation_id)?;
            if !app.provider.connected() {
                return Err(ApiError::new(
                    409,
                    "Connectez-vous à ChatGPT pour relancer cette réponse.",
                ));
            }
            if app
                .provider
                .running(&input.conversation_id, &input.message_id)
            {
                return Err(ApiError::new(409, "Cette réponse est déjà en cours."));
            }
            let restarted = app.connection.execute(
                "UPDATE wrapper_generation SET status = 'streaming', error = NULL, error_kind = NULL, notice = NULL, updated_at = ?4 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND status IN ('failed', 'stopped')",
                params![ACCOUNT, input.conversation_id, input.message_id, now()],
            )?;
            if restarted == 0 {
                return Err(ApiError::new(
                    409,
                    "Cette réponse ne peut pas être relancée.",
                ));
            }
            app.connection.execute("UPDATE wrapper_remote SET reply = '' WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, input.conversation_id, input.message_id])?;
            app.start_generation(&input.conversation_id, &input.message_id)?;
            app.conversation_response(&input.conversation_id)
        }
        _ => unreachable!("route filtered above"),
    }
}

/// Model, effort and service tier actually sent to the provider.
type ResolvedModel = (String, Option<String>, Option<String>);

impl App {
    pub(crate) fn model_preferences(
        &self,
    ) -> Result<(Option<String>, Option<String>, bool), ApiError> {
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

    /// Resolve preferences against the account's current catalog, keeping its defaults.
    pub(crate) fn resolved_model(&self) -> Result<Option<ResolvedModel>, ApiError> {
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
        let tier = chosen
            .tiers
            .first()
            .filter(|_| fast)
            .map(|tier| tier.id.clone());
        Ok(Some((chosen.slug.clone(), effort, tier)))
    }

    pub(crate) fn history(
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
                let mut lines = Vec::new();
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

    pub(crate) fn start_generation(
        &self,
        conversation: &str,
        message: &str,
    ) -> Result<(), ApiError> {
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
        let turns = history.iter().filter(|(role, _)| *role == "user").count();
        let (automatic, bot_id): (bool, String) = self.connection.query_row(
            "SELECT title_customized = 0, bot_id FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
            params![ACCOUNT, conversation], |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
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

    pub(crate) fn finish_echo(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<(), ApiError> {
        if matches!(
            self.session.state(conversation, message)?,
            Some(SubmissionState::AcceptedAwaitingEcho { .. })
        ) {
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
}

pub fn reply(
    ctx: &super::Context<'_>,
    conversation: &str,
    message: &str,
    proposals: Vec<Proposal>,
) -> Result<Option<Message>, ApiError> {
    let reply: Option<(String, u64)> = ctx.connection.query_row("SELECT reply, created_at FROM wrapper_remote WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, conversation, message], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((body, created_at)) = reply else {
        return Ok(None);
    };
    let generation: Option<GenerationRow> = ctx.connection.query_row("SELECT status, error, error_kind, notice FROM wrapper_generation WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, conversation, message], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
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
    Ok(Some(Message {
        id: format!("reply-{message}"),
        role: "assistant",
        body,
        state,
        created_at,
        error,
        error_kind,
        notice,
        proposals,
        kind: "message".into(),
        payload: Value::Null,
        attachments: Vec::new(),
    }))
}

pub fn context(ctx: &super::Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    let block = |source, priority, text: String| ContextBlock {
        source,
        priority,
        budget: text.chars().count(),
        text,
    };
    let mut blocks = vec![block(
        ContextSource::AppContract,
        super::priority::APP,
        BASE_INSTRUCTIONS.into(),
    )];
    if let Ok(Some(bot)) = ctx.connection.query_row(
        "SELECT id, name, color, description, instructions, template, customized, deleted, shape FROM wrapper_bots WHERE account_id = ?1 AND id = ?2",
        params![ACCOUNT, request.bot_id], bot_from_row,
    ).optional() {
        let mut identity = format!("Tu t'appelles {}.", bot.name);
        if !bot.description.is_empty() { identity.push_str(&format!("\n\nTon rôle, tel que les autres le voient : {}", bot.description)); }
        blocks.push(block(ContextSource::BotIdentity, super::priority::IDENTITY, identity));
        if !bot.instructions.is_empty() { blocks.push(block(ContextSource::BotInstructions, super::priority::INSTRUCTIONS, format!("Consignes privées sur ta façon de te comporter :\n{}", bot.instructions))); }
    }
    blocks.push(block(
        ContextSource::CurrentMessage,
        super::priority::CURRENT,
        request.current_message.text.clone(),
    ));
    blocks
}

pub fn context_preview(app: &App, conversation_id: &str) -> Result<Vec<ContextBlock>, ApiError> {
    app.require_conversation(conversation_id)?;
    let conversation = app
        .conversations()?
        .into_iter()
        .find(|item| item.id == conversation_id)
        .ok_or_else(|| ApiError::new(404, "Conversation introuvable."))?;
    let current = conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user");
    let message_id = current.map_or("preview", |message| message.id.as_str());
    let request = ContextRequest {
        run_id: "preview".into(),
        trigger: Trigger::UserMessage {
            message_id: message_id.into(),
            attempt: 0,
        },
        bot_id: conversation.bot_id.clone(),
        conversation_id: conversation_id.into(),
        current_message: HistoryItem {
            entry_id: message_id.into(),
            role: Role::User,
            text: current.map_or(String::new(), |message| message.body.clone()),
            images: Vec::new(),
            tool_items: Vec::new(),
        },
    };
    let mut ctx = app.feature_context();
    ctx.bot_id = Some(&conversation.bot_id);
    ctx.conversation_id = Some(conversation_id);
    let mut blocks = super::context(&ctx, &request);
    if let Some(current) = current {
        let history = app.history(conversation_id, &current.id)?;
        // history() includes the current message only after acceptance. A queued
        // input must not accidentally remove the preceding assistant response.
        let included_current = usize::from(matches!(current.state, "accepted" | "echoed"));
        let text = history
            .iter()
            .take(history.len().saturating_sub(included_current))
            .map(|(role, text)| format!("{role}: {text}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        if !text.is_empty() {
            blocks.push(ContextBlock {
                source: ContextSource::History,
                priority: super::priority::HISTORY,
                budget: text.chars().count(),
                text,
            });
        }
    }
    blocks.sort_by_key(|block| block.priority);
    Ok(blocks)
}

pub(crate) struct LocalTransport<'a> {
    pub(crate) connection: &'a Connection,
    pub(crate) lose_reply: bool,
    pub(crate) live: bool,
    pub(crate) error: Option<ApiError>,
}
impl LocalTransport<'_> {
    fn accept(&self, request: &SendRequest) -> Result<SendOutcome, ApiError> {
        let account = request.actor().account_id.as_str();
        let conversation = request.conversation_id().as_str();
        let message = request.message_id().as_str();
        let tx = super::write_transaction(self.connection)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composed_instructions_put_identity_role_and_private_rules_after_the_app_contract() {
        let bot = Bot {
            id: "bot-1".into(),
            name: "Juriste".into(),
            color: "violet".into(),
            shape: "nuage".into(),
            description: "Relit vos contrats.".into(),
            instructions: "Cite toujours l'article.".into(),
            is_default: false,
            template: None,
            customized: false,
            deleted: false,
        };
        assert_eq!(compose_instructions(&bot, &[]), "Tu es un assistant dans une application de bureau. Réponds dans la langue de l'utilisateur. Tu peux utiliser le Markdown, des blocs de code avec le nom du langage, des diagrammes dans des blocs ```mermaid et des formules LaTeX entre $...$ ou $$...$$.\n\nTu t'appelles Juriste.\n\nTon rôle, tel que les autres le voient : Relit vos contrats.\n\nConsignes privées sur ta façon de te comporter :\nCite toujours l'article.");
        let bare = Bot {
            description: String::new(),
            instructions: String::new(),
            ..bot
        };
        assert!(compose_instructions(&bare, &[]).ends_with("$$...$$.\n\nTu t'appelles Juriste."));
    }
}
