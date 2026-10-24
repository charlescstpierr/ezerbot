//! Read-only current work, independent of the unread notification outbox.
use super::Context;
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use grokbot_engine::{
    contracts::{ToolCall, ToolOutcome, ToolSpec, Trigger},
    store,
};
use grokbot_forms::redact::redact;
use rusqlite::Connection;
use serde_json::json;

empty_hooks!(state, tick, context, entry, on_run_event, link_attachments);

const QUESTION_TOOL: &str = "attendre_reponse";
const QUESTION_CHARACTERS: usize = 140;
const QUESTION_BOUND: usize = 500;

// The journal already holds every question: the tool call is an engine event (its
// arguments) and a timeline entry `run:seq`. The partial index finds those calls
// without reading every event; the thread index finds what follows a question
// (Charles's message, a reset) without reading what came before it.
pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS wrapper_question_calls ON engine_run_events(run_id,seq)
         WHERE kind='tool_call' AND json_extract(payload,'$.name')='attendre_reponse';
         CREATE INDEX IF NOT EXISTS wrapper_messages_thread ON wrapper_messages(account_id,conversation_id,ordinal)",
    )?;
    Ok(())
}

pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: QUESTION_TOOL.into(),
        description: "Signale que tu ne peux pas avancer sans la réponse de Charles : ta question apparaît dans « À décider » jusqu'à ce qu'il écrive. Une question posée seulement dans ton texte n'est pas signalée. N'appelle cet outil que si tu es bloqué, sinon avance. Pas dans une sous-tâche. Pose une question courte, puis termine ton tour.".into(),
        schema: json!({"type":"object","additionalProperties":false,"properties":{"question":{"type":"string","minLength":1,"maxLength":QUESTION_BOUND}},"required":["question"]}),
        requires_approval: false,
    }]
}

/// Nothing is stored here: `GET /api/taches` reads the question from the journal, so
/// the call must be refused when the journal would show a line Charles cannot answer.
pub fn call_tool(ctx: &Context<'_>, call: &ToolCall) -> Option<ToolOutcome> {
    if call.name != QUESTION_TOOL {
        return None;
    }
    let asked = call.arguments["question"].as_str().is_some_and(|question| {
        !question.trim().is_empty() && question.chars().count() <= QUESTION_BOUND
    });
    let delegated = ctx
        .run_id
        .and_then(|id| store::get(ctx.connection, id).ok().flatten())
        .is_some_and(|run| matches!(run.trigger, Trigger::Delegation { .. }));
    Some(if !asked {
        ToolOutcome::Failed {
            reason: format!("La question est obligatoire, de {QUESTION_BOUND} caractères au plus."),
        }
    } else if delegated {
        ToolOutcome::Failed { reason: "Une sous-tâche ne peut pas attendre Charles : termine-la avec complete_task en disant ce qui manque.".into() }
    } else {
        ToolOutcome::Done {
            output: json!({"status":"signalée","message":"Charles voit ta question. Termine ton tour."}),
        }
    })
}

fn shortened(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= QUESTION_CHARACTERS {
        return text.into();
    }
    let mut cut: String = text.chars().take(QUESTION_CHARACTERS - 1).collect();
    cut.push('…');
    cut
}

// Only identifiers, fixed labels and allowlisted lifecycle states cross this
// boundary. Never project objectives, commands, results, form fields or secrets.
// The one free text is the question of `attendre_reponse`; the route masks vault
// secrets in the whole question before cutting it, never the other way round.
// `questions` repeats the tool's test: not blank (the characters of `char::is_whitespace`), at most 500 characters.
// `live_runs` is the one definition of a visible run: a run of a delegation (its
// own, or a later turn in its task conversation, such as a late approval) is
// visible only through a chain of visible edges up to a run that was not delegated.
// It is computed once (MATERIALIZED), and the CROSS JOIN keeps SQLite walking from an
// edge to the runs of its conversation by index instead of scanning every run per edge.
// Public so the scale test can count the VM instructions of this exact statement.
pub const SNAPSHOT: &str = "
WITH RECURSIVE open_runs AS (
 SELECT r.*,json_extract(r.record,'$.started_at') AS started_at,
 CASE WHEN EXISTS (SELECT 1 FROM engine_run_wakes w WHERE w.run_id=r.run_id AND w.state='pending')
 THEN 'running' ELSE 'stopped' END AS wait_status FROM engine_runs r
 JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=r.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=r.bot_id AND b.deleted=0
), open_edges AS (
 SELECT d.* FROM delegation_graph d WHERE d.conversation_id IS NULL
 OR EXISTS (SELECT 1 FROM wrapper_conversations c WHERE c.account_id=?1 AND c.id=d.conversation_id AND c.deleted=0)
), root_runs AS (
 SELECT r.* FROM open_runs r WHERE r.trigger_kind<>'delegation'
 AND NOT EXISTS (SELECT 1 FROM delegation_graph g WHERE g.conversation_id=r.conversation_id)
), live_edges AS (
 SELECT d.*,p.conversation_id AS origin_conversation_id,p.started_at AS origin_started_at
 FROM open_edges d JOIN root_runs p ON p.run_id=d.parent_run_id
 UNION
 SELECT d.*,p.conversation_id,p.started_at
 FROM live_edges e JOIN open_runs p ON p.run_id=e.child_run_id
 JOIN open_edges d ON d.parent_run_id=p.run_id
 UNION
 SELECT d.*,p.conversation_id,p.started_at
 FROM live_edges e CROSS JOIN open_runs p ON p.conversation_id=e.conversation_id
 JOIN open_edges d ON d.parent_run_id=p.run_id
), live_runs AS MATERIALIZED (
 SELECT * FROM open_runs r WHERE r.run_id IN (SELECT run_id FROM root_runs UNION SELECT child_run_id FROM live_edges)
 OR r.conversation_id IN (SELECT conversation_id FROM live_edges)
), questions AS (
 SELECT r.bot_id,r.conversation_id,m.ordinal,m.created_at,json_extract(e.payload,'$.arguments.question') AS question,
 row_number() OVER (PARTITION BY r.conversation_id ORDER BY m.ordinal DESC) AS latest
 FROM engine_run_events e INDEXED BY wrapper_question_calls CROSS JOIN live_runs r ON r.run_id=e.run_id
 JOIN wrapper_messages m ON m.account_id=?1 AND m.conversation_id=r.conversation_id AND m.message_id=e.run_id||':'||e.seq AND m.kind='engine_tool'
 WHERE e.kind='tool_call' AND json_extract(e.payload,'$.name')='attendre_reponse'
 AND json_type(e.payload,'$.arguments.question')='text'
 AND trim(json_extract(e.payload,'$.arguments.question'),char(9,10,11,12,13,32,133,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288))<>''
 AND length(json_extract(e.payload,'$.arguments.question'))<=500
 AND r.trigger_kind<>'delegation'
), pending AS (
 SELECT q.id,q.run_id,q.entry_id,r.bot_id,c.id AS conversation_id,
 CASE r.trigger_kind WHEN 'delegation' THEN 'delegation' WHEN 'occurrence' THEN 'routine' ELSE 'task' END AS kind,
 CASE json_extract(q.kind_json,'$.kind') WHEN 'approval' THEN 'approval'
 WHEN 'secret' THEN 'secret' ELSE 'form' END AS request_kind,
 COALESCE(NULLIF(q.created_at,0),r.started_at) AS started_at
 FROM forms_requests q JOIN live_runs r ON r.run_id=q.run_id
 JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=json_extract(q.route_json,'$.target_conversation_id') AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=c.bot_id AND b.deleted=0
 WHERE q.status='pending' AND q.replaced=0
 AND (json_extract(q.kind_json,'$.expires_at') IS NULL OR json_extract(q.kind_json,'$.expires_at')>?2)
 AND (json_extract(q.kind_json,'$.kind')<>'secret' OR q.boot_id=(SELECT boot_id FROM wrapper_forms_boot WHERE singleton=1))
), sources(id,kind,bot_id,conversation_id,title,status,started_at,request_id,request_kind,card_id,question) AS (
 SELECT 'input:'||id,kind,bot_id,conversation_id,
 CASE request_kind WHEN 'approval' THEN 'Autorisation demandée' WHEN 'secret' THEN 'Secret demandé' ELSE 'Informations demandées' END,
 'pending',started_at,id,request_kind,entry_id,NULL FROM pending
 UNION ALL
 SELECT 'shell:'||s.run_id||'/'||s.call_id,'shell',r.bot_id,r.conversation_id,'Commande de fond',
 CASE s.status WHEN 'en cours' THEN 'running' WHEN 'terminé' THEN 'finished' WHEN 'échec' THEN 'failed' ELSE 'stopped' END,
 r.started_at,NULL,NULL,NULL,NULL
 FROM wrapper_shell_tasks s JOIN live_runs r ON r.run_id=s.run_id
 UNION ALL
 SELECT 'task:'||r.run_id,'task',r.bot_id,r.conversation_id,'Tâche du bot',
 CASE r.status WHEN 'finished' THEN CASE WHEN json_extract(r.record,'$.kind')='task'
 AND json_extract(r.record,'$.task_status.status')<>'done' THEN 'stopped' ELSE 'finished' END
 WHEN 'failed' THEN 'failed' WHEN 'cancelled' THEN 'stopped' WHEN 'interrupted' THEN 'stopped'
 WHEN 'awaiting_approval' THEN r.wait_status WHEN 'awaiting_human_input' THEN r.wait_status ELSE 'running' END,
 r.started_at,NULL,NULL,NULL,NULL FROM live_runs r
 WHERE r.trigger_kind='user_message'
 AND NOT EXISTS (SELECT 1 FROM pending p WHERE p.run_id=r.run_id)
 UNION ALL
 SELECT 'delegation:'||d.id,'delegation',d.child_bot,d.origin_conversation_id,'Sous-tâche',
 CASE json_extract(d.status,'$.status') WHEN 'done' THEN 'finished' WHEN 'failed' THEN 'failed'
 WHEN 'refused' THEN 'failed' WHEN 'cancelled' THEN 'stopped'
 ELSE CASE r.status WHEN 'interrupted' THEN 'stopped' WHEN 'cancelled' THEN 'stopped' WHEN 'failed' THEN 'failed'
 WHEN 'awaiting_approval' THEN r.wait_status WHEN 'awaiting_human_input' THEN r.wait_status
 WHEN 'finished' THEN CASE json_extract(r.record,'$.task_status.status') WHEN 'done' THEN 'finished' ELSE 'failed' END
 ELSE 'running' END END,
 COALESCE(r.started_at,d.origin_started_at),NULL,NULL,NULL,NULL
 FROM live_edges d LEFT JOIN live_runs r ON r.run_id=d.child_run_id
 WHERE (r.run_id IS NOT NULL OR NOT EXISTS (SELECT 1 FROM engine_runs child WHERE child.run_id=d.child_run_id))
 AND NOT EXISTS (SELECT 1 FROM pending q WHERE q.run_id=d.child_run_id)
 UNION ALL
 SELECT 'routine:'||o.occurrence_id,'routine',s.bot_id,s.conversation_id,'Routine',
 CASE o.status WHEN 'done' THEN 'finished' WHEN 'failed' THEN 'failed'
 ELSE CASE r.status WHEN 'interrupted' THEN 'stopped' WHEN 'cancelled' THEN 'stopped' WHEN 'failed' THEN 'failed'
 WHEN 'awaiting_approval' THEN r.wait_status WHEN 'awaiting_human_input' THEN r.wait_status
 WHEN 'finished' THEN CASE json_extract(r.record,'$.task_status.status') WHEN 'done' THEN 'finished' ELSE 'failed' END
 ELSE 'running' END END,
 COALESCE(r.started_at,o.scheduled_at_utc),NULL,NULL,NULL,NULL
 FROM scheduler_occurrences o JOIN scheduler_routines s ON s.routine_id=o.routine_id
 LEFT JOIN live_runs r ON r.run_id=o.run_id
 WHERE o.status IN ('running','done','failed') AND (o.run_id IS NULL OR r.run_id IS NOT NULL)
 AND NOT EXISTS (SELECT 1 FROM pending q WHERE q.run_id=o.run_id)
 UNION ALL
 SELECT 'question:'||q.conversation_id,'question',q.bot_id,q.conversation_id,'Question en attente','pending',q.created_at,NULL,NULL,NULL,q.question
 FROM questions q WHERE q.latest=1 AND NOT EXISTS (SELECT 1 FROM wrapper_messages s WHERE s.account_id=?1
 AND s.conversation_id=q.conversation_id AND s.kind IN ('submission','reset','engine_reset') AND s.ordinal>q.ordinal)
), visible AS (
 SELECT s.*,
 CASE s.status WHEN 'pending' THEN 'decision' WHEN 'running' THEN 'running' ELSE 'recent' END AS grouping
 FROM sources s JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=s.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=s.bot_id AND b.deleted=0
), ranked AS (
 SELECT *,row_number() OVER (PARTITION BY grouping ORDER BY started_at DESC,id) AS recency FROM visible
)
SELECT id,kind,bot_id,conversation_id,title,status,started_at,grouping,request_id,request_kind,card_id,question
 FROM ranked WHERE grouping<>'recent' OR recency<=20
 ORDER BY CASE grouping WHEN 'decision' THEN 0 WHEN 'running' THEN 1 ELSE 2 END,
 CASE WHEN grouping='decision' THEN started_at END ASC,
 started_at DESC,id";

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    _: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if method != "GET" || path != "/api/taches" {
        return None;
    }
    Some((|| {
        let mut statement = app.connection.prepare(SNAPSHOT)?;
        let mut items = statement.query_map(rusqlite::params![ACCOUNT, crate::now()], |r| {
            let request_id: Option<String> = r.get(8)?;
            let request = match request_id {
                Some(id) => json!({"id":id,"kind":r.get::<_,String>(9)?,"card_id":r.get::<_,String>(10)?}),
                None => serde_json::Value::Null,
            };
            Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,
                "bot_id":r.get::<_,String>(2)?,"conversation_id":r.get::<_,String>(3)?,
                "title":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,
                "started_at":r.get::<_,i64>(6)?,"group":r.get::<_,String>(7)?,
                "request":request,"question":r.get::<_,Option<String>>(11)?}))
        })?.collect::<Result<Vec<_>,_>>()?;
        if items.iter().any(|item| item["question"].is_string()) {
            let vault = super::forms::vault(&app.feature_context())?;
            for item in &mut items {
                if let Some(question) = item["question"].as_str() {
                    item["question"] = json!(shortened(&redact(&vault, question)?));
                }
            }
        }
        Ok(ApiResponse::Json(json!({"items":items})))
    })())
}
