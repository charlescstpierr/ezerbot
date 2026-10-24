//! Read-only current work, independent of the unread notification outbox.
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use serde_json::json;

empty_hooks!(
    migrate,
    state,
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

// Only identifiers, fixed labels and allowlisted lifecycle states cross this
// boundary. Never project objectives, commands, results, form fields or secrets.
// `live_runs` is the one definition of a visible run: a delegated run is visible
// only through a chain of visible edges up to a run that was not delegated.
const SNAPSHOT: &str = "
WITH RECURSIVE open_runs AS (
 SELECT r.*,json_extract(r.record,'$.started_at') AS started_at,
 CASE WHEN EXISTS (SELECT 1 FROM engine_run_wakes w WHERE w.run_id=r.run_id AND w.state='pending')
 THEN 'running' ELSE 'stopped' END AS wait_status FROM engine_runs r
 JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=r.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=r.bot_id AND b.deleted=0
), open_edges AS (
 SELECT d.* FROM delegation_graph d WHERE d.conversation_id IS NULL
 OR EXISTS (SELECT 1 FROM wrapper_conversations c WHERE c.account_id=?1 AND c.id=d.conversation_id AND c.deleted=0)
), live_edges AS (
 SELECT d.*,p.conversation_id AS origin_conversation_id,p.started_at AS origin_started_at
 FROM open_edges d JOIN open_runs p ON p.run_id=d.parent_run_id WHERE p.trigger_kind<>'delegation'
 UNION
 SELECT d.*,p.conversation_id,p.started_at
 FROM live_edges e JOIN open_edges d ON d.parent_run_id=e.child_run_id
 JOIN open_runs p ON p.run_id=d.parent_run_id
), live_runs AS (
 SELECT * FROM open_runs WHERE trigger_kind<>'delegation'
 OR run_id IN (SELECT child_run_id FROM live_edges)
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
), sources(id,kind,bot_id,conversation_id,title,status,started_at,request_id,request_kind,card_id) AS (
 SELECT 'input:'||id,kind,bot_id,conversation_id,
 CASE request_kind WHEN 'approval' THEN 'Autorisation demandée' WHEN 'secret' THEN 'Secret demandé' ELSE 'Informations demandées' END,
 'pending',started_at,id,request_kind,entry_id FROM pending
 UNION ALL
 SELECT 'shell:'||s.run_id||'/'||s.call_id,'shell',r.bot_id,r.conversation_id,'Commande de fond',
 CASE s.status WHEN 'en cours' THEN 'running' WHEN 'terminé' THEN 'finished' WHEN 'échec' THEN 'failed' ELSE 'stopped' END,
 r.started_at,NULL,NULL,NULL
 FROM wrapper_shell_tasks s JOIN live_runs r ON r.run_id=s.run_id
 UNION ALL
 SELECT 'task:'||r.run_id,'task',r.bot_id,r.conversation_id,'Tâche du bot',
 CASE r.status WHEN 'finished' THEN CASE WHEN json_extract(r.record,'$.kind')='task'
 AND json_extract(r.record,'$.task_status.status')<>'done' THEN 'stopped' ELSE 'finished' END
 WHEN 'failed' THEN 'failed' WHEN 'cancelled' THEN 'stopped' WHEN 'interrupted' THEN 'stopped'
 WHEN 'awaiting_approval' THEN r.wait_status WHEN 'awaiting_human_input' THEN r.wait_status ELSE 'running' END,
 r.started_at,NULL,NULL,NULL FROM live_runs r
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
 COALESCE(r.started_at,d.origin_started_at),NULL,NULL,NULL
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
 COALESCE(r.started_at,o.scheduled_at_utc),NULL,NULL,NULL
 FROM scheduler_occurrences o JOIN scheduler_routines s ON s.routine_id=o.routine_id
 LEFT JOIN live_runs r ON r.run_id=o.run_id
 WHERE o.status IN ('running','done','failed') AND (o.run_id IS NULL OR r.run_id IS NOT NULL)
 AND NOT EXISTS (SELECT 1 FROM pending q WHERE q.run_id=o.run_id)
), visible AS (
 SELECT s.*,
 CASE s.status WHEN 'pending' THEN 'decision' WHEN 'running' THEN 'running' ELSE 'recent' END AS grouping
 FROM sources s JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=s.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=s.bot_id AND b.deleted=0
), ranked AS (
 SELECT *,row_number() OVER (PARTITION BY grouping ORDER BY started_at DESC,id) AS recency FROM visible
)
SELECT id,kind,bot_id,conversation_id,title,status,started_at,grouping,request_id,request_kind,card_id
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
        let items = statement.query_map(rusqlite::params![ACCOUNT, crate::now()], |r| {
            let request_id: Option<String> = r.get(8)?;
            let request = match request_id {
                Some(id) => json!({"id":id,"kind":r.get::<_,String>(9)?,"card_id":r.get::<_,String>(10)?}),
                None => serde_json::Value::Null,
            };
            Ok(json!({"id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,
                "bot_id":r.get::<_,String>(2)?,"conversation_id":r.get::<_,String>(3)?,
                "title":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,
                "started_at":r.get::<_,i64>(6)?,"group":r.get::<_,String>(7)?,
                "request":request}))
        })?.collect::<Result<Vec<_>,_>>()?;
        Ok(ApiResponse::Json(json!({"items":items})))
    })())
}
