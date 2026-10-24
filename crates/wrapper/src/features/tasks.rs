//! Read-only current work, independent of the unread notification outbox.
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use rusqlite::Connection;
use serde_json::json;

empty_hooks!(
    state,
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    // Source journals have no completion timestamps. Record transitions, not poll
    // times, so a long-running task completed today sorts ahead of yesterday's.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_task_clock (
        source TEXT NOT NULL, id TEXT NOT NULL, started_at INTEGER NOT NULL,
        changed_at INTEGER NOT NULL, PRIMARY KEY(source,id));",
    )?;
    for (source, table, id, start) in [
        (
            "run",
            "engine_runs",
            "NEW.run_id",
            "json_extract(NEW.record,'$.started_at')",
        ),
        (
            "shell",
            "wrapper_shell_tasks",
            "NEW.run_id||'/'||NEW.call_id",
            "CAST(unixepoch('subsec')*1000 AS INTEGER)",
        ),
        (
            "delegation",
            "delegation_graph",
            "NEW.id",
            "CAST(unixepoch('subsec')*1000 AS INTEGER)",
        ),
        (
            "routine",
            "scheduler_occurrences",
            "NEW.occurrence_id",
            "CAST(unixepoch('subsec')*1000 AS INTEGER)",
        ),
    ] {
        conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS task_{source}_insert AFTER INSERT ON {table}
            BEGIN INSERT OR REPLACE INTO wrapper_task_clock VALUES
                ('{source}',{id},{start},CAST(unixepoch('subsec')*1000 AS INTEGER)); END;
            CREATE TRIGGER IF NOT EXISTS task_{source}_update AFTER UPDATE OF status ON {table}
            WHEN NEW.status<>OLD.status BEGIN
                INSERT INTO wrapper_task_clock VALUES ('{source}',{id},{start},CAST(unixepoch('subsec')*1000 AS INTEGER))
                ON CONFLICT(source,id) DO UPDATE SET changed_at=excluded.changed_at;
            END;"))?;
    }
    Ok(())
}

// Only identifiers, fixed labels and allowlisted lifecycle states cross this
// boundary. Never project objectives, commands, results, form fields or secrets.
const SNAPSHOT: &str = "
WITH live_runs AS (
 SELECT r.*,json_extract(r.record,'$.started_at') AS started_at FROM engine_runs r
 JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=r.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=r.bot_id AND b.deleted=0
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
), sources(id,kind,bot_id,conversation_id,title,status,started_at,source,clock_id,request_id,request_kind,card_id) AS (
 SELECT 'input:'||id,kind,bot_id,conversation_id,
 CASE request_kind WHEN 'approval' THEN 'Autorisation demandée' WHEN 'secret' THEN 'Secret demandé' ELSE 'Informations demandées' END,
 'pending',started_at,'',id,id,request_kind,entry_id FROM pending
 UNION ALL
 SELECT 'shell:'||s.run_id||'/'||s.call_id,'shell',r.bot_id,r.conversation_id,'Commande de fond',
 CASE s.status WHEN 'en cours' THEN 'running' WHEN 'terminé' THEN 'finished' WHEN 'échec' THEN 'failed' ELSE 'stopped' END,
 r.started_at,'shell',s.run_id||'/'||s.call_id,NULL,NULL,NULL
 FROM wrapper_shell_tasks s JOIN live_runs r ON r.run_id=s.run_id
 UNION ALL
 SELECT 'task:'||r.run_id,'task',r.bot_id,r.conversation_id,'Tâche du bot',
 CASE r.status WHEN 'finished' THEN CASE WHEN json_extract(r.record,'$.kind')='task'
 AND json_extract(r.record,'$.task_status.status')<>'done' THEN 'stopped' ELSE 'finished' END
 WHEN 'failed' THEN 'failed' WHEN 'cancelled' THEN 'stopped' WHEN 'interrupted' THEN 'stopped' ELSE 'running' END,
 r.started_at,'run',r.run_id,NULL,NULL,NULL FROM live_runs r
 WHERE r.trigger_kind='user_message' AND r.status NOT IN ('awaiting_approval','awaiting_human_input')
 AND NOT EXISTS (SELECT 1 FROM pending p WHERE p.run_id=r.run_id)
 UNION ALL
 SELECT 'delegation:'||d.id,'delegation',d.child_bot,p.conversation_id,'Sous-tâche',
 CASE json_extract(d.status,'$.status') WHEN 'done' THEN 'finished' WHEN 'failed' THEN 'failed'
 WHEN 'refused' THEN 'failed' WHEN 'cancelled' THEN 'stopped'
 ELSE CASE r.status WHEN 'interrupted' THEN 'stopped' WHEN 'cancelled' THEN 'stopped' WHEN 'failed' THEN 'failed'
 WHEN 'finished' THEN CASE json_extract(r.record,'$.task_status.status') WHEN 'done' THEN 'finished' ELSE 'failed' END
 ELSE 'running' END END,
 COALESCE(r.started_at,p.started_at),
 CASE WHEN json_extract(d.status,'$.status') IN ('queued','running') AND r.status IN ('finished','failed','cancelled','interrupted') THEN 'run' ELSE 'delegation' END,
 CASE WHEN json_extract(d.status,'$.status') IN ('queued','running') AND r.status IN ('finished','failed','cancelled','interrupted') THEN r.run_id ELSE d.id END,NULL,NULL,NULL
 FROM delegation_graph d JOIN live_runs p ON p.run_id=d.parent_run_id
 LEFT JOIN live_runs r ON r.run_id=d.child_run_id
 WHERE (d.child_run_id IS NULL OR r.run_id IS NOT NULL)
 AND (d.conversation_id IS NULL OR EXISTS (SELECT 1 FROM wrapper_conversations c WHERE c.account_id=?1 AND c.id=d.conversation_id AND c.deleted=0))
 AND NOT EXISTS (SELECT 1 FROM pending q WHERE q.run_id=d.child_run_id)
 UNION ALL
 SELECT 'routine:'||o.occurrence_id,'routine',s.bot_id,s.conversation_id,'Routine',
 CASE o.status WHEN 'done' THEN 'finished' WHEN 'failed' THEN 'failed'
 ELSE CASE r.status WHEN 'interrupted' THEN 'stopped' WHEN 'cancelled' THEN 'stopped' WHEN 'failed' THEN 'failed'
 WHEN 'finished' THEN CASE json_extract(r.record,'$.task_status.status') WHEN 'done' THEN 'finished' ELSE 'failed' END
 ELSE 'running' END END,
 COALESCE(r.started_at,o.scheduled_at_utc),
 CASE WHEN o.status='running' AND r.status IN ('finished','failed','cancelled','interrupted') THEN 'run' ELSE 'routine' END,
 CASE WHEN o.status='running' AND r.status IN ('finished','failed','cancelled','interrupted') THEN r.run_id ELSE o.occurrence_id END,NULL,NULL,NULL
 FROM scheduler_occurrences o JOIN scheduler_routines s ON s.routine_id=o.routine_id
 LEFT JOIN live_runs r ON r.run_id=o.run_id
 WHERE o.status IN ('running','done','failed') AND (o.run_id IS NULL OR r.run_id IS NOT NULL)
 AND NOT EXISTS (SELECT 1 FROM pending q WHERE q.run_id=o.run_id)
), visible AS (
 SELECT s.*,CASE WHEN s.source='shell' THEN COALESCE(t.started_at,s.started_at) ELSE s.started_at END AS start,
 COALESCE(t.changed_at,s.started_at) AS changed,
 CASE s.status WHEN 'pending' THEN 'decision' WHEN 'running' THEN 'running' ELSE 'recent' END AS grouping
 FROM sources s JOIN wrapper_conversations c ON c.account_id=?1 AND c.id=s.conversation_id AND c.deleted=0
 JOIN wrapper_bots b ON b.account_id=?1 AND b.id=s.bot_id AND b.deleted=0
 LEFT JOIN wrapper_task_clock t ON t.source=s.source AND t.id=s.clock_id
), ranked AS (
 SELECT *,row_number() OVER (PARTITION BY grouping ORDER BY changed DESC,id) AS recency FROM visible
)
SELECT id,kind,bot_id,conversation_id,title,status,start,grouping,request_id,request_kind,card_id
 FROM ranked WHERE grouping<>'recent' OR recency<=20
 ORDER BY CASE grouping WHEN 'decision' THEN 0 WHEN 'running' THEN 1 ELSE 2 END,
 CASE WHEN grouping='decision' THEN start END ASC,
 CASE WHEN grouping='running' THEN start ELSE changed END DESC,id";

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
