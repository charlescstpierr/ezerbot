//! Local run host. REA `goal-execution-autonomy.md` §14 keeps turn, tool and task
//! completion separate; CM11 (`goal-context-memory-learning.md`) keeps reset
//! separate from forgetting. Budgets/recovery policy are local reconstruction.
//! Summary bounds follow RC06, with the 70% fallback from `compaction-runtimes.md`.

use crate::*;
use grokbot_domain::{ConversationId, DeliveryReceipt, MessageId, Refusal, TranscriptEntryId};
use grokbot_engine::contracts::{
    component_version, set_component_version, ApprovalCheck, ContextBlock, ContextBudget,
    ContextRequest, ContextSource, Error, HistoryItem, ImageInput, ModelEvent, ModelGateway,
    ModelRequest, Role, RunBudgets, RunEvent, RunEventKind, RunKind, Summary, SuspendCause,
    TaskStatus, ToolItem, ToolOutcome, ToolSpec, Trigger, TurnEnd,
};
use grokbot_engine::{
    compaction, context as assembler,
    model::{self, Run, RunStatus},
    run_loop::{self, RunServices},
    store,
};
use grokbot_forms::{redact, requests, vault::Vault};
use grokbot_session::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SubmissionTransport,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
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
    mentions: Vec<super::multibot::Mention>,
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
struct Reset {
    conversation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    estimated_tokens: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

empty_hooks!(tools, call_tool, link_attachments);
const BASE_INSTRUCTIONS: &str = "Tu es un assistant dans une application de bureau. Réponds dans la langue de l'utilisateur. Tu peux utiliser le Markdown, des blocs de code avec le nom du langage, des diagrammes dans des blocs ```mermaid et des formules LaTeX entre $...$ ou $$...$$.";
const INTERRUPTED: &str = "La réponse a été interrompue avant la fin. Vous pouvez la relancer.";
const CONTEXT_TOO_LARGE: &str = "Le texte et les pièces jointes dépassent ce que le modèle peut lire en un message. Raccourcissez le texte ou joignez moins de documents dans un nouveau message.";
fn context_budget_failed(conn: &Connection, run: &Run) -> grokbot_engine::contracts::Result<bool> {
    if run.status != RunStatus::Failed {
        return Ok(false);
    }
    Ok(store::events(conn, &run.run_id, 0)?.iter().any(|event| {
        event.kind == RunEventKind::TurnEnded
            && event.payload["failed"] == true
            && event.payload["reason"]
                .as_str()
                .is_some_and(|reason| reason.starts_with("context budget cannot retain"))
    }))
}
const DEMO_REPLY: &str = "Bien reçu. Ton message est conservé dans cette conversation. Le mode local simule les réponses ; aucun service d'intelligence artificielle n'est connecté.";
const DEMO_DIAGRAM: &str = "Voici un diagramme d'exemple. Le mode local simule les réponses.\n\n```mermaid\nflowchart LR\n  A[Votre message] --> B{Ezerbot}\n  B -->|texte| C[Réponse]\n  B -->|schéma| D[Diagramme]\n  C & D --> E[(Historique local)]\n```\n\nPassez la souris sur le diagramme pour le copier, le télécharger ou l'agrandir.";
const DEMO_CODE: &str = "Voici un exemple de code. Le mode local simule les réponses.\n\n```rust\nfn saluer(nom: &str) -> String {\n    format!(\"Bonjour, {nom} !\")\n}\n\nfn main() {\n    println!(\"{}\", saluer(\"Ezerbot\"));\n}\n```\n\nLe bouton en haut du bloc copie le code.";
const DEMO_TABLE: &str = "Voici un tableau d'exemple. Le mode local simule les réponses.\n\n| Élément | Rôle | État |\n| --- | --- | --- |\n| Moteur Rust | Garde l'historique | Actif |\n| Interface | Affiche la conversation | Active |\n| Service d'IA | Répond aux messages | **Non connecté** |\n\nLa formule $E = mc^2$ s'affiche aussi.";
pub(crate) fn demo_reply(body: &str) -> &'static str {
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
// Proposal tools remain disabled, as before this cutover.
#[cfg(test)]
pub(crate) fn tools_for(_: &Bot) -> Vec<Value> {
    Vec::new()
}
#[cfg(test)]
pub(crate) fn compose_instructions(bot: &Bot, _: &[Bot]) -> String {
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
    parts.join("\n\n")
}
fn item(id: &str, role: Role, text: String) -> HistoryItem {
    HistoryItem {
        entry_id: id.into(),
        role,
        text,
        images: vec![],
        tool_items: vec![],
    }
}
fn engine_error(_: ApiError) -> Error {
    Error::InvalidInput("capability storage operation failed".into())
}
fn database(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.pragma_update(None, "secure_delete", true)?;
    Ok(conn)
}
fn vault(path: &Path) -> grokbot_engine::contracts::Result<Vault> {
    Vault::open_app(&path.with_extension("secrets.json"))
}
pub(super) fn budget(conn: &Connection) -> grokbot_engine::contracts::Result<ContextBudget> {
    let tokens = conn.query_row(
        "SELECT estimated_tokens FROM wrapper_engine_config WHERE account_id=?1",
        [ACCOUNT],
        |r| r.get(0),
    )?;
    Ok(ContextBudget {
        estimated_tokens: tokens,
        ..ContextBudget::default()
    })
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    store::migrate(conn)?;
    compaction::migrate(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_engine_runs (
        run_id TEXT PRIMARY KEY, input_json TEXT NOT NULL, live INTEGER NOT NULL,
        model TEXT NOT NULL, effort TEXT, service_tier TEXT, error TEXT, error_kind TEXT,
        notice TEXT, title_state TEXT NOT NULL DEFAULT 'none', retries INTEGER NOT NULL DEFAULT 0,
        images_degraded INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS wrapper_engine_config (
        account_id TEXT PRIMARY KEY, estimated_tokens INTEGER NOT NULL
    );",
    )?;
    let has_degraded: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('wrapper_engine_runs') WHERE name = 'images_degraded')",
        [], |row| row.get(0),
    )?;
    if !has_degraded {
        conn.execute(
            "ALTER TABLE wrapper_engine_runs ADD COLUMN images_degraded INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_engine_config VALUES (?1,100000)",
        [ACCOUNT],
    )?;
    // Only pre-cutover rows are imported. New wrapper_remote rows are acceptance
    // receipts, NOT another response journal. The version write is atomic with import.
    if component_version(conn, "wrapper.engine")? == 0 {
        let mut stmt = conn.prepare("SELECT r.conversation_id,r.message_id,r.body,r.reply,r.created_at,c.bot_id,
            g.status,g.error,g.error_kind,g.notice FROM wrapper_remote r
            JOIN wrapper_conversations c ON c.account_id=r.account_id AND c.id=r.conversation_id
            LEFT JOIN wrapper_generation g ON g.account_id=r.account_id AND g.conversation_id=r.conversation_id AND g.message_id=r.message_id
            WHERE r.account_id=?1 ORDER BY r.created_at,r.rowid")?;
        let rows = stmt
            .query_map([ACCOUNT], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (conversation, message, text, reply, created, bot, status, error, kind, notice) in rows
        {
            let run = Run {
                run_id: legacy_run_id(&conversation, &message),
                bot_id: bot,
                trigger: user_trigger(&conversation, &message, 0),
                conversation_id: conversation,
                kind: RunKind::Chat,
                status: RunStatus::Queued,
                task_status: None,
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at: created,
            };
            let mut run = model::start(conn, &run)?;
            conn.execute("INSERT OR IGNORE INTO wrapper_engine_runs(run_id,input_json,live,model,error,error_kind,notice) VALUES (?1,?2,?3,'',?4,?5,?6)",
                params![run.run_id,serde_json::to_string(&item(&message,Role::User,text)).map_err(|_| ApiError::new(500,"Migration invalide."))?,status.is_some(),error,kind,notice])?;
            model::set_running_at(conn, &run.run_id, created)?;
            if !reply.is_empty() {
                store::append_event(
                    conn,
                    &run.run_id,
                    RunEventKind::TextDelta,
                    &json!({"text":reply,"response":0}),
                )?;
            }
            match status.as_deref() {
                Some("streaming") => {
                    run.status = RunStatus::Interrupted;
                    store::save(conn, &run)?;
                    store::append_event(
                        conn,
                        &run.run_id,
                        RunEventKind::Interrupted,
                        &json!({"reason":"process_restart"}),
                    )?;
                }
                Some("failed") => {
                    run.status = RunStatus::Failed;
                    store::save(conn, &run)?;
                    store::append_event(
                        conn,
                        &run.run_id,
                        RunEventKind::TurnEnded,
                        &json!({"failed":true,"response":0}),
                    )?;
                }
                Some("stopped") => {
                    model::cancel(conn, &run.run_id, TurnEnd::Stopped)?;
                }
                _ => {
                    run.status = RunStatus::Finished;
                    run.turn_end = Some(TurnEnd::Finished);
                    store::save(conn, &run)?;
                    store::append_event(
                        conn,
                        &run.run_id,
                        RunEventKind::TurnEnded,
                        &json!({"turn_end":"finished","response":0}),
                    )?;
                }
            }
        }
        set_component_version(conn, "wrapper.engine", 1)?;
    }
    if component_version(conn, "wrapper.engine")? < 2 {
        set_component_version(conn, "wrapper.engine", 2)?;
    }
    Ok(())
}

pub fn startup(conn: &Connection) -> Result<(), ApiError> {
    conn.pragma_update(None, "secure_delete", true)?;
    let path = Path::new(
        conn.path()
            .ok_or_else(|| ApiError::new(500, "Recovery requires a file database."))?,
    );
    // Construction is inert (no status/auth/network calls). Recovery reactions
    // only use the transaction, under the same no-I/O contract as normal hooks.
    let provider = Provider::with_endpoints(path, Endpoints::default(), false);
    let mut services = Services {
        path,
        provider: &provider,
        connection: conn,
        cancel: Arc::new(AtomicBool::new(false)),
        bot_id: None,
        host_mcp: false,
    };
    for run in store::runs_with_status(conn, RunStatus::Running)? {
        run_loop::with_run_events(conn, &mut services, &run.run_id, |conn, _| {
            let mut current = store::required(conn, &run.run_id)?;
            if current.status == RunStatus::Running {
                current.status = RunStatus::Interrupted;
                store::save(conn, &current)?;
                store::append_event(
                    conn,
                    &current.run_id,
                    RunEventKind::Interrupted,
                    &json!({"reason":"process_restart"}),
                )?;
            }
            Ok(())
        })?;
    }
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
                | "/api/conversations/reset"
                | "/api/engine/config"
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
            validate_conversation_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.require_conversation(&input.conversation_id)?;
            super::delegation::ensure_send_allowed(&app.feature_context(), &input.conversation_id)?;
            for id in &input.attachments {
                validate_id(id)?;
            }
            let text = input.body.trim();
            let conversation = ConversationId::new(&input.conversation_id).expect("validated");
            let message = MessageId::new(&input.message_id).expect("validated");
            let previous = app.session.state(&conversation, &message)?;
            if (previous.is_none() && text.is_empty() && input.attachments.is_empty())
                || text.len() > 16 * 1024
            {
                return Err(ApiError::new(
                    400,
                    "Ajoutez du texte (16 Ko au plus) ou un fichier.",
                ));
            }
            let prepared = if previous.is_some() {
                Some(
                    app.session
                        .prepare(conversation.clone(), message.clone(), text.into())?,
                )
            } else {
                None
            };
            let tx = super::write_transaction(&app.connection)?;
            super::multibot::capture(
                &tx,
                &input.conversation_id,
                &input.message_id,
                &input.mentions,
                text,
                app.provider.connected(),
            )?;
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
            let has_prior = app
                .reader
                .list_submissions()?
                .iter()
                .any(|r| r.conversation_id == conversation && r.message_id != message);
            if !has_prior {
                if let Some(title) = suggested_title(text) {
                    app.connection.execute("UPDATE wrapper_conversations SET title=?3 WHERE account_id=?1 AND id=?2 AND title=?4 AND title_customized=0",params![ACCOUNT,input.conversation_id,title,DEFAULT_TITLE])?;
                }
            }
            if matches!(state, SubmissionState::Queued) {
                let mut transport = LocalTransport {
                    connection: &app.connection,
                    lose_reply: input.simulate_loss,
                    #[cfg(test)]
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
            app.start_message(&input.conversation_id, &input.message_id, 0)?;
            app.touch(&input.conversation_id)?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/reconcile" => {
            let input: ReconcileMessage = parse(body)?;
            validate_conversation_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            app.require_conversation(&input.conversation_id)?;
            let conversation = ConversationId::new(&input.conversation_id).expect("validated");
            let message = MessageId::new(&input.message_id).expect("validated");
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
                    #[cfg(test)]
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
            app.start_message(&input.conversation_id, &input.message_id, 0)?;
            app.touch(&input.conversation_id)?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/provider/signin" => {
            let _: Empty = parse(body)?;
            app.provider
                .start_signin()
                .map_err(|f| ApiError::new(503, f.message))?;
            Ok(json!({"provider":app.provider.status()}))
        }
        "/api/provider/cancel" => {
            let _: Empty = parse(body)?;
            app.provider.cancel_signin();
            Ok(json!({"provider":app.provider.status()}))
        }
        "/api/provider/welcomed" => {
            let _: Empty = parse(body)?;
            app.provider
                .mark_welcomed()
                .map_err(|_| ApiError::new(500, "Impossible d’enregistrer ce réglage."))?;
            Ok(json!({"provider":app.provider.status()}))
        }
        "/api/provider/signout" => {
            let _: Empty = parse(body)?;
            for status in [
                RunStatus::Queued,
                RunStatus::Running,
                RunStatus::Interrupted,
            ] {
                for run in store::runs_with_status(&app.connection, status)? {
                    let live: bool = app.connection.query_row(
                        "SELECT live FROM wrapper_engine_runs WHERE run_id=?1",
                        [&run.run_id],
                        |r| r.get(0),
                    )?;
                    if live {
                        stop_run(&app.feature_context(), &run, TurnEnd::Stopped)?;
                    }
                }
            }
            let revoked = app.provider.signout();
            Ok(json!({"provider":app.provider.status(),"revoked":revoked}))
        }
        "/api/stop" | "/api/regenerate" => {
            let input: ReconcileMessage = parse(body)?;
            validate_conversation_id(&input.conversation_id)?;
            validate_id(&input.message_id)?;
            if path == "/api/regenerate" {
                app.require_conversation(&input.conversation_id)?;
            } else {
                let exists:bool=app.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id=?1 AND id=?2)",params![ACCOUNT,input.conversation_id],|r|r.get(0))?;
                if !exists {
                    return Err(ApiError::new(404, "Conversation introuvable."));
                }
            }
            if path == "/api/stop"
                && super::multibot::is_group(&app.connection, &input.conversation_id)
            {
                let tx = super::write_transaction(&app.connection)?;
                super::multibot::cancel(&app.feature_context(), &tx, &input.conversation_id, None)?;
                tx.commit()?;
                for run in store::runs_for_conversation(&app.connection, &input.conversation_id)? {
                    if !run.status.is_terminal() {
                        stop_run(&app.feature_context(), &run, TurnEnd::Stopped)?;
                    }
                }
                super::multibot::tick(&app.feature_context());
                return app.conversation_response(&input.conversation_id);
            }
            if path == "/api/regenerate"
                && super::multibot::is_group(&app.connection, &input.conversation_id)
            {
                return Err(ApiError::new(409, "Ce tour de groupe est conservé. Envoyez un nouveau message ou une mention pour demander une nouvelle réponse."));
            }
            let trigger = user_trigger(&input.conversation_id, &input.message_id, 0);
            let old = store::latest_run_for_trigger(&app.connection, &trigger)?
                .or(store::get(&app.connection, &input.message_id)?)
                .filter(|r| r.conversation_id == input.conversation_id)
                .ok_or_else(|| ApiError::new(404, "Réponse introuvable."))?;
            if path == "/api/stop" {
                stop_run(&app.feature_context(), &old, TurnEnd::Stopped)?;
                let deleted: bool = app.connection.query_row(
                    "SELECT deleted FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
                    params![ACCOUNT, input.conversation_id],
                    |r| r.get(0),
                )?;
                if deleted {
                    return Ok(json!({"ok":true,"run_id":old.run_id}));
                }
            } else {
                if !matches!(old.trigger, Trigger::UserMessage { .. }) {
                    return Err(ApiError::new(
                        409,
                        "Une réponse de tâche ne peut pas être régénérée.",
                    ));
                }
                let live: bool = app.connection.query_row(
                    "SELECT live FROM wrapper_engine_runs WHERE run_id=?1",
                    [&old.run_id],
                    |r| r.get(0),
                )?;
                if live && !app.provider.connected() {
                    return Err(ApiError::new(
                        409,
                        "Connectez-vous à ChatGPT pour relancer cette réponse.",
                    ));
                }
                if !old.status.is_terminal() && old.status != RunStatus::Interrupted {
                    return Err(ApiError::new(409, "Cette réponse est déjà en cours."));
                }
                if context_budget_failed(&app.connection, &old)? {
                    return Err(ApiError::new(400, CONTEXT_TOO_LARGE));
                }
                let original = run_input(&app.connection, &old.run_id)?;
                super::attachments::validate_message_documents(
                    &app.feature_context(),
                    &input.conversation_id,
                    &original.entry_id,
                )?;
                let Trigger::UserMessage { attempt, .. } = old.trigger else {
                    unreachable!()
                };
                let next = attempt
                    .checked_add(1)
                    .ok_or_else(|| ApiError::new(409, "Trop de relances."))?;
                stop_run(&app.feature_context(), &old, TurnEnd::Superseded)?;
                app.start_message(&input.conversation_id, &input.message_id, next)?;
            }
            app.conversation_response(&input.conversation_id)
        }
        "/api/conversations/reset" => {
            let input: Reset = parse(body)?;
            validate_conversation_id(&input.conversation_id)?;
            app.require_conversation(&input.conversation_id)?;
            if super::multibot::is_group(&app.connection, &input.conversation_id) {
                let tx = super::write_transaction(&app.connection)?;
                super::multibot::cancel(&app.feature_context(), &tx, &input.conversation_id, None)?;
                tx.commit()?;
            }
            for run in store::runs_for_conversation(&app.connection, &input.conversation_id)? {
                if !run.status.is_terminal() {
                    stop_run(&app.feature_context(), &run, TurnEnd::Stopped)?;
                }
            }
            let tx = super::write_transaction(&app.connection)?;
            let id = format!("reset-{}", random_id(&tx)?);
            super::conversations::append_entry(&tx, &input.conversation_id, "reset", &id, now())?;
            compaction::invalidate_all(&tx, &input.conversation_id)?;
            tx.commit()?;
            app.conversation_response(&input.conversation_id)
        }
        "/api/engine/config" => {
            let input: Configuration = parse(body)?;
            // Below 12k the summary's protocol metadata can exceed its own window
            // (compaction r4). Refuse an unusable configuration, not silent loss.
            if !(12000..=2_000_000).contains(&input.estimated_tokens) {
                return Err(ApiError::new(
                    400,
                    "Le budget doit être entre 12 000 et 2 000 000 tokens estimés.",
                ));
            }
            app.connection.execute(
                "UPDATE wrapper_engine_config SET estimated_tokens=?2 WHERE account_id=?1",
                params![ACCOUNT, input.estimated_tokens],
            )?;
            Ok(json!({"estimated_tokens":input.estimated_tokens}))
        }
        _ => unreachable!(),
    }
}
fn suggested_title(body: &str) -> Option<String> {
    let title = body
        .lines()
        .next()?
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        None
    } else if title.chars().count() <= 80 {
        Some(title)
    } else {
        Some(format!(
            "{}…",
            title.chars().take(79).collect::<String>().trim_end()
        ))
    }
}
fn user_trigger(conversation: &str, message: &str, attempt: u32) -> Trigger {
    // API message IDs are conversation-scoped; '/' cannot occur in either ID.
    Trigger::UserMessage {
        message_id: format!("{conversation}/{message}"),
        attempt,
    }
}
fn prompt_cache_key(conversation: &str) -> String {
    use sha2::{Digest, Sha256};
    // Local conversation identity only: no account id, secret, timestamp or run id.
    // Child/task/routine conversations have distinct prompts and their own affinity.
    // Responses limits keys to 64 characters. Keep 128 hash bits plus our prefix (40 total).
    let digest = format!("{:x}", Sha256::digest(conversation.as_bytes()));
    format!("ezerbot-{}", &digest[..32])
}
fn legacy_run_id(conversation: &str, message: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "legacy-{:x}",
        Sha256::digest(serde_json::to_vec(&(conversation, message)).expect("string tuple"))
    )
}
fn random_id(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
}
type ResolvedModel = (String, Option<String>, Option<String>);
impl App {
    #[cfg(test)]
    pub(crate) fn history(
        &self,
        conversation: &str,
        message: &str,
    ) -> Result<Vec<(&'static str, String)>, ApiError> {
        let mut history = Vec::new();
        if let Some(found) = self
            .conversations()?
            .into_iter()
            .find(|c| c.id == conversation)
        {
            for item in found.messages {
                if item.role == "user" {
                    if matches!(item.state, "accepted" | "echoed") {
                        history.push(("user", item.body));
                    }
                    if item.id == message {
                        break;
                    }
                } else if item.state != "failed" && !item.body.is_empty() {
                    history.push(("assistant", item.body));
                }
            }
        }
        Ok(history)
    }
    pub(crate) fn model_preferences(
        &self,
    ) -> Result<(Option<String>, Option<String>, bool), ApiError> {
        Ok(self
            .connection
            .query_row(
                "SELECT model,effort,fast FROM wrapper_preferences WHERE account_id=?1",
                [ACCOUNT],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .unwrap_or((None, None, false)))
    }
    pub(crate) fn resolved_model(&self) -> Result<Option<ResolvedModel>, ApiError> {
        resolve_model(&self.connection, &self.provider).map_err(Into::into)
    }
    fn start_message(
        &self,
        conversation: &str,
        message: &str,
        attempt: u32,
    ) -> Result<(), ApiError> {
        let cid = ConversationId::new(conversation).expect("validated");
        let mid = MessageId::new(message).expect("validated");
        if !matches!(
            self.session.state(&cid, &mid)?,
            Some(SubmissionState::AcceptedAwaitingEcho { .. } | SubmissionState::Echoed { .. })
        ) {
            return Ok(());
        }
        if super::multibot::enqueue(&self.connection, conversation, message)? {
            super::multibot::tick(&self.feature_context());
            return Ok(());
        }
        let bot: String = self.connection.query_row(
            "SELECT bot_id FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
            params![ACCOUNT, conversation],
            |r| r.get(0),
        )?;
        let text:String=self.connection.query_row("SELECT body FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3",params![ACCOUNT,conversation,message],|r|r.get(0))?;
        let mut ctx = self.feature_context();
        ctx.bot_id = Some(&bot);
        ctx.conversation_id = Some(conversation);
        start_run(
            &ctx,
            user_trigger(conversation, message, attempt),
            item(message, Role::User, text),
        )?;
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
            let exists:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3)",params![ACCOUNT,conversation.as_str(),message.as_str()],|r|r.get(0))?;
            if exists {
                self.session.observe_echo(
                    conversation,
                    message,
                    TranscriptEntryId::new(format!("local-{}", message.as_str())).expect("echo ID"),
                )?;
            }
        }
        Ok(())
    }
}
pub(crate) fn resolve_model(
    conn: &Connection,
    provider: &Provider,
) -> grokbot_engine::contracts::Result<Option<ResolvedModel>> {
    let (model, effort, fast): (Option<String>, Option<String>, bool) = conn
        .query_row(
            "SELECT model,effort,fast FROM wrapper_preferences WHERE account_id=?1",
            [ACCOUNT],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .unwrap_or((None, None, false));
    let models = provider.models();
    Ok(models
        .iter()
        .find(|m| Some(&m.slug) == model.as_ref())
        .or_else(|| models.first())
        .map(|m| {
            let effort = effort
                .filter(|e| m.efforts.iter().any(|l| l.effort == *e))
                .or_else(|| m.default_effort.clone());
            (
                m.slug.clone(),
                effort,
                m.tiers.first().filter(|_| fast).map(|t| t.id.clone()),
            )
        }))
}

/// Shared launch point for chat, occurrence and delegation adapters. The caller
/// supplies its own conversation, bot and explicit input (never parent history).
/// Call outside a transaction. Identity, input and model selection commit together.
pub fn start_run(
    ctx: &super::Context<'_>,
    trigger: Trigger,
    input: HistoryItem,
) -> Result<Run, ApiError> {
    create_run(ctx, trigger, input, None)
}
pub(crate) fn start_group_run(
    ctx: &super::Context<'_>,
    turn: &str,
    prompt: &str,
    snapshot: &str,
) -> Result<Run, ApiError> {
    create_run(
        ctx,
        user_trigger(ctx.conversation_id.unwrap(), turn, 0),
        item(turn, Role::User, prompt.into()),
        Some((turn, snapshot)),
    )
}
fn create_run(
    ctx: &super::Context<'_>,
    trigger: Trigger,
    input: HistoryItem,
    group: Option<(&str, &str)>,
) -> Result<Run, ApiError> {
    let bot = ctx
        .bot_id
        .ok_or_else(|| ApiError::new(400, "Bot requis."))?;
    let conversation = ctx
        .conversation_id
        .ok_or_else(|| ApiError::new(400, "Conversation requise."))?;
    let trigger = match trigger {
        Trigger::UserMessage { attempt, .. } => {
            user_trigger(conversation, &input.entry_id, attempt)
        }
        other => other,
    };
    let kind = if matches!(trigger, Trigger::UserMessage { .. }) {
        RunKind::Chat
    } else {
        RunKind::Task
    };
    let run = Run {
        run_id: group
            .map(|(id, _)| format!("group-{id}"))
            .unwrap_or(format!("run-{}", random_id(ctx.connection)?)),
        bot_id: bot.into(),
        conversation_id: conversation.into(),
        trigger,
        kind,
        status: RunStatus::Queued,
        task_status: (kind == RunKind::Task).then_some(TaskStatus::Working),
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: now() as i64,
    };
    let live = if let Some((turn, _)) = group {
        ctx.connection.query_row("SELECT i.live FROM wrapper_group_inputs i JOIN wrapper_group_turns t ON t.account_id=i.account_id AND t.conversation_id=i.conversation_id AND t.root_id=i.message_id WHERE t.id=?1",[turn],|r|r.get(0))?
    } else {
        ctx.provider.connected()
    };
    let (model, effort, tier) = if live {
        resolve_model(ctx.connection, ctx.provider)?.unwrap_or_else(|| (String::new(), None, None))
    } else {
        ("simulated".into(), None, None)
    };
    let mut services = Services::new(ctx)?;
    let saved = run_loop::with_run_events(
        ctx.connection,
        &mut services,
        &run.run_id,
        |conn, _| {
            let saved = model::start(conn, &run)?;
            conn.execute("INSERT OR IGNORE INTO wrapper_engine_runs(run_id,input_json,live,model,effort,service_tier) VALUES (?1,?2,?3,?4,?5,?6)",params![saved.run_id,serde_json::to_string(&input)?,live,model,effort,tier])?;
            if let Some((turn, snapshot)) = group {
                conn.execute("UPDATE wrapper_group_turns SET run_id=?2,snapshot=?3,status='running' WHERE id=?1 AND status='queued'", params![turn,saved.run_id,snapshot])?;
            }
            if group.is_none() && !matches!(saved.trigger, Trigger::UserMessage { .. }) {
                super::conversations::append_entry(
                    conn,
                    conversation,
                    "engine_reply",
                    &saved.run_id,
                    now(),
                )
                .map_err(engine_error)?;
            }
            Ok(saved)
        },
    )?;
    launch(ctx, &saved)?;
    Ok(saved)
}
/// Settings actions are durable host-authored calls, not model instructions.
/// They use the exact same engine/forms gate, journal, wake and tool cards.
pub(crate) fn queue_approved_action(
    ctx: &super::Context<'_>,
    name: &str,
    arguments: Value,
) -> Result<Value, ApiError> {
    let conversation = super::mcp::approval_conversation(ctx)?;
    let run = Run {
        run_id: format!("mcp-host-{}", random_id(ctx.connection)?),
        bot_id: DEFAULT_BOT.id.into(),
        conversation_id: conversation.clone(),
        trigger: Trigger::UserMessage {
            message_id: format!("mcp-settings/{}", random_id(ctx.connection)?),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Queued,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: now() as i64,
    };
    let local = super::Context {
        bot_id: Some(DEFAULT_BOT.id),
        conversation_id: Some(&conversation),
        run_id: Some(&run.run_id),
        ..*ctx
    };
    let mut services = Services::new(&local)?;
    run_loop::with_run_events(ctx.connection, &mut services, &run.run_id, |conn, _| {
        model::start(conn, &run)?;
        conn.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,0,'mcp-local')",params![run.run_id,serde_json::to_string(&item(&run.run_id,Role::User,if name == super::routines_background::ACTION { "Routines quand Ezerbot est fermé".into() } else { "Gestion du connecteur MCP".into() }))?])?;
        model::set_running_at(conn, &run.run_id, now() as i64)?;
        store::append_event(
            conn,
            &run.run_id,
            RunEventKind::ToolCall,
            &json!({"call_id":format!("{}-call",run.run_id),"name":name,"arguments":arguments,"response":0}),
        )?;
        Ok(())
    })?;
    launch(&local, &run)?;
    Ok(json!({"conversation_id":conversation,"run_id":run.run_id}))
}

pub(crate) fn stop_run(ctx: &super::Context<'_>, run: &Run, end: TurnEnd) -> Result<(), ApiError> {
    ctx.provider.cancel_run(&run.run_id);
    let mut services = Services::new(ctx)?;
    run_loop::cancel(ctx.connection, &mut services, &run.run_id, end)?;
    Ok(())
}
pub fn tick(ctx: &super::Context<'_>) {
    tick_selected(ctx, false);
}
pub(crate) fn tick_routines(ctx: &super::Context<'_>) {
    tick_selected(ctx, true);
}
fn tick_selected(ctx: &super::Context<'_>, routines_only: bool) {
    // run() commits Resumed and the matching receipt together. Merely scanning
    // pending wakes must NEVER mark them Delivered (SR01/SR02).
    for status in [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Interrupted,
        RunStatus::AwaitingHumanInput,
        RunStatus::AwaitingApproval,
        RunStatus::AwaitingChildren,
    ] {
        if let Ok(runs) = store::runs_with_status(ctx.connection, status) {
            for run in runs {
                if routines_only {
                    let belongs: bool = ctx.connection.query_row("WITH RECURSIVE ancestors(id,parent_id,kind) AS (SELECT id,parent_id,kind FROM wrapper_conversations WHERE account_id=?1 AND id=?2 UNION SELECT c.id,c.parent_id,c.kind FROM wrapper_conversations c JOIN ancestors a ON a.parent_id=c.id WHERE c.account_id=?1) SELECT EXISTS(SELECT 1 FROM ancestors WHERE kind='routine')", params![ACCOUNT,run.conversation_id], |r| r.get(0)).unwrap_or(false);
                    if !belongs {
                        continue;
                    }
                }
                if status == RunStatus::Interrupted
                    && super::multibot::is_group(ctx.connection, &run.conversation_id)
                {
                    continue;
                }
                if super::multibot::managed(ctx.connection, &run.run_id) {
                    if status.is_suspended() {
                        let _ = reconcile_input_wait(ctx.connection, &run);
                    }
                    continue;
                }
                if status.is_suspended() && reconcile_input_wait(ctx.connection, &run).is_err() {
                    continue;
                }
                if !status.is_suspended()
                    || model::ready_to_resume(ctx.connection, &run.run_id).unwrap_or(false)
                {
                    let _ = launch(ctx, &run);
                }
            }
        }
    }
}
/// Forms receipts name resolutions, whereas suspension events name requests.
/// Bridge that binding before delivering; scanning alone still never ACKs a wake.
fn reconcile_input_wait(conn: &Connection, run: &Run) -> grokbot_engine::contracts::Result<()> {
    if !matches!(
        run.status,
        RunStatus::AwaitingApproval | RunStatus::AwaitingHumanInput
    ) {
        return Ok(());
    }
    for reference in store::awaiting_references(conn, &run.run_id)? {
        let Some(request) = requests::get(conn, &reference)? else {
            continue;
        };
        let resolution = match request.status {
            requests::RequestStatus::Resolved { resolution_id }
            | requests::RequestStatus::Expired { resolution_id }
            | requests::RequestStatus::Cancelled {
                resolution_id: Some(resolution_id),
            } => resolution_id,
            requests::RequestStatus::Denied => {
                let id: Option<String> = conn
                    .query_row(
                        "SELECT resolution_id FROM forms_requests WHERE id=?1",
                        [&request.id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                let Some(id) = id else {
                    continue;
                };
                id
            }
            _ => continue,
        };
        if store::pending_wakes(conn)?
            .iter()
            .any(|w| w.run_id == run.run_id && w.reference == resolution)
        {
            let tx = super::write_transaction(conn)?;
            if store::required(&tx, &run.run_id)?.status == run.status {
                store::set_awaiting(
                    &tx,
                    &run.run_id,
                    &SuspendCause::HumanInput { id: resolution },
                )?;
            }
            tx.commit()?;
        }
    }
    Ok(())
}
pub(crate) fn launch(ctx: &super::Context<'_>, run: &Run) -> Result<(), ApiError> {
    if run.status.is_terminal() {
        return Ok(());
    }
    let deleted: bool = ctx
        .connection
        .query_row(
            "SELECT deleted FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
            params![ACCOUNT, run.conversation_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(true);
    if deleted || !super::delegation::parent_conversations_available(ctx, run)? {
        return stop_run(ctx, run, TurnEnd::Stopped);
    }
    let live: bool = ctx.connection.query_row(
        "SELECT live FROM wrapper_engine_runs WHERE run_id=?1",
        [&run.run_id],
        |r| r.get(0),
    )?;
    // Do not silently turn a real interrupted job into a simulated success.
    if live && !ctx.provider.connected() {
        return Ok(());
    }
    if live {
        let model: String = ctx.connection.query_row(
            "SELECT model FROM wrapper_engine_runs WHERE run_id=?1",
            [&run.run_id],
            |r| r.get(0),
        )?;
        if model.is_empty() {
            let Some((model, effort, tier)) = resolve_model(ctx.connection, ctx.provider)? else {
                if super::multibot::managed(ctx.connection, &run.run_id) {
                    return Ok(());
                }
                ctx.connection.execute("UPDATE wrapper_engine_runs SET error=?2,error_kind='other' WHERE run_id=?1",params![run.run_id,"Les modèles de votre compte ChatGPT ne sont pas encore chargés. Relancez la réponse dans un instant."])?;
                let mut services = Services::new(ctx)?;
                fail_host(ctx.connection, &mut services, &run.run_id)?;
                return Ok(());
            };
            ctx.connection.execute(
                "UPDATE wrapper_engine_runs SET model=?2,effort=?3,service_tier=?4 WHERE run_id=?1",
                params![run.run_id, model, effort, tier],
            )?;
        }
    }
    let images_degraded: bool = ctx.connection.query_row(
        "SELECT images_degraded FROM wrapper_engine_runs WHERE run_id=?1",
        [&run.run_id],
        |row| row.get(0),
    )?;
    let Some(cancel) = ctx.provider.claim_run(&run.run_id) else {
        return Ok(());
    };
    let guard = RunOwner {
        provider: ctx.provider.clone(),
        id: run.run_id.clone(),
        titling: live,
    };
    // Mark before the final event can stop UI polling; clear even on unwind.
    if live {
        ctx.provider.title_started();
    }
    if !live && !run.run_id.starts_with("mcp-host-") {
        let _guard = guard;
        let mut services = Services::new(ctx)?;
        services.cancel = cancel;
        services.bot_id = Some(run.bot_id.clone());
        let gateway = RunGateway {
            path: ctx.db_path,
            provider: ctx.provider,
            run_id: &run.run_id,
            cancel: &services.cancel.clone(),
            image_urls: Mutex::new(HashMap::new()),
            no_vision: AtomicBool::new(false),
            captions_only: AtomicBool::new(images_degraded),
        };
        drive_owned(ctx.connection, &gateway, &mut services, &run.run_id)?;
        return Ok(());
    }
    let path = ctx.db_path.to_path_buf();
    let provider = ctx.provider.clone();
    let id = run.run_id.clone();
    std::thread::spawn(move || {
        let _guard = guard;
        let result = (|| -> grokbot_engine::contracts::Result<()> {
            let conn = database(&path)?;
            let mut services = Services {
                path: &path,
                provider: &provider,
                connection: &conn,
                cancel: cancel.clone(),
                bot_id: Some(store::required(&conn, &id)?.bot_id),
                host_mcp: id.starts_with("mcp-host-"),
            };
            let gateway = RunGateway {
                path: &path,
                provider: &provider,
                run_id: &id,
                cancel: &cancel,
                image_urls: Mutex::new(HashMap::new()),
                no_vision: AtomicBool::new(false),
                captions_only: AtomicBool::new(images_degraded),
            };
            drive_owned(&conn, &gateway, &mut services, &id)?;
            if live {
                retitle(&conn, &provider, &id, &cancel, &path)?;
            }
            Ok(())
        })();
        // A failed DB open retains the durable queued/interrupted record for tick.
        let _ = result;
    });
    Ok(())
}
/// Bound adapter/hook failures in both simulated and connected modes. Retrying
/// retains the committed tool prefix; deterministic errors cannot spin forever.
fn drive_owned(
    conn: &Connection,
    gateway: &dyn ModelGateway,
    services: &mut dyn RunServices,
    id: &str,
) -> grokbot_engine::contracts::Result<()> {
    for _ in 0..4 {
        let result = run_loop::run(conn, gateway, services, id);
        let run = store::required(conn, id)?;
        if run.status.is_terminal() || run.status.is_suspended() {
            return Ok(());
        }
        if run.status == RunStatus::Interrupted
            && super::multibot::is_group(conn, &run.conversation_id)
        {
            // A group's uncertain remote attempt stays uncertain even when a
            // callback/storage failure happens without a process restart.
            return Ok(());
        }
        if result.is_err() {
            let retries:i64=conn.query_row("UPDATE wrapper_engine_runs SET retries=retries+1 WHERE run_id=?1 RETURNING retries",[id],|r|r.get(0))?;
            if retries >= 3 {
                return fail_host(conn, services, id);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Err(Error::InvalidInput(
        "run owner could not make progress".into(),
    ))
}
struct RunOwner {
    provider: Provider,
    id: String,
    titling: bool,
}
impl Drop for RunOwner {
    fn drop(&mut self) {
        if self.titling {
            self.provider.title_finished();
        }
        self.provider.release_run(&self.id);
    }
}
fn fail_host(
    conn: &Connection,
    services: &mut dyn RunServices,
    id: &str,
) -> grokbot_engine::contracts::Result<()> {
    run_loop::with_run_events(conn, services, id, |conn, _| {
        let mut run = store::required(conn, id)?;
        if run.status.is_terminal() {
            return Ok(());
        }
        if run.status == RunStatus::Interrupted {
            model::resume(conn, id)?;
        }
        let events = store::events(conn, id, 0)?;
        let settled: HashSet<_> = events
            .iter()
            .filter(|e| e.kind == RunEventKind::ToolResult)
            .filter_map(|e| e.payload["call_id"].as_str())
            .collect();
        for event in &events {
            if event.kind == RunEventKind::ToolCall {
                if let Some(call) = event.payload["call_id"].as_str() {
                    if !settled.contains(call) {
                        let admitted:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM engine_tool_admissions WHERE run_id=?1 AND call_id=?2)",params![id,call],|r|r.get(0))?;
                        let reason = if admitted {
                            "Résultat d’appel externe non confirmé ; son effet est incertain."
                        } else {
                            "local service unavailable"
                        };
                        store::append_event(
                            conn,
                            id,
                            RunEventKind::ToolResult,
                            &json!({"call_id":call,"outcome":{"status":"failed","reason":reason}}),
                        )?;
                    }
                }
            }
        }
        run = store::required(conn, id)?;
        run.status = RunStatus::Failed;
        run.turn_end = None;
        if run.task_status == Some(TaskStatus::NeedsInput) {
            run.task_status = Some(TaskStatus::Working);
        }
        store::save(conn, &run)?;
        store::append_event(
            conn,
            id,
            RunEventKind::TurnEnded,
            &json!({"failed":true,"reason":"local_service_unavailable"}),
        )?;
        Ok(())
    })
}

struct Services<'a> {
    path: &'a Path,
    provider: &'a Provider,
    connection: &'a Connection,
    cancel: Arc<AtomicBool>,
    bot_id: Option<String>,
    host_mcp: bool,
}
impl<'a> Services<'a> {
    fn new(ctx: &super::Context<'a>) -> grokbot_engine::contracts::Result<Self> {
        Ok(Self {
            path: ctx.db_path,
            provider: ctx.provider,
            connection: ctx.connection,
            cancel: Arc::new(AtomicBool::new(false)),
            bot_id: ctx.bot_id.map(str::to_owned),
            host_mcp: ctx.run_id.is_some_and(|id| id.starts_with("mcp-host-")),
        })
    }
    fn ctx<'b>(&'b self, conn: &'b Connection, run: &'b Run) -> super::Context<'b> {
        super::Context {
            connection: conn,
            provider: self.provider,
            db_path: self.path,
            now_ms: now(),
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
        }
    }
}
impl RunServices for Services<'_> {
    fn context(&mut self, run: &Run) -> grokbot_engine::contracts::Result<ModelRequest> {
        let mut input = run_input(self.connection, &run.run_id)?;
        let (model, effort, service_tier): (String, Option<String>, Option<String>) =
            self.connection.query_row(
                "SELECT model,effort,service_tier FROM wrapper_engine_runs WHERE run_id=?1",
                [&run.run_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
        let ctx = self.ctx(self.connection, run);
        if !super::delegation::parent_conversations_available(&ctx, run)? {
            stop_run(&ctx, run, TurnEnd::Stopped).map_err(engine_error)?;
            return Err(Error::InvalidInput(
                "task conversation is unavailable".into(),
            ));
        }
        let source_message =
            super::multibot::source_message(self.connection, &run.run_id, &input.entry_id)?;
        let linked = super::attachments::images_for_message(
            &ctx,
            &run.conversation_id,
            &source_message,
            &model,
            false,
        )
        .map_err(engine_error)?;
        if !linked.is_empty() && input.images != linked {
            input.images = linked;
            // Transport hydration happens only at the final model boundary;
            // SQLite retains references/captions, and unchanged input stays inert.
            self.connection.execute(
                "UPDATE wrapper_engine_runs SET input_json=?2 WHERE run_id=?1",
                params![run.run_id, serde_json::to_string(&input)?],
            )?;
        }
        let mut request = ContextRequest {
            run_id: run.run_id.clone(),
            trigger: run.trigger.clone(),
            bot_id: run.bot_id.clone(),
            conversation_id: run.conversation_id.clone(),
            current_message: input.clone(),
        };
        let mut blocks =
            super::delegation::with_context(&ctx, &request, || super::context(&ctx, &request))?
                .into_iter()
                // Captions are hydrated into history below for reset/summary/budget
                // selection. Do not also send the legacy aggregate instruction copy.
                .filter(|b| {
                    b.source != ContextSource::CurrentMessage
                        && !(b.source == ContextSource::ImageCaption
                            && b.text.starts_with("<untrusted-image-captions>"))
                })
                .collect::<Vec<_>>();
        if let Some(images) = take_image_inputs(&mut blocks)? {
            if input.images != images {
                input.images = images;
                self.connection.execute(
                    "UPDATE wrapper_engine_runs SET input_json=?2 WHERE run_id=?1",
                    params![run.run_id, serde_json::to_string(&input)?],
                )?;
            }
            request.current_message = input.clone();
        }
        // First fence capability data. The final pass after replay performs the
        // actual full-request budget, so current-run tool outputs cannot bypass it.
        let assembled = assembler::assemble_input(
            &request,
            &blocks,
            &[],
            None,
            None,
            &budget(self.connection)?,
        )?;
        let instructions = assembled
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let (mut history, _) = history(self.connection, run, &input)?;
        hydrate_history_images(&ctx, &run.conversation_id, &model, &mut history)?;
        input.text.push_str(
            &super::attachments::document_text(&ctx, &run.conversation_id, &input.entry_id)
                .map_err(engine_error)?,
        );
        if source_message != input.entry_id {
            // A group's sending image is attached to its distinct turn input;
            // the shared human entry must not repeat that caption a second time.
            for entry in history
                .iter_mut()
                .filter(|entry| entry.entry_id == source_message)
            {
                entry.images.clear();
            }
        }
        history.push(input);
        Ok(ModelRequest {
            model,
            instructions,
            input: history,
            tools: vec![],
            effort,
            service_tier,
            prompt_cache_key: Some(prompt_cache_key(&run.conversation_id)),
        })
    }
    fn tools(&self) -> Vec<ToolSpec> {
        let mut tools = super::tools();
        let ctx = super::Context {
            connection: self.connection,
            provider: self.provider,
            db_path: self.path,
            now_ms: now(),
            bot_id: self.bot_id.as_deref(),
            conversation_id: None,
            run_id: self.host_mcp.then_some("mcp-host-"),
        };
        tools.extend(super::mcp::scoped_tools(&ctx));
        if self.host_mcp {
            tools.extend(super::routines_background::host_tools());
        }
        tools
    }
    fn tools_for_run(&self, run: &Run) -> Vec<ToolSpec> {
        self.tools()
            .into_iter()
            .filter(|s| {
                !super::local::is_tool(&s.name)
                    || super::local::enabled(self.connection, &run.bot_id, &s.name)
            })
            .collect()
    }
    fn call_tool(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> grokbot_engine::contracts::Result<ToolOutcome> {
        let outcome = if matches!(call.name.as_str(), "propose_bot" | "propose_bot_update") {
            // Proposal tools are not advertised; retain their existing recording
            // adapter for a future explicit enable, without a second generator.
            let input = run_input(conn, &run.run_id)?;
            let proposal = crate::provider::ToolCall {
                item_id: String::new(),
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                arguments: serde_json::to_string(&call.arguments)?,
            };
            match crate::record_proposal(conn, &run.conversation_id, &input.entry_id, &proposal)
                .map_err(engine_error)?
            {
                Some(notice) => ToolOutcome::Failed { reason: notice },
                None => ToolOutcome::Done {
                    output: json!({"proposed":true}),
                },
            }
        } else {
            super::call_tool(&self.ctx(conn, run), call).unwrap_or_else(|| ToolOutcome::Failed {
                reason: "unknown tool".into(),
            })
        };
        let value = redact::redact_value(&vault(self.path)?, &serde_json::to_value(outcome)?)?;
        Ok(serde_json::from_value(value)?)
    }
    fn is_external_tool(&self, call: &grokbot_engine::contracts::ToolCall) -> bool {
        call.name.starts_with("mcp_") || call.name == super::routines_background::ACTION
    }
    fn call_external_tool(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> grokbot_engine::contracts::Result<ToolOutcome> {
        self.call_tool(conn, run, call)
    }
    fn is_journaled_external_tool(&self, call: &grokbot_engine::contracts::ToolCall) -> bool {
        super::local::is_tool(&call.name)
    }
    fn external_unavailable_reason(
        &self,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> Option<String> {
        super::local::validate(&self.ctx(self.connection, run), call)
            .err()
            .map(|e| e.message)
    }
    fn call_journaled_external_tool(
        &mut self,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> ToolOutcome {
        let outcome =
            super::local::execute(&self.ctx(self.connection, run), call, self.cancel.clone());
        // No private vault values may leak through command/file output.
        match vault(self.path)
            .and_then(|v| redact::redact_value(&v, &serde_json::to_value(outcome)?))
            .and_then(|value| Ok(serde_json::from_value(value)?))
        {
            Ok(outcome) => outcome,
            Err(_) => ToolOutcome::Failed {
                reason: "Sortie locale indisponible.".into(),
            },
        }
    }
    fn check_approval(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> grokbot_engine::contracts::Result<ApprovalCheck> {
        if super::local::is_tool(&call.name) {
            if let Err(error) = super::local::validate(&self.ctx(conn, run), call) {
                return Ok(ApprovalCheck::Denied {
                    reason: error.message,
                });
            }
        }
        let decision = super::forms::check_tool_approval(&self.ctx(conn, run), run, call)
            .map_err(engine_error)?;
        if call.name == "write_file" && matches!(decision, ApprovalCheck::Ask) {
            if let Err(error) = super::local::approval_preview(&self.ctx(conn, run), call) {
                return Ok(ApprovalCheck::Denied {
                    reason: error.message,
                });
            }
        }
        Ok(decision)
    }
    fn request_approval(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &grokbot_engine::contracts::ToolCall,
    ) -> grokbot_engine::contracts::Result<SuspendCause> {
        super::forms::request_tool_approval(&self.ctx(conn, run), run, call).map_err(engine_error)
    }
    fn is_cancelled(&self, _: &str) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
    fn on_run_event(
        &mut self,
        conn: &Connection,
        event: &RunEvent,
    ) -> grokbot_engine::contracts::Result<()> {
        let run = store::required(conn, &event.run_id)?;
        let kind = match event.kind {
            RunEventKind::ToolCall => Some("engine_tool"),
            RunEventKind::TaskChanged
            | RunEventKind::Suspended
            | RunEventKind::TurnEnded
            | RunEventKind::Cancelled
                if run.kind == RunKind::Task =>
            {
                Some("engine_task")
            }
            _ => None,
        };
        if let Some(kind) = kind {
            super::conversations::append_entry(
                conn,
                &run.conversation_id,
                kind,
                &format!("{}:{}", run.run_id, event.seq),
                now(),
            )
            .map_err(engine_error)?;
        }
        // Check the forms reaction before the legacy void registry hook; any
        // invalidation/retargeting failure rolls back this event and is retried.
        let ctx = self.ctx(conn, &run);
        super::forms::on_run_event_checked(&ctx, event).map_err(engine_error)?;
        // Registry reactions are idempotent and share this SAME writer transaction.
        super::on_run_event(&ctx, event);
        Ok(())
    }
    fn resolution(
        &mut self,
        conn: &Connection,
        run: &Run,
        call_id: &str,
    ) -> grokbot_engine::contracts::Result<Option<Value>> {
        resolution(conn, run, call_id)
    }
}
fn resolution(
    conn: &Connection,
    run: &Run,
    call_id: &str,
) -> grokbot_engine::contracts::Result<Option<Value>> {
    let events = store::events(conn, &run.run_id, 0)?;
    let id = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == call_id)
        .and_then(|e| e.payload["outcome"]["cause"]["id"].as_str());
    let Some(id) = id else {
        return Ok(None);
    };
    let Some(request) = requests::get(conn, id)? else {
        return Ok(None);
    };
    let resolution = match request.status {
        requests::RequestStatus::Resolved { resolution_id }
        | requests::RequestStatus::Expired { resolution_id }
        | requests::RequestStatus::Cancelled {
            resolution_id: Some(resolution_id),
        } => resolution_id,
        _ => return Ok(None),
    };
    Ok(Some(requests::resolved_values(conn, &resolution)?))
}
const IMAGE_INPUT_PREFIX: &str = "grokbot:image-inputs:v1:";
/// Typed extension for attachments' context hook. Transport bytes are extracted
/// into HistoryItem.images, NEVER inserted into instructions or charged as text.
/// Emit only images already validated/linked to request.current_message.entry_id.
pub fn image_input_block(images: &[ImageInput]) -> grokbot_engine::contracts::Result<ContextBlock> {
    let text = format!("{IMAGE_INPUT_PREFIX}{}", serde_json::to_string(images)?);
    Ok(ContextBlock {
        source: ContextSource::ImageCaption,
        priority: super::priority::HISTORY,
        budget: 0,
        text,
    })
}
fn take_image_inputs(
    blocks: &mut Vec<ContextBlock>,
) -> grokbot_engine::contracts::Result<Option<Vec<ImageInput>>> {
    let mut images = None;
    for block in blocks.iter() {
        if block.source == ContextSource::ImageCaption {
            if let Some(json) = block.text.strip_prefix(IMAGE_INPUT_PREFIX) {
                images = Some(serde_json::from_str(json)?);
            }
        }
    }
    blocks.retain(|b| {
        !(b.source == ContextSource::ImageCaption && b.text.starts_with(IMAGE_INPUT_PREFIX))
    });
    Ok(images)
}
fn hydrate_history_images(
    ctx: &super::Context<'_>,
    conversation: &str,
    model: &str,
    history: &mut [HistoryItem],
) -> grokbot_engine::contracts::Result<()> {
    for item in history.iter_mut().filter(|i| i.role == Role::User) {
        item.text.push_str(
            &super::attachments::document_text(ctx, conversation, &item.entry_id)
                .map_err(engine_error)?,
        );
        let linked =
            super::attachments::images_for_message(ctx, conversation, &item.entry_id, model, false)
                .map_err(engine_error)?;
        if !linked.is_empty() {
            item.images = linked;
        }
    }
    Ok(())
}
fn run_input(conn: &Connection, id: &str) -> grokbot_engine::contracts::Result<HistoryItem> {
    let text: String = conn.query_row(
        "SELECT input_json FROM wrapper_engine_runs WHERE run_id=?1",
        [id],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&text)?)
}
/// Read raw, ordered evidence. Select the LAST reset preceding this particular
/// input, not the last reset of the conversation (context r3/r4).
fn history(
    conn: &Connection,
    run: &Run,
    current: &HistoryItem,
) -> grokbot_engine::contracts::Result<(Vec<HistoryItem>, Option<String>)> {
    if let Some(snapshot) = super::multibot::snapshot(conn, &run.run_id)? {
        return Ok((snapshot, None));
    }
    let mut stmt=conn.prepare("SELECT message_id,kind FROM wrapper_messages WHERE account_id=?1 AND conversation_id=?2 ORDER BY ordinal")?;
    let rows = stmt
        .query_map(params![ACCOUNT, run.conversation_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut result = Vec::new();
    let mut reset = None;
    for (id, kind) in rows {
        if id == current.entry_id || (kind == "engine_reply" && id == run.run_id) {
            break;
        }
        match kind.as_str() {
            "reset" | "engine_reset" => {
                reset = Some(id.clone());
                result.push(item(&id, Role::System, String::new()));
            }
            "submission" => {
                let text:Option<String>=conn.query_row("SELECT body FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3",params![ACCOUNT,run.conversation_id,id],|r|r.get(0)).optional()?;
                let Some(text) = text else {
                    continue;
                };
                let mut user = item(&id, Role::User, text);
                let previous = store::latest_run_for_trigger(
                    conn,
                    &user_trigger(&run.conversation_id, &id, 0),
                )?;
                if let Some(previous) = &previous {
                    user.images = run_input(conn, &previous.run_id)?.images;
                }
                result.push(user);
                if let Some(previous) = previous {
                    result.extend(replay(conn, &previous)?);
                }
            }
            "engine_reply" => {
                if let Some(previous) = store::get(conn, &id)? {
                    result.push(run_input(conn, &id)?);
                    result.extend(replay(conn, &previous)?);
                }
            }
            _ => {}
        }
    }
    Ok((result, reset))
}
fn replay(conn: &Connection, run: &Run) -> grokbot_engine::contracts::Result<Vec<HistoryItem>> {
    let events = store::events(conn, &run.run_id, 0)?;
    let mut items = run_loop::replay_items(&events)?;
    // replay_items coalesces each completed response before this defense-in-depth
    // redaction, including legacy events written by a pre-cutover generator.
    let vault = event_vault(conn)?;
    for item in &mut items {
        item.text = redact::redact(&vault, &item.text)?;
        for tool in &mut item.tool_items {
            if let ToolItem::Output { call_id, output } = tool {
                if output["status"] == "suspend" {
                    if let Some(value) = resolution(conn, run, call_id)? {
                        *output = json!({"status":"resolved","value":value});
                    }
                }
            }
        }
    }
    Ok(items)
}

/// Final model boundary: full replay is budgeted/fenced here, not just the base
/// context before run_loop appends its events. Compaction never rewrites transcript.
#[derive(Clone, Copy)]
enum ImageFallback {
    NoVision,
    RejectedFile,
}

struct RunGatewayCall<'a> {
    path: &'a Path,
    provider: &'a Provider,
    connection: &'a Connection,
    run_id: &'a str,
    cancel: &'a AtomicBool,
    source_revision: u64,
    image_urls: &'a Mutex<HashMap<String, String>>,
    no_vision: &'a AtomicBool,
    captions_only: &'a AtomicBool,
}
impl RunGatewayCall<'_> {
    /// A whole sending image suffix is useful only while it fits. Once a
    /// budget-bound preparation fails, allow normal rolling compaction rather
    /// than fail the task. This is not evidence about model capabilities.
    fn degrade_image_budget(
        &self,
        error: &Error,
        has_image_bytes: bool,
    ) -> grokbot_engine::contracts::Result<bool> {
        let budget_error = matches!(error, Error::InvalidInput(message)
            if message.contains("budget") || message.contains("cannot fit") || message.contains("input window"));
        if !has_image_bytes || !budget_error || self.captions_only.load(Ordering::SeqCst) {
            return Ok(false);
        }
        self.captions_only.store(true, Ordering::SeqCst);
        self.connection.execute(
            "UPDATE wrapper_engine_runs SET notice=?2,images_degraded=1 WHERE run_id=?1",
            params![self.run_id, "La limite de contexte impose un tour sans octets d'image ; seules les descriptions disponibles sont conservées."],
        )?;
        Ok(true)
    }

    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        let mut request = request.clone();
        // Typed function outputs have no role-bearing text. Avoid fabricating a
        // free-form Tool message when context fences an empty Role::Tool.text.
        for entry in &mut request.input {
            if entry.role == Role::Tool && entry.text.is_empty() && !entry.tool_items.is_empty() {
                entry.role = Role::Assistant;
            }
        }
        let run = store::required(self.connection, self.run_id)?;
        // Preserve the single-bot model contract: the group-only tool is not
        // advertised in direct chats or unrelated delegated/routine threads.
        if !super::multibot::managed(self.connection, self.run_id) {
            request.tools.retain(|tool| tool.name != "message_bot");
        }
        let original = run_input(self.connection, self.run_id)?;
        let (_, reset) = history(self.connection, &run, &original)?;
        let ctx = super::Context {
            connection: self.connection,
            provider: self.provider,
            db_path: self.path,
            now_ms: now(),
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
        };
        let source_message =
            super::multibot::source_message(self.connection, &run.run_id, &original.entry_id)?;
        let sending_bytes =
            !self.no_vision.load(Ordering::SeqCst) && !self.captions_only.load(Ordering::SeqCst);
        let followups: HashSet<String> = store::events(self.connection, &run.run_id, 0)?
            .iter()
            .filter(|e| e.kind == RunEventKind::InputReceived)
            .filter_map(|e| e.payload["input"]["entry_id"].as_str().map(str::to_owned))
            .collect();
        // InputConsumed is a request checkpoint, not the end of a sending turn:
        // keep this run's follow-up bytes through its tool continuations too.
        let sending_entry_ids: Vec<&str> = std::iter::once(original.entry_id.as_str())
            .chain(followups.iter().map(String::as_str))
            .collect();
        for sending in request.input.iter_mut().filter(|entry| {
            entry.role == Role::User
                && (entry.entry_id == original.entry_id || followups.contains(&entry.entry_id))
        }) {
            if sending.entry_id == original.entry_id
                && matches!(run.trigger, Trigger::Delegation { .. })
            {
                let sources = super::delegation::material_sources(self.connection, &run)?;
                let (images, documents) = super::attachments::selected_material(
                    &ctx,
                    &sources,
                    &request.model,
                    sending_bytes,
                    &mut self.image_urls.lock().unwrap_or_else(|e| e.into_inner()),
                )
                .map_err(engine_error)?;
                sending.images = images;
                sending.text.push_str(&documents);
            } else {
                let message = if sending.entry_id == original.entry_id {
                    &source_message
                } else {
                    &sending.entry_id
                };
                let linked = super::attachments::images_for_message_cached(
                    &ctx,
                    &run.conversation_id,
                    message,
                    &request.model,
                    sending_bytes,
                    &mut self.image_urls.lock().unwrap_or_else(|e| e.into_inner()),
                )
                .map_err(engine_error)?;
                if !linked.is_empty() {
                    sending.images = linked;
                }
                if followups.contains(&sending.entry_id) {
                    sending.text.push_str(
                        &super::attachments::document_text(&ctx, &run.conversation_id, message)
                            .map_err(engine_error)?,
                    );
                }
            }
        }
        if self.no_vision.load(Ordering::SeqCst) {
            for entry in &mut request.input {
                super::attachments::without_vision(&mut entry.images);
            }
        }
        if self.captions_only.load(Ordering::SeqCst) {
            for entry in &mut request.input {
                super::attachments::captions_only(&mut entry.images);
            }
        }
        let has_sending_bytes = request.input.iter().any(|entry| {
            sending_entry_ids.contains(&entry.entry_id.as_str())
                && entry.images.iter().any(|image| image.url.is_some())
        });
        let reset_index = reset
            .as_ref()
            .and_then(|id| request.input.iter().position(|i| &i.entry_id == id))
            .map_or(0, |n| n + 1);
        // Compaction may summarize previous turns, never the sending image or
        // its current tool continuation. Its bytes must remain available until
        // this run ends, even with store:false and a rolling summary.
        let compaction_end = request
            .input
            .iter()
            .position(|entry| {
                sending_entry_ids.contains(&entry.entry_id.as_str())
                    && entry.images.iter().any(|image| image.url.is_some())
            })
            .map_or(request.input.len(), |index| index + 1);
        let eligible = &request.input[reset_index..compaction_end];
        let segment = eligible
            .first()
            .map(|i| i.entry_id.as_str())
            .unwrap_or(&original.entry_id);
        let mut summary =
            compaction::latest_in_segment(self.connection, &run.conversation_id, segment)?;
        // A later head is not applicable to a regenerated old turn. Walk parents;
        // a new root for that old turn stays ephemeral, never poisoning the head.
        let head = summary.as_ref().map(|s| s.summary_id.clone());
        while let Some(s) = &summary {
            if eligible
                .iter()
                .any(|i| i.entry_id == s.resume_from_entry_inclusive)
            {
                break;
            }
            summary=match &s.previous_summary_id {Some(id)=>self.connection.query_row("SELECT record FROM engine_summaries WHERE summary_id=?1 AND record IS NOT NULL",[id],|r|r.get::<_,String>(0)).optional()?.map(|s|serde_json::from_str(&s)).transpose()?,None=>None};
        }
        let ephemeral =
            head.is_some() && summary.as_ref().map(|s| s.summary_id.as_str()) != head.as_deref();
        let budget = budget(self.connection)?;
        let vault = vault(self.path)?;
        let mut pending = HashSet::new();
        for status in [
            RunStatus::Running,
            RunStatus::Interrupted,
            RunStatus::AwaitingApproval,
            RunStatus::AwaitingHumanInput,
            RunStatus::AwaitingChildren,
        ] {
            for other in store::runs_with_status(self.connection, status)? {
                if other.conversation_id == run.conversation_id {
                    let events = store::events(self.connection, &other.run_id, 0)?;
                    let settled: HashSet<_> = events
                        .iter()
                        .filter(|e| e.kind == RunEventKind::ToolResult)
                        .filter_map(|e| e.payload["call_id"].as_str())
                        .collect();
                    for event in &events {
                        if event.kind == RunEventKind::ToolCall {
                            if let Some(id) = event.payload["call_id"].as_str() {
                                if !settled.contains(id) {
                                    pending.insert(id.into());
                                }
                            }
                        }
                    }
                }
            }
        }
        if super::multibot::managed(self.connection, self.run_id) {
            summary = None;
        }
        loop {
            if super::multibot::managed(self.connection, self.run_id) {
                break;
            }
            let start = summary
                .as_ref()
                .and_then(|s| {
                    eligible
                        .iter()
                        .position(|i| i.entry_id == s.resume_from_entry_inclusive)
                })
                .unwrap_or(0);
            let characters = eligible[start..]
                .iter()
                .map(assembler::input_characters)
                .collect::<grokbot_engine::contracts::Result<Vec<_>>>()?
                .into_iter()
                .sum::<usize>()
                + request.instructions.chars().count()
                + summary.as_ref().map_or(0, |s| s.text.chars().count());
            if eligible.len().saturating_sub(start) < 2
                || !compaction::should_compact(characters, &budget)?
            {
                break;
            }
            let mut services = Services {
                path: self.path,
                provider: self.provider,
                connection: self.connection,
                cancel: Arc::new(AtomicBool::new(false)),
                bot_id: Some(run.bot_id.clone()),
                host_mcp: false,
            };
            let reserved = run_loop::with_run_events(
                self.connection,
                &mut services,
                self.run_id,
                |conn, _| {
                    let mut usage = store::budget_usage(conn, self.run_id, now() as i64)?;
                    // run_loop reserves the ordinary response AFTER context(). This
                    // boundary already has that reservation: leave it available.
                    if usage.model_calls >= run.budgets.model_calls
                        || self.cancel.load(Ordering::SeqCst)
                    {
                        return Ok(false);
                    }
                    usage.model_calls += 1;
                    store::save_budget_usage(conn, self.run_id, &usage)?;
                    Ok(true)
                },
            )?;
            if !reserved {
                break;
            }
            let revision = self.source_revision;
            let options = compaction::SummaryOptions {
                source_revision: revision,
                pending_call_ids: pending.clone(),
                budget: budget.clone(),
            };
            let clean: Vec<HistoryItem> = serde_json::from_value(redact::redact_value(
                &vault,
                &serde_json::to_value(eligible)?,
            )?)?;
            let gateway = ProviderGateway {
                provider: self.provider,
                cancel: self.cancel,
                metadata: None,
            };
            let mut next = match compaction::summarize_with_options(
                &gateway,
                &request.model,
                &run.conversation_id,
                &clean,
                summary.as_ref(),
                &options,
            ) {
                Ok(summary) => summary,
                Err(error) if self.degrade_image_budget(&error, has_sending_bytes)? => {
                    return self.stream(&request, emit);
                }
                Err(error) => return Err(error),
            };
            next.text = redact::redact(&vault, &next.text)?;
            if !ephemeral {
                let tx = super::write_transaction(self.connection)?;
                if compaction::revision(&tx, &run.conversation_id)? != revision {
                    return Err(Error::Conflict("context source changed".into()));
                }
                compaction::save_with_history_pending_at_revision(
                    &tx, &next, &clean, &pending, revision,
                )
                .map_err(|_| Error::Conflict("summary head changed; rebuild context".into()))?;
                super::conversations::append_entry(
                    &tx,
                    &run.conversation_id,
                    "engine_summary",
                    &next.summary_id,
                    now(),
                )
                .map_err(engine_error)?;
                tx.commit()?;
            }
            if summary
                .as_ref()
                .is_some_and(|s| s.resume_from_entry_inclusive == next.resume_from_entry_inclusive)
            {
                break;
            }
            summary = Some(next);
        }
        // Keep built-in tool protocol with the app contract, BEFORE bot identity.
        let (instructions, protocol) = request
            .instructions
            .rsplit_once("\nUse begin_task")
            .unwrap_or((&request.instructions, ""));
        let instructions = if protocol.is_empty() {
            instructions.to_owned()
        } else {
            instructions.replacen(
                BASE_INSTRUCTIONS,
                &format!("{BASE_INSTRUCTIONS}\nUse begin_task{protocol}"),
                1,
            )
        };
        let block = ContextBlock {
            source: ContextSource::AppContract,
            priority: 0,
            budget: instructions.chars().count(),
            text: instructions,
        };
        let assemble = |input: &[HistoryItem]| -> grokbot_engine::contracts::Result<ModelRequest> {
            let current = input
                .last()
                .ok_or_else(|| Error::InvalidInput("empty run input".into()))?;
            let context_request = ContextRequest {
                run_id: run.run_id.clone(),
                trigger: run.trigger.clone(),
                bot_id: run.bot_id.clone(),
                conversation_id: run.conversation_id.clone(),
                current_message: serde_json::from_value(redact::redact_value(
                    &vault,
                    &serde_json::to_value(current)?,
                )?)?,
            };
            let assembled = assembler::assemble_input_for_entries(
                &context_request,
                std::slice::from_ref(&block),
                input,
                summary.as_ref(),
                reset.as_deref(),
                &budget,
                &sending_entry_ids,
            )?;
            let mut outgoing = request.clone();
            outgoing.instructions = assembled
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            outgoing.input = assembled.input;
            Ok(serde_json::from_value(redact::redact_value(
                &vault,
                &serde_json::to_value(outgoing)?,
            )?)?)
        };
        let group = super::multibot::managed(self.connection, self.run_id);
        let group_error = "Le fil dépasse le budget de contexte disponible. Ouvrez une conversation plus courte ; aucun ancien message n’a été supprimé ni résumé automatiquement.";
        let outgoing = match assemble(&request.input) {
            Ok(outgoing) if group && outgoing.input.len() != request.input.len() => {
                self.connection.execute(
                    "UPDATE wrapper_engine_runs SET error=?2,error_kind='context' WHERE run_id=?1",
                    params![self.run_id, group_error],
                )?;
                return emit(ModelEvent::Failed {
                    reason: group_error.into(),
                });
            }
            Ok(outgoing) => outgoing,
            Err(_) if group => {
                self.connection.execute(
                    "UPDATE wrapper_engine_runs SET error=?2,error_kind='context' WHERE run_id=?1",
                    params![self.run_id, group_error],
                )?;
                return emit(ModelEvent::Failed {
                    reason: group_error.into(),
                });
            }
            Err(error) if self.degrade_image_budget(&error, has_sending_bytes)? => {
                return self.stream(&request, emit);
            }
            Err(error) => return Err(error),
        };
        if compaction::revision(self.connection, &run.conversation_id)? != self.source_revision {
            return Err(Error::Conflict("context source changed".into()));
        }
        let gateway = ProviderGateway {
            provider: self.provider,
            cancel: self.cancel,
            metadata: Some((self.path, self.run_id)),
        };
        let mut deltas = RedactedDeltas::new(&vault)?;
        let mut forward = |event| {
            if let ModelEvent::TextDelta { text } = event {
                deltas.pending.push_str(&text);
                return deltas.flush(false, emit);
            }
            if matches!(
                event,
                ModelEvent::Completed | ModelEvent::Failed { .. } | ModelEvent::Incomplete { .. }
            ) {
                deltas.flush(true, emit)?;
            }
            let event = serde_json::from_value(redact::redact_value(
                &vault,
                &serde_json::to_value(event)?,
            )?)?;
            emit(event)
        };
        let has_images = outgoing
            .input
            .iter()
            .any(|entry| entry.images.iter().any(|image| image.url.is_some()));
        let mut image_fallback = None;
        let mut progressed = false;
        gateway.stream(&outgoing, &mut |event| {
            if let ModelEvent::Failed { reason } = &event {
                // Do not downgrade auth/usage/network errors, unrelated settings,
                // or any attempt that already emitted text/tool calls.
                if has_images && !progressed {
                    let classification = if super::attachments::model_lacks_vision(reason) {
                        Some(ImageFallback::NoVision)
                    } else if super::attachments::image_rejected(reason) {
                        Some(ImageFallback::RejectedFile)
                    } else {
                        None
                    };
                    if let Some(classification) = classification {
                        let kind: Option<String> = self.connection.query_row(
                            "SELECT error_kind FROM wrapper_engine_runs WHERE run_id=?1",
                            [self.run_id],
                            |row| row.get(0),
                        )?;
                        if kind.as_deref() == Some("other") {
                            image_fallback = Some(classification);
                            return Ok(());
                        }
                    }
                }
            }
            progressed |= !matches!(&event, ModelEvent::Failed { .. });
            forward(event)
        })?;
        let Some(image_fallback) = image_fallback else {
            return Ok(());
        };
        let (notice, degraded) = match image_fallback {
            ImageFallback::NoVision => {
                self.no_vision.store(true, Ordering::SeqCst);
                super::attachments::record_no_vision(self.connection, &request.model, now())
                    .map_err(engine_error)?;
                ("Ce modèle ne lit pas les images ; la réponse utilise le texte et les descriptions disponibles.", false)
            }
            ImageFallback::RejectedFile => {
                self.captions_only.store(true, Ordering::SeqCst);
                ("Une image jointe a été refusée ; seules les descriptions disponibles sont utilisées.", true)
            }
        };
        self.connection.execute(
            "UPDATE wrapper_engine_runs SET error=NULL,error_kind=NULL,notice=?2,images_degraded=MAX(images_degraded,?3) WHERE run_id=?1",
            params![self.run_id, notice, degraded],
        )?;
        if compaction::revision(self.connection, &run.conversation_id)? != self.source_revision {
            return Err(Error::Conflict("context source changed".into()));
        }
        let mut fallback = request.input.clone();
        for entry in &mut fallback {
            match image_fallback {
                ImageFallback::NoVision => super::attachments::without_vision(&mut entry.images),
                ImageFallback::RejectedFile => super::attachments::captions_only(&mut entry.images),
            }
        }
        // One retry only, through the same fencing, budget, secret redaction,
        // cancellation and metadata path. Further failures are real failures.
        gateway.stream(&assemble(&fallback)?, &mut forward)
    }
}
/// Retain the longest known secret's suffix across deltas BEFORE persisting or
/// displaying text. Snapshot values are never model-facing and survive rotation
/// for this stream, so removing a vault value cannot reveal its buffered tail.
struct RedactedDeltas<'a> {
    vault: &'a Vault,
    pending: String,
    known: Vec<String>,
    tail: usize,
}
impl<'a> RedactedDeltas<'a> {
    fn new(vault: &'a Vault) -> grokbot_engine::contracts::Result<Self> {
        let mut state = Self {
            vault,
            pending: String::new(),
            known: vec![],
            tail: 0,
        };
        state.refresh()?;
        Ok(state)
    }
    fn refresh(&mut self) -> grokbot_engine::contracts::Result<()> {
        for metadata in self.vault.list()? {
            if let Some(value) = self.vault.get(&metadata.name)? {
                self.tail = self.tail.max(value.chars().count());
                if !self.known.contains(&value) {
                    self.known.push(value);
                }
            }
        }
        self.known.sort_by_key(|v| std::cmp::Reverse(v.len()));
        Ok(())
    }
    fn flush(
        &mut self,
        finished: bool,
        emit: &mut dyn FnMut(ModelEvent) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        self.refresh()?;
        self.pending = redact::redact(self.vault, &self.pending)?;
        loop {
            let old = self.pending.clone();
            for value in &self.known {
                self.pending = self.pending.replace(value, "[...]");
            }
            if self.pending == old {
                break;
            }
        }
        let kept = if finished { 0 } else { self.tail };
        let count = self.pending.chars().count().saturating_sub(kept);
        let boundary = self
            .pending
            .char_indices()
            .nth(count)
            .map_or(self.pending.len(), |(n, _)| n);
        let text = self.pending[..boundary].to_owned();
        self.pending.drain(..boundary);
        if !text.is_empty() {
            emit(ModelEvent::TextDelta { text })?;
        }
        Ok(())
    }
}
fn event_vault(conn: &Connection) -> grokbot_engine::contracts::Result<Vault> {
    let path = conn
        .path()
        .ok_or_else(|| Error::InvalidInput("file database required".into()))?;
    vault(Path::new(path))
}
struct ProviderGateway<'a> {
    provider: &'a Provider,
    cancel: &'a AtomicBool,
    metadata: Option<(&'a Path, &'a str)>,
}
impl ModelGateway for ProviderGateway<'_> {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        if request.model == "mcp-local" {
            return emit(ModelEvent::Completed);
        }
        if request.model == "simulated" {
            return crate::gateway::SimulatedGateway.stream(request, emit);
        }
        self.provider.stream_run(request,self.cancel,&mut |notice,failure| {
            if let Some((path,id))=self.metadata {
                let conn=database(path)?;
                let vault=vault(path)?;
                let error=failure.map(|f|redact::redact(&vault,&f.message)).transpose()?;
                conn.execute("UPDATE wrapper_engine_runs SET notice=COALESCE(?2,notice),error=COALESCE(?3,error),error_kind=COALESCE(?4,error_kind) WHERE run_id=?1",params![id,notice,error,failure.map(|f|f.kind)])?;
            }
            Ok(())
        },emit)
    }
}
/// Projection deliberately differs from replay: Stop, failure and duration-budget
/// exhaustion keep the last visible partial response; a crashed response does not
/// contaminate the next model call or duplicate a recovered reply (loop r2 §6).
fn projected_text(conn: &Connection, run: &Run) -> grokbot_engine::contracts::Result<String> {
    Ok(projected_parts(conn, run)?
        .into_iter()
        .map(|part| part.body)
        .collect())
}

#[derive(Serialize)]
struct ReplyPart {
    body: String,
    before: Option<String>,
}

/// Keep the text on either side of a call separate, without exposing raw events
/// (notably opaque reasoning). Anchors refer to existing transcript cards, so
/// historical runs get the same ordering as live runs without rewriting storage.
fn projected_parts(
    conn: &Connection,
    run: &Run,
) -> grokbot_engine::contracts::Result<Vec<ReplyPart>> {
    let events = store::events(conn, &run.run_id, 0)?;
    let vault = event_vault(conn)?;
    let completed: HashSet<_> = events
        .iter()
        .filter(|e| matches!(e.kind, RunEventKind::ToolCall | RunEventKind::TurnEnded))
        .filter_map(|e| e.payload["response"].as_u64())
        .collect();
    let last = events
        .iter()
        .rev()
        .find(|e| e.kind == RunEventKind::TextDelta)
        .and_then(|e| e.payload["response"].as_u64());
    let last_interrupted = events
        .iter()
        .rfind(|e| e.kind == RunEventKind::Interrupted)
        .map_or(0, |e| e.seq);
    let mut parts = Vec::new();
    let mut text = String::new();
    for event in &events {
        let anchor = event.kind == RunEventKind::ToolCall
            || (run.kind == RunKind::Task
                && matches!(
                    event.kind,
                    RunEventKind::TaskChanged
                        | RunEventKind::Suspended
                        | RunEventKind::TurnEnded
                        | RunEventKind::Cancelled
                ));
        if anchor && !text.is_empty() {
            parts.push(ReplyPart {
                body: redact::redact(&vault, &std::mem::take(&mut text))?,
                before: Some(format!("{}:{}", run.run_id, event.seq)),
            });
        }
        if event.kind != RunEventKind::TextDelta {
            continue;
        }
        let response = event.payload["response"].as_u64();
        let visible = response.is_some_and(|r| completed.contains(&r))
            || (response == last
                && (run.status == RunStatus::Interrupted || event.seq > last_interrupted)
                && run.turn_end != Some(TurnEnd::Superseded));
        if visible {
            text.push_str(event.payload["text"].as_str().unwrap_or_default());
        }
    }
    if !text.is_empty() {
        parts.push(ReplyPart {
            body: redact::redact(&vault, &text)?,
            before: None,
        });
    }
    // A secret added after these events may straddle two rendered fragments.
    // Preserve the old whole-reply redaction guarantee, even in that rare case.
    let joined: String = parts.iter().map(|part| part.body.as_str()).collect();
    let safe = redact::redact(&vault, &joined)?;
    if safe != joined {
        return Ok(vec![ReplyPart {
            body: safe,
            before: parts.last().and_then(|part| part.before.clone()),
        }]);
    }
    Ok(parts)
}
fn run_reply(
    conn: &Connection,
    run: &Run,
    id: String,
    proposals: Vec<Proposal>,
) -> Result<Message, ApiError> {
    let parts = projected_parts(conn, run)?;
    let body = parts.iter().map(|part| part.body.as_str()).collect();
    let mut metadata: (Option<String>, Option<String>, Option<String>) = conn.query_row(
        "SELECT error,error_kind,notice FROM wrapper_engine_runs WHERE run_id=?1",
        [&run.run_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if context_budget_failed(conn, run)? {
        metadata.0 = Some(CONTEXT_TOO_LARGE.into());
        metadata.1 = Some("context_budget".into());
    }
    let (state, error) = match run.status {
        RunStatus::Queued | RunStatus::Running => ("streaming", None),
        RunStatus::Interrupted if super::multibot::is_group(conn, &run.conversation_id) => ("failed", Some("La génération distante a été interrompue. Son résultat est incertain ; elle n’a pas été relancée automatiquement.".into())),
        RunStatus::Interrupted => ("failed", Some(INTERRUPTED.into())),
        RunStatus::Failed if metadata.1.as_deref() == Some("incomplete") => ("echoed", None),
        RunStatus::Failed => ("failed", metadata.0.or_else(|| Some(INTERRUPTED.into()))),
        RunStatus::Cancelled => ("stopped", None),
        _ => ("echoed", None),
    };
    Ok(Message {
        id,
        role: "assistant",
        body,
        state,
        created_at: run.started_at.max(0) as u64,
        error,
        error_kind: metadata.1,
        notice: metadata.2,
        proposals,
        kind: "message".into(),
        payload: json!({"run_id":run.run_id,"bot_id":run.bot_id,"parts":parts,"usage":store::token_usage(conn, &run.run_id)?}),
        attachments: vec![],
    })
}
pub(crate) fn group_reply(ctx: &super::Context<'_>, id: &str) -> Result<Message, ApiError> {
    let run = store::required(ctx.connection, id)?;
    run_reply(ctx.connection, &run, format!("reply-{id}"), vec![])
}
pub fn reply(
    ctx: &super::Context<'_>,
    conversation: &str,
    message: &str,
    proposals: Vec<Proposal>,
) -> Result<Option<Message>, ApiError> {
    let Some(run) =
        store::latest_run_for_trigger(ctx.connection, &user_trigger(conversation, message, 0))?
            .filter(|r| r.conversation_id == conversation)
    else {
        return Ok(None);
    };
    Ok(Some(run_reply(
        ctx.connection,
        &run,
        format!("reply-{message}"),
        proposals,
    )?))
}
pub fn entry(ctx: &super::Context<'_>, kind: &str, id: &str) -> Option<Message> {
    if kind == "engine_reply" {
        let run = store::get(ctx.connection, id).ok()??;
        return run_reply(ctx.connection, &run, format!("reply-{id}"), vec![]).ok();
    }
    if matches!(kind, "reset" | "engine_reset") {
        return Some(Message {
            id: id.into(),
            role: "assistant",
            body: "Nouvelle conversation — la mémoire est conservée.".into(),
            state: "echoed",
            created_at: ctx.now_ms,
            error: None,
            error_kind: None,
            notice: None,
            proposals: vec![],
            kind: "reset".into(),
            payload: json!({"reset":true}),
            attachments: vec![],
        });
    }
    if kind == "engine_summary" {
        let record: String = ctx
            .connection
            .query_row(
                "SELECT record FROM engine_summaries WHERE summary_id=?1 AND record IS NOT NULL",
                [id],
                |r| r.get(0),
            )
            .optional()
            .ok()??;
        let summary: Summary = serde_json::from_str(&record).ok()?;
        return Some(card(
            id,
            "summary",
            summary.text.clone(),
            serde_json::to_value(summary).ok()?,
            ctx.now_ms,
        ));
    }
    if !matches!(kind, "engine_tool" | "engine_task") {
        return None;
    }
    let (run_id, seq) = id.rsplit_once(':')?;
    let seq: u64 = seq.parse().ok()?;
    let run = store::get(ctx.connection, run_id).ok()??;
    if ctx
        .conversation_id
        .is_some_and(|id| id != run.conversation_id)
    {
        return None;
    }
    let event = store::events(ctx.connection, run_id, seq.checked_sub(1)?)
        .ok()?
        .into_iter()
        .next()?;
    if kind == "engine_task" {
        return Some(card(
            id,
            "task",
            String::new(),
            json!({"run_id":run_id,"task_status":run.task_status,"turn_end":run.turn_end,"status":run.status,
                "event_kind":event.kind,"routine":matches!(run.trigger, Trigger::Occurrence { .. })}),
            run.started_at.max(0) as u64,
        ));
    }
    let call_id = event.payload["call_id"].as_str()?;
    let result = store::events(ctx.connection, run_id, seq)
        .ok()?
        .into_iter()
        .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == call_id);
    let call = serde_json::from_value::<grokbot_engine::contracts::ToolCall>(event.payload.clone())
        .ok()?;
    let mut admission = result
        .as_ref()
        .and_then(|e| e.payload.get("authorized_by"))
        .cloned()
        .or_else(|| {
            run_loop::external_authorization(ctx.connection, run_id, &call)
                .ok()
                .flatten()
                .map(|by| json!(by))
        });
    let requester: String = ctx
        .connection
        .query_row(
            "SELECT name FROM wrapper_bots WHERE account_id=?1 AND id=?2",
            params![ACCOUNT, run.bot_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| run.bot_id.clone());
    let mut outcome = result.as_ref().map(|e| e.payload["outcome"].clone());
    // A cancellation settles the engine call immediately, but the process owner
    // may finish draining its bounded output a few milliseconds later.
    if event.payload["name"]
        .as_str()
        .is_some_and(super::local::is_tool)
    {
        let stored: Option<(Option<String>,Option<String>)> = ctx
            .connection
            .query_row(
                "SELECT outcome,authorized_by FROM engine_external_tools WHERE run_id=?1 AND call_id=?2",
                params![run_id, call_id],
                |r| Ok((r.get(0)?,r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        if let Some((stored, authority)) = stored {
            if let Some(value) = stored.and_then(|s| serde_json::from_str(&s).ok()) {
                outcome = Some(value);
            }
            if let Some(value) = authority.and_then(|s| serde_json::from_str(&s).ok()) {
                admission = Some(value);
            }
        }
    }
    let mut display_call = event.payload;
    if display_call["name"] == "propose_skill" {
        // Keep canonical skill content in its lazy approval, not every state row.
        display_call.as_object_mut()?.remove("arguments");
    }
    Some(card(
        id,
        "tool",
        String::new(),
        json!({"run_id":run_id,"run_status":run.status,"call":display_call,"outcome":outcome,
            "authorized_by":admission,"requesting_bot":requester,
            "approval_status":super::forms::tool_approval_status(ctx,run_id,&call),
            "connector":super::mcp::call_display(ctx,&call.name,&call.arguments)}),
        run.started_at.max(0) as u64,
    ))
}
fn card(id: &str, kind: &str, body: String, payload: Value, created_at: u64) -> Message {
    Message {
        id: id.into(),
        role: "assistant",
        body,
        state: "echoed",
        created_at,
        error: None,
        error_kind: None,
        notice: None,
        proposals: vec![],
        kind: kind.into(),
        payload,
        attachments: vec![],
    }
}
pub fn state(ctx: &super::Context<'_>) -> Value {
    let runs = [
        RunStatus::Queued,
        RunStatus::Running,
        RunStatus::Interrupted,
        RunStatus::AwaitingApproval,
        RunStatus::AwaitingHumanInput,
        RunStatus::AwaitingChildren,
    ]
    .into_iter()
    .flat_map(|s| store::runs_with_status(ctx.connection, s).unwrap_or_default())
    .collect::<Vec<_>>();
    json!({"runs":runs,"context_budget":budget(ctx.connection).ok()})
}
pub fn on_run_event(_: &super::Context<'_>, _: &RunEvent) {}
pub fn context(ctx: &super::Context<'_>, request: &ContextRequest) -> Vec<ContextBlock> {
    let block = |source, priority, text: String| ContextBlock {
        source,
        priority,
        // Private instructions and the host's delegated-task contract share
        // this category cap; a short private instruction must not erase it.
        budget: if source == ContextSource::BotInstructions {
            16_384
        } else {
            text.chars().count()
        },
        text,
    };
    let mut blocks = vec![block(
        ContextSource::AppContract,
        super::priority::APP,
        super::multibot::contract(ctx)
            .map(|contract| format!("{BASE_INSTRUCTIONS}\n\n{contract}"))
            .unwrap_or_else(|| BASE_INSTRUCTIONS.into()),
    )];
    if let Ok(Some(bot))=ctx.connection.query_row("SELECT id,name,color,description,instructions,template,customized,deleted,shape,created_at FROM wrapper_bots WHERE account_id=?1 AND id=?2",params![ACCOUNT,request.bot_id],bot_from_row).optional(){
        let mut identity=format!("Tu t'appelles {}.",bot.name);
        if !bot.description.is_empty(){identity.push_str(&format!("\n\nTon rôle, tel que les autres le voient : {}",bot.description));}
        blocks.push(block(ContextSource::BotIdentity,super::priority::IDENTITY,identity));
        if !bot.instructions.is_empty(){blocks.push(block(ContextSource::BotInstructions,super::priority::INSTRUCTIONS,format!("Consignes privées sur ta façon de te comporter :\n{}",bot.instructions)));}
    }
    blocks.push(block(
        ContextSource::CurrentMessage,
        super::priority::CURRENT,
        request.current_message.text.clone(),
    ));
    blocks
}
/// Compatibility probe: capability-owned RAW blocks, ready for one assembly pass.
/// Keep queued user text and monotone priorities; callers inspect memory/skill JSON.
pub fn context_preview(app: &App, conversation_id: &str) -> Result<Vec<ContextBlock>, ApiError> {
    app.require_conversation(conversation_id)?;
    let conversation = app
        .conversations()?
        .into_iter()
        .find(|c| c.id == conversation_id)
        .ok_or_else(|| ApiError::new(404, "Conversation introuvable."))?;
    let current = conversation
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user");
    let id = current.map_or("preview", |m| m.id.as_str());
    let request = ContextRequest {
        run_id: "preview".into(),
        trigger: user_trigger(conversation_id, id, 0),
        bot_id: conversation.bot_id.clone(),
        conversation_id: conversation_id.into(),
        current_message: item(
            id,
            Role::User,
            current.map_or_else(String::new, |m| m.body.clone()),
        ),
    };
    let mut ctx = app.feature_context();
    ctx.bot_id = Some(&conversation.bot_id);
    ctx.conversation_id = Some(conversation_id);
    let mut blocks = super::context(&ctx, &request);
    let mut lines = Vec::new();
    for message in &conversation.messages {
        if message.id == id {
            break;
        }
        if matches!(message.kind.as_str(), "reset" | "engine_reset") {
            lines.clear();
            continue;
        }
        if message.kind == "message"
            && ((message.role == "user" && matches!(message.state, "accepted" | "echoed"))
                || (message.role == "assistant" && message.state != "failed"))
            && !message.body.is_empty()
        {
            lines.push(format!("{}: {}", message.role, message.body));
        }
    }
    if !lines.is_empty() {
        let text = lines.join("\n\n");
        blocks.push(ContextBlock {
            source: ContextSource::History,
            priority: super::priority::HISTORY,
            budget: text.chars().count(),
            text,
        });
    }
    blocks.sort_by_key(|b| b.priority);
    Ok(blocks)
}
/// Assembled diagnostic view, deliberately separate from the raw feature probe.
#[doc(hidden)]
pub fn request_preview(app: &App, conversation_id: &str) -> Result<Vec<ContextBlock>, ApiError> {
    app.require_conversation(conversation_id)?;
    let bot: String = app.connection.query_row(
        "SELECT bot_id FROM wrapper_conversations WHERE account_id=?1 AND id=?2",
        params![ACCOUNT, conversation_id],
        |r| r.get(0),
    )?;
    let latest:Option<String>=app.connection.query_row("SELECT message_id FROM wrapper_messages WHERE account_id=?1 AND conversation_id=?2 AND kind='submission' AND ordinal>COALESCE((SELECT MAX(ordinal) FROM wrapper_messages WHERE account_id=?1 AND conversation_id=?2 AND kind IN ('reset','engine_reset')),-1) ORDER BY ordinal DESC LIMIT 1",params![ACCOUNT,conversation_id],|r|r.get(0)).optional()?;
    let text=latest.as_deref().and_then(|id|app.connection.query_row("SELECT body FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3",params![ACCOUNT,conversation_id,id],|r|r.get(0)).optional().ok().flatten()).unwrap_or_default();
    let current = item(latest.as_deref().unwrap_or("preview"), Role::User, text);
    let run = Run {
        run_id: "preview".into(),
        bot_id: bot.clone(),
        conversation_id: conversation_id.into(),
        trigger: user_trigger(conversation_id, &current.entry_id, 0),
        kind: RunKind::Chat,
        status: RunStatus::Queued,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: now() as i64,
    };
    let mut request = ContextRequest {
        run_id: run.run_id.clone(),
        trigger: run.trigger.clone(),
        bot_id: bot.clone(),
        conversation_id: conversation_id.into(),
        current_message: current.clone(),
    };
    let mut ctx = app.feature_context();
    ctx.bot_id = Some(&bot);
    ctx.conversation_id = Some(conversation_id);
    let mut blocks = super::context(&ctx, &request);
    blocks.retain(|b| {
        b.source != ContextSource::CurrentMessage
            && !(b.source == ContextSource::ImageCaption
                && b.text.starts_with("<untrusted-image-captions>"))
    });
    let model = app
        .resolved_model()?
        .map(|(model, _, _)| model)
        .or(app.model_preferences()?.0)
        .unwrap_or_else(|| "simulated".into());
    let linked = super::attachments::images_for_message(
        &ctx,
        conversation_id,
        &current.entry_id,
        &model,
        true,
    )?;
    if !linked.is_empty() {
        request.current_message.images = linked;
    }
    if let Some(images) = take_image_inputs(&mut blocks)? {
        request.current_message.images = images;
    }
    let (mut history, reset) = history(&app.connection, &run, &current)?;
    request
        .current_message
        .text
        .push_str(&super::attachments::document_text(
            &ctx,
            conversation_id,
            &current.entry_id,
        )?);
    hydrate_history_images(&ctx, conversation_id, &model, &mut history)?;
    Ok(assembler::assemble(
        &request,
        &blocks,
        &history,
        None,
        reset.as_deref(),
        &budget(&app.connection)?,
    )?)
}
fn retitle(
    conn: &Connection,
    provider: &Provider,
    id: &str,
    cancel: &AtomicBool,
    path: &Path,
) -> grokbot_engine::contracts::Result<()> {
    let run = store::required(conn, id)?;
    let group = super::multibot::managed(conn, id);
    if group {
        // One title from the first completed group turn, not twelve competing
        // title requests. Existing direct conversations keep their own policy.
        let first: Option<String> = conn.query_row("SELECT t.run_id FROM wrapper_group_turns t JOIN engine_runs r ON r.run_id=t.run_id WHERE t.account_id=?1 AND t.conversation_id=?2 AND r.status='finished' ORDER BY t.ordinal LIMIT 1",params![ACCOUNT,run.conversation_id],|r|r.get(0)).optional()?;
        let new_group: bool = conn.query_row("SELECT NOT EXISTS(SELECT 1 FROM wrapper_messages m WHERE m.account_id=?1 AND m.conversation_id=?2 AND m.kind='submission' AND NOT EXISTS(SELECT 1 FROM wrapper_group_inputs i WHERE i.account_id=m.account_id AND i.conversation_id=m.conversation_id AND i.message_id=m.message_id AND i.group_mode=1))",params![ACCOUNT,run.conversation_id],|r|r.get(0))?;
        if first.as_deref() != Some(id) || !new_group {
            return Ok(());
        }
    }
    if run.status != RunStatus::Finished || cancel.load(Ordering::SeqCst) {
        return Ok(());
    }
    let Trigger::UserMessage { attempt: 0, .. } = run.trigger else {
        return Ok(());
    };
    let current = run_input(conn, id)?;
    let history = if group {
        super::multibot::snapshot(conn, id)?.unwrap_or_default()
    } else {
        history(conn, &run, &current)?.0
    };
    let turns = history.iter().filter(|i| i.role == Role::User).count() + 1;
    if !group && !matches!(turns, 1 | 3) {
        return Ok(());
    }
    let automatic:bool=conn.query_row("SELECT title_customized=0 AND deleted=0 FROM wrapper_conversations WHERE account_id=?1 AND id=?2",params![ACCOUNT,run.conversation_id],|r|r.get(0))?;
    if !automatic {
        return Ok(());
    }
    let claimed=conn.execute("UPDATE wrapper_engine_runs SET title_state='running' WHERE run_id=?1 AND title_state='none'",[id])?;
    if claimed == 0 {
        return Ok(());
    }
    let result = (|| -> grokbot_engine::contracts::Result<()> {
        let mut transcript = history;
        transcript.push(current);
        transcript.push(item("reply", Role::Assistant, projected_text(conn, &run)?));
        let text = transcript
            .iter()
            .filter(|i| !i.text.is_empty())
            .map(|i| {
                format!(
                    "{} : {}",
                    if i.role == Role::User {
                        "Utilisateur"
                    } else {
                        "Assistant"
                    },
                    i.text.chars().take(700).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = redact::redact(&vault(path)?, &text)?;
        let model = conn.query_row(
            "SELECT model FROM wrapper_engine_runs WHERE run_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let mut title = String::new();
        let mut completed = false;
        ProviderGateway{provider,cancel,metadata:None}.stream(&ModelRequest{model,instructions:"Tu écris le titre d'une conversation pour la retrouver facilement dans une liste. Réponds seulement par le titre : 3 à 6 mots, dans la langue de l'utilisateur, sans guillemets, sans point final, sans mise en forme. Le titre nomme le sujet précis dont on parle, jamais une salutation ni une formule vague.".into(),input:vec![item("title",Role::User,text)],tools:vec![],effort:None,service_tier:None,prompt_cache_key:None},&mut |event|{
            match event {
                ModelEvent::TextDelta{text}=>title.push_str(&text),
                ModelEvent::Completed=>completed=true,
                ModelEvent::Reasoning{..}|ModelEvent::Usage{..}=>{},
                _=>return Err(Error::InvalidInput("title generation incomplete".into())),
            }
            Ok(())
        })?;
        if !completed {
            return Err(Error::InvalidInput("title generation incomplete".into()));
        }
        let title = redact::redact(&vault(path)?, &title)?;
        if let Some(title) = clean_title(&title) {
            conn.execute("UPDATE wrapper_conversations SET title=?3 WHERE account_id=?1 AND id=?2 AND title_customized=0 AND deleted=0",params![ACCOUNT,run.conversation_id,title])?;
        }
        Ok(())
    })();
    conn.execute(
        "UPDATE wrapper_engine_runs SET title_state='done' WHERE run_id=?1",
        [id],
    )?;
    result
}
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().find(|s| !s.trim().is_empty())?;
    let line=line.chars().map(|c|if c.is_whitespace(){' '}else{c}).filter(|c|!c.is_control() && !matches!(*c,'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')).collect::<String>();
    let line = line
        .trim()
        .trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\'' | '«' | '»' | '“' | '”' | '`' | '*' | '#' | '_'
                )
        })
        .trim_end_matches(['.', ':', ' ']);
    let title = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        None
    } else if title.chars().count() > 60 {
        Some(format!(
            "{}…",
            title.chars().take(59).collect::<String>().trim_end()
        ))
    } else {
        Some(title)
    }
}
pub(crate) struct LocalTransport<'a> {
    pub(crate) connection: &'a Connection,
    pub(crate) lose_reply: bool,
    #[cfg(test)]
    pub(crate) live: bool,
    pub(crate) error: Option<ApiError>,
}
impl LocalTransport<'_> {
    fn accept(&self, request: &SendRequest) -> Result<SendOutcome, ApiError> {
        #[cfg(test)]
        let _ = self.live;
        let account = request.actor().account_id.as_str();
        let conversation = request.conversation_id().as_str();
        let message = request.message_id().as_str();
        let tx = super::write_transaction(self.connection)?;
        let existing:Option<String>=tx.query_row("SELECT body FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3",params![account,conversation,message],|r|r.get(0)).optional()?;
        if existing.is_some_and(|body| body != request.body()) {
            return Ok(SendOutcome::Refused(Refusal::Conflict));
        }
        tx.execute("INSERT INTO wrapper_remote(account_id,conversation_id,message_id,body,reply,created_at,send_count) VALUES (?1,?2,?3,?4,'',?5,1) ON CONFLICT(account_id,conversation_id,message_id) DO UPDATE SET send_count=send_count+1",params![account,conversation,message,request.body(),now()])?;
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
        let found:rusqlite::Result<bool>=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_remote WHERE account_id=?1 AND conversation_id=?2 AND message_id=?3)",params![request.actor().account_id.as_str(),request.conversation_id().as_str(),request.message_id().as_str()],|r|r.get(0));
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
struct RunGateway<'a> {
    path: &'a Path,
    provider: &'a Provider,
    run_id: &'a str,
    cancel: &'a AtomicBool,
    image_urls: Mutex<HashMap<String, String>>,
    no_vision: AtomicBool,
    captions_only: AtomicBool,
}
impl ModelGateway for RunGateway<'_> {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        let conn = database(self.path)?;
        for _ in 0..3 {
            let run = store::required(&conn, self.run_id)?;
            // Snapshot BEFORE reading capability context/history/replay. Forgetting
            // bumps this revision; stale summaries and requests are discarded.
            let source_revision = compaction::revision(&conn, &run.conversation_id)?;
            let mut services = Services {
                path: self.path,
                provider: self.provider,
                connection: &conn,
                cancel: Arc::new(AtomicBool::new(false)),
                bot_id: Some(run.bot_id.clone()),
                host_mcp: false,
            };
            let mut fresh = services.context(&run)?;
            fresh.input.extend(replay(&conn, &run)?);
            fresh.tools = request
                .tools
                .iter()
                .filter(|tool| {
                    !tool.name.starts_with("mcp_start_") && !tool.name.starts_with("mcp_remove_")
                })
                .cloned()
                .collect();
            if let Some((_, protocol)) = request.instructions.rsplit_once("\nUse begin_task") {
                fresh
                    .instructions
                    .push_str(&format!("\nUse begin_task{protocol}"));
            }
            let result = RunGatewayCall {
                path: self.path,
                provider: self.provider,
                connection: &conn,
                run_id: self.run_id,
                cancel: self.cancel,
                source_revision,
                image_urls: &self.image_urls,
                no_vision: &self.no_vision,
                captions_only: &self.captions_only,
            }
            .stream(&fresh, emit);
            if !matches!(&result, Err(Error::Conflict(_))) {
                return result;
            }
        }
        Err(Error::InvalidInput(
            "context changed repeatedly; retry this response".into(),
        ))
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    struct Fixture {
        app: App,
        dir: PathBuf,
        run: Run,
    }
    impl Fixture {
        fn new(trigger: Trigger) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "grokbot-card-projection-{}-{:?}-{}",
                std::process::id(),
                std::thread::current().id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let app = App::open(&dir.join("workspace.sqlite")).unwrap();
            app.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            let mut run = Run {
                run_id: "projection".into(),
                bot_id: DEFAULT_BOT.id.into(),
                conversation_id: "cards".into(),
                trigger,
                kind: RunKind::Task,
                status: RunStatus::Queued,
                task_status: Some(TaskStatus::Working),
                turn_end: None,
                budgets: RunBudgets::default(),
                started_at: 1,
            };
            model::start(&app.connection, &run).unwrap();
            run = model::set_running_at(&app.connection, &run.run_id, 2).unwrap();
            app.connection.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES ('projection','{}',0,'simulated')", []).unwrap();
            Self { app, dir, run }
        }
        fn event(&mut self, kind: RunEventKind, payload: Value) -> u64 {
            match kind {
                RunEventKind::TurnEnded => {
                    self.run.status = RunStatus::Finished;
                    self.run.turn_end = Some(TurnEnd::Finished);
                }
                RunEventKind::Interrupted => self.run.status = RunStatus::Interrupted,
                _ if self.run.status == RunStatus::Interrupted => {
                    self.run.status = RunStatus::Running
                }
                _ => {}
            }
            store::save(&self.app.connection, &self.run).unwrap();
            store::append_event(&self.app.connection, &self.run.run_id, kind, &payload)
                .unwrap()
                .seq
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.app.connection.execute_batch("ROLLBACK");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn reply_fragments_follow_calls_and_keep_the_transport_body() {
        let mut f = Fixture::new(user_trigger("cards", "question", 0));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":0,"text":"Avant. "}),
        );
        let call = f.event(RunEventKind::ToolCall, json!({"response":0,"call_id":"a","name":"remember","arguments":{},"reasoning":["opaque-never-render"]}));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":1,"text":"Entre. "}),
        );
        let next = f.event(
            RunEventKind::ToolCall,
            json!({"response":1,"call_id":"b","name":"complete_task","arguments":{}}),
        );
        f.event(RunEventKind::TaskChanged, json!({}));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":2,"text":"Après."}),
        );
        let end = f.event(
            RunEventKind::TurnEnded,
            json!({"response":2,"turn_end":"finished"}),
        );
        f.run.status = RunStatus::Finished;
        f.run.turn_end = Some(TurnEnd::Finished);
        let reply = run_reply(&f.app.connection, &f.run, "reply-question".into(), vec![]).unwrap();
        assert_eq!(reply.body, "Avant. Entre. Après.");
        assert_eq!(
            reply.payload["parts"],
            json!([
                {"body":"Avant. ","before":format!("projection:{call}")},
                {"body":"Entre. ","before":format!("projection:{next}")},
                {"body":"Après.","before":format!("projection:{end}")}
            ])
        );
        assert!(!reply.payload.to_string().contains("opaque-never-render"));
    }

    #[test]
    fn crashed_text_is_not_reintroduced_by_fragment_projection() {
        let mut f = Fixture::new(user_trigger("cards", "question", 0));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":0,"text":"Crashed"}),
        );
        f.event(RunEventKind::Interrupted, json!({}));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":1,"text":"Recovered"}),
        );
        f.event(RunEventKind::TurnEnded, json!({"response":1}));
        f.run.status = RunStatus::Finished;
        f.run.turn_end = Some(TurnEnd::Finished);
        assert_eq!(
            projected_text(&f.app.connection, &f.run).unwrap(),
            "Recovered"
        );
    }

    #[test]
    fn newly_registered_secret_spanning_fragments_stays_redacted() {
        let mut f = Fixture::new(user_trigger("cards", "question", 0));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":0,"text":"sensitive-"}),
        );
        f.event(RunEventKind::ToolCall, json!({"response":0,"call_id":"a"}));
        f.event(
            RunEventKind::TextDelta,
            json!({"response":1,"text":"value-123"}),
        );
        f.event(RunEventKind::TurnEnded, json!({"response":1}));
        vault(&f.dir.join("workspace.sqlite"))
            .unwrap()
            .set("QA_SECRET", "test", "sensitive-value-123")
            .unwrap();
        let reply = run_reply(&f.app.connection, &f.run, "reply-question".into(), vec![]).unwrap();
        assert_eq!(reply.body, "[...]");
        assert_eq!(reply.payload["parts"].as_array().unwrap().len(), 1);
        assert_eq!(reply.payload["parts"][0]["body"], "[...]");
    }

    #[test]
    fn routine_snapshot_is_identified_without_claiming_task_success() {
        let mut f = Fixture::new(Trigger::Occurrence {
            occurrence_id: "routine".into(),
        });
        f.run.status = RunStatus::Finished;
        f.run.turn_end = Some(TurnEnd::Finished);
        store::save(&f.app.connection, &f.run).unwrap();
        let end = f.event(RunEventKind::TurnEnded, json!({"response":0}));
        let mut ctx = f.app.feature_context();
        ctx.conversation_id = Some("cards");
        let card = entry(&ctx, "engine_task", &format!("projection:{end}")).unwrap();
        assert_eq!(card.payload["routine"], true);
        assert_eq!(card.payload["task_status"]["status"], "working");
        assert_eq!(card.payload["event_kind"], "turn_ended");
        let reply = run_reply(&f.app.connection, &f.run, "reply-question".into(), vec![]).unwrap();
        assert_eq!(reply.body, "");
        assert_eq!(reply.payload["parts"], json!([]));
    }
}

#[cfg(test)]
mod cache_key_tests {
    use super::prompt_cache_key;

    #[test]
    fn cache_key_is_bounded_and_hashes_the_entire_long_conversation_id() {
        let conversation = format!("task:child:routine:{}", "long-id-".repeat(1024));
        let key = prompt_cache_key(&conversation);
        assert!(key.is_ascii() && key.len() <= 64);
        assert!(key.starts_with("ezerbot-"));
        assert_eq!(key, prompt_cache_key(&conversation));
        assert_ne!(key, prompt_cache_key(&format!("{conversation}-other")));
        assert!(prompt_cache_key("short-conversation").len() <= 64);
    }
}

#[cfg(test)]
mod title_tests {
    use super::clean_title;
    #[test]
    fn titles_are_short_clean_and_control_free() {
        assert_eq!(
            clean_title("\n« Service de notifications en Rust. »\nAutre ligne").as_deref(),
            Some("Service de notifications en Rust")
        );
        assert_eq!(
            clean_title("**Recette   de\tpain**").as_deref(),
            Some("Recette de pain")
        );
        assert_eq!(clean_title("  \n \"\" "), None);
        assert_eq!(
            clean_title("« Su\u{0007}jet\u{200e}. »").as_deref(),
            Some("Sujet")
        );
        let long = clean_title(&"mot ".repeat(40)).unwrap();
        assert!(long.chars().count() <= 60 && long.ends_with('…'));
    }
}
