//! Durable runs, append-only events and idempotent wakes (all tables engine-prefixed).
//! The caller owns transactions: no function here starts or commits one. Multi-write
//! operations reject autocommit rather than risk partially committing a lifecycle change.
//! REA `reports/server-orchestration-residual.md` SR01–SR02 establishes pending wake
//! reconciliation, not a server delivery guarantee. Our receipt is committed with Resumed.
//! Privacy redaction is the sole exception to append-only event payloads.
use crate::contracts::{
    Result, RunEvent, RunEventKind, TaskStatus, Trigger, TurnEnd, WakeCause, WakeStatus,
};
use crate::model::{invalid, BudgetUsage, Run, RunStatus};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wake {
    pub wake_id: String,
    pub run_id: String,
    pub cause: WakeCause,
    pub reference: String,
    pub status: WakeStatus,
}

pub(crate) fn require_transaction(conn: &Connection) -> Result<()> {
    if conn.is_autocommit() {
        return Err(invalid(
            "engine mutations require a caller-owned transaction",
        ));
    }
    Ok(())
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS component_versions (
            component TEXT PRIMARY KEY, version INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS engine_runs (
            run_id TEXT PRIMARY KEY, bot_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
            trigger_kind TEXT NOT NULL, trigger_ref TEXT NOT NULL, attempt INTEGER NOT NULL,
            status TEXT NOT NULL, record TEXT NOT NULL,
            usage TEXT NOT NULL DEFAULT '{\"model_calls\":0,\"tool_calls\":0,\"elapsed_ms\":0}',
            UNIQUE(trigger_kind, trigger_ref, attempt)
        );
        CREATE INDEX IF NOT EXISTS engine_runs_bot ON engine_runs(bot_id);
        CREATE TABLE IF NOT EXISTS engine_run_events (
            run_id TEXT NOT NULL REFERENCES engine_runs(run_id), seq INTEGER NOT NULL CHECK(seq > 0),
            kind TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(run_id, seq)
        );
        CREATE TABLE IF NOT EXISTS engine_run_wakes (
            wake_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES engine_runs(run_id),
            cause TEXT NOT NULL, reference TEXT NOT NULL, state TEXT NOT NULL,
            status TEXT NOT NULL, after_seq INTEGER NOT NULL,
            UNIQUE(run_id, cause, reference)
        );
        CREATE INDEX IF NOT EXISTS engine_wakes_pending ON engine_run_wakes(state);
        INSERT INTO component_versions(component, version) VALUES ('engine', 1)
            ON CONFLICT(component) DO NOTHING;",
    )?;
    let version: i64 = conn.query_row(
        "SELECT version FROM component_versions WHERE component = 'engine'",
        [],
        |row| row.get(0),
    )?;
    if version != 1 {
        return Err(invalid(format!(
            "unsupported engine schema version {version}"
        )));
    }
    Ok(())
}

fn tag(value: &impl Serialize) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("expected a string enum"))
}

pub(crate) fn trigger_key(trigger: &Trigger) -> (&str, &str, u32) {
    match trigger {
        Trigger::UserMessage {
            message_id,
            attempt,
        } => ("user_message", message_id, *attempt),
        Trigger::Occurrence { occurrence_id } => ("occurrence", occurrence_id, 0),
        Trigger::Delegation { delegation_id } => ("delegation", delegation_id, 0),
    }
}

pub fn get(conn: &Connection, run_id: &str) -> Result<Option<Run>> {
    let record: Option<String> = conn
        .query_row(
            "SELECT record FROM engine_runs WHERE run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    record
        .map(|record| Ok(serde_json::from_str(&record)?))
        .transpose()
}

pub(crate) fn required(conn: &Connection, run_id: &str) -> Result<Run> {
    get(conn, run_id)?.ok_or_else(|| invalid(format!("unknown run {run_id}")))
}

pub(crate) fn by_trigger(
    conn: &Connection,
    kind: &str,
    reference: &str,
    attempt: u32,
) -> Result<Option<Run>> {
    let record: Option<String> = conn.query_row(
        "SELECT record FROM engine_runs WHERE trigger_kind=?1 AND trigger_ref=?2 AND attempt=?3",
        params![kind, reference, attempt], |row| row.get(0),
    ).optional()?;
    record
        .map(|record| Ok(serde_json::from_str(&record)?))
        .transpose()
}

pub(crate) fn latest_attempt(
    conn: &Connection,
    kind: &str,
    reference: &str,
) -> Result<Option<Run>> {
    let record: Option<String> = conn
        .query_row(
            "SELECT record FROM engine_runs WHERE trigger_kind=?1 AND trigger_ref=?2
         ORDER BY attempt DESC LIMIT 1",
            params![kind, reference],
            |row| row.get(0),
        )
        .optional()?;
    record
        .map(|record| Ok(serde_json::from_str(&record)?))
        .transpose()
}

pub(crate) fn insert(conn: &Connection, run: &Run) -> Result<()> {
    require_transaction(conn)?;
    let (kind, reference, attempt) = trigger_key(&run.trigger);
    conn.execute(
        "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![run.run_id, run.bot_id, run.conversation_id, kind, reference, attempt,
            tag(&run.status)?, serde_json::to_string(run)?],
    )?;
    Ok(())
}

/// Update an existing run, never its identity/trigger. Creation goes through
/// model::start so uniqueness and regeneration cannot be bypassed by an upsert.
/// Append the matching lifecycle event in the same caller-owned transaction.
pub fn save(conn: &Connection, run: &Run) -> Result<()> {
    require_transaction(conn)?;
    run.validate()?;
    let old = required(conn, &run.run_id)?;
    if old == *run {
        return Ok(());
    }
    if old.bot_id != run.bot_id
        || old.conversation_id != run.conversation_id
        || old.trigger != run.trigger
        || old.started_at != run.started_at
        || old.budgets != run.budgets
    {
        return Err(invalid("run identity, trigger and budgets are immutable"));
    }
    let mut superseded = old.clone();
    superseded.status = RunStatus::Cancelled;
    superseded.turn_end = Some(TurnEnd::Superseded);
    if !old.status.can_transition_to(run.status) && *run != superseded {
        return Err(invalid("invalid run lifecycle transition"));
    }
    if (old.kind != run.kind && run.task_status != Some(TaskStatus::Working))
        || (old.kind == crate::contracts::RunKind::Task && run.kind != old.kind)
        || (matches!(old.task_status, Some(TaskStatus::Done { .. }))
            && old.task_status != run.task_status)
    {
        return Err(invalid("task conversion or completion cannot be reversed"));
    }
    conn.execute(
        "UPDATE engine_runs SET status=?2, record=?3 WHERE run_id=?1",
        params![run.run_id, tag(&run.status)?, serde_json::to_string(run)?],
    )?;
    if run.status.is_terminal() {
        fail_pending_wakes(conn, &run.run_id, "run ended before wake delivery")?;
    }
    Ok(())
}

/// A rejected late result has no event and cannot change the run. Tool effects
/// must be composed with this append in a transaction and errors must propagate.
pub fn append_event(
    conn: &Connection,
    run_id: &str,
    kind: RunEventKind,
    payload: &Value,
) -> Result<RunEvent> {
    require_transaction(conn)?;
    let run = required(conn, run_id)?;
    let allowed = match kind {
        RunEventKind::Cancelled => run.status == RunStatus::Cancelled,
        RunEventKind::TurnEnded => run.status.is_terminal(),
        RunEventKind::Interrupted => run.status == RunStatus::Interrupted,
        RunEventKind::Resumed => run.status == RunStatus::Running,
        _ => !run.status.is_terminal() && run.status != RunStatus::Interrupted,
    };
    if !allowed {
        return Err(invalid(
            "event ignored after interruption or invalid lifecycle change",
        ));
    }
    let seq: u64 = conn.query_row(
        "INSERT INTO engine_run_events(run_id,seq,kind,payload)
         SELECT ?1, COALESCE(MAX(seq),0)+1, ?2, ?3 FROM engine_run_events WHERE run_id=?1
         RETURNING seq",
        params![run_id, tag(&kind)?, serde_json::to_string(payload)?],
        |row| row.get(0),
    )?;
    Ok(RunEvent {
        run_id: run_id.into(),
        seq,
        kind,
        payload: payload.clone(),
    })
}

pub fn events(conn: &Connection, run_id: &str, after_seq: u64) -> Result<Vec<RunEvent>> {
    if after_seq > i64::MAX as u64 {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "SELECT seq,kind,payload FROM engine_run_events WHERE run_id=?1 AND seq>?2 ORDER BY seq",
    )?;
    let rows = statement.query_map(params![run_id, after_seq], |row| {
        Ok((
            row.get::<_, u64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.map(|row| {
        let (seq, kind, payload) = row?;
        Ok(RunEvent {
            run_id: run_id.into(),
            seq,
            kind: serde_json::from_value(Value::String(kind))?,
            payload: serde_json::from_str(&payload)?,
        })
    })
    .collect()
}

/// Durable counters survive interruption. Elapsed time includes time suspended
/// or offline; clocks before started_at are clamped rather than underflowing.
pub fn budget_usage(conn: &Connection, run_id: &str, now_ms: i64) -> Result<BudgetUsage> {
    let run = required(conn, run_id)?;
    let usage: String = conn.query_row(
        "SELECT usage FROM engine_runs WHERE run_id=?1",
        [run_id],
        |row| row.get(0),
    )?;
    let mut usage: BudgetUsage = serde_json::from_str(&usage)?;
    usage.elapsed_ms = usage
        .elapsed_ms
        .max(now_ms.saturating_sub(run.started_at).max(0) as u64);
    Ok(usage)
}

pub fn save_budget_usage(conn: &Connection, run_id: &str, usage: &BudgetUsage) -> Result<()> {
    require_transaction(conn)?;
    let run = required(conn, run_id)?;
    let old = budget_usage(conn, run_id, run.started_at)?;
    if run.status != RunStatus::Running
        || old.model_calls > usage.model_calls
        || old.tool_calls > usage.tool_calls
        || old.elapsed_ms > usage.elapsed_ms
    {
        return Err(invalid("budget usage must be monotone on a running run"));
    }
    conn.execute(
        "UPDATE engine_runs SET usage=?2 WHERE run_id=?1",
        params![run_id, serde_json::to_string(usage)?],
    )?;
    Ok(())
}

pub fn enqueue_wake(
    conn: &Connection,
    run_id: &str,
    cause: WakeCause,
    reference: &str,
) -> Result<Wake> {
    require_transaction(conn)?;
    if reference.is_empty() {
        return Err(invalid("wake reference must not be empty"));
    }
    let run = required(conn, run_id)?;
    // JSON tuple boundaries avoid ambiguous concatenations and make retry IDs stable.
    let key = serde_json::to_vec(&(run_id, cause, reference))?;
    let wake_id = format!("wake_{:x}", Sha256::digest(key));
    let status = if run.status.is_terminal() {
        WakeStatus::Failed {
            reason: "run already ended".into(),
        }
    } else {
        WakeStatus::Pending
    };
    conn.execute(
        "INSERT INTO engine_run_wakes(wake_id,run_id,cause,reference,state,status,after_seq)
         SELECT ?1,?2,?3,?4,?5,?6,COALESCE(MAX(seq),0) FROM engine_run_events WHERE run_id=?2
         ON CONFLICT(run_id,cause,reference) DO NOTHING",
        params![
            wake_id,
            run_id,
            tag(&cause)?,
            reference,
            if run.status.is_terminal() {
                "failed"
            } else {
                "pending"
            },
            serde_json::to_string(&status)?
        ],
    )?;
    Ok(Wake {
        status: wake_status(conn, &wake_id)?,
        wake_id,
        run_id: run_id.into(),
        cause,
        reference: reference.into(),
    })
}

pub fn pending_wakes(conn: &Connection) -> Result<Vec<Wake>> {
    let mut statement = conn.prepare(
        "SELECT wake_id,run_id,cause,reference,status FROM engine_run_wakes WHERE state='pending' ORDER BY rowid",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (wake_id, run_id, cause, reference, status) = row?;
        Ok(Wake {
            wake_id,
            run_id,
            cause: serde_json::from_value(Value::String(cause))?,
            reference,
            status: serde_json::from_str(&status)?,
        })
    })
    .collect()
}

pub fn wake_status(conn: &Connection, wake_id: &str) -> Result<WakeStatus> {
    let status: String = conn.query_row(
        "SELECT status FROM engine_run_wakes WHERE wake_id=?1",
        [wake_id],
        |row| row.get(0),
    )?;
    Ok(serde_json::from_str(&status)?)
}

/// Call only in the transaction that commits the first post-wake Resumed event.
/// Merely reading a pending wake or appending unrelated progress is not delivery.
pub fn mark_wake_delivered(conn: &Connection, wake_id: &str, first_event_seq: u64) -> Result<()> {
    require_transaction(conn)?;
    let (run_id, after_seq): (String, u64) = conn.query_row(
        "SELECT run_id,after_seq FROM engine_run_wakes WHERE wake_id=?1",
        [wake_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let delivered = WakeStatus::Delivered {
        run_id: run_id.clone(),
        first_event_seq,
    };
    match wake_status(conn, wake_id)? {
        status if status == delivered => return Ok(()),
        WakeStatus::Pending => {}
        _ => return Err(invalid("wake is already settled with another receipt")),
    }
    let first = events(conn, &run_id, after_seq)?.into_iter().find(|event| {
        event.kind == RunEventKind::Resumed
            && event
                .payload
                .get("wake_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(wake_id)))
    });
    if !first.is_some_and(|event| event.seq == first_event_seq) {
        return Err(invalid(
            "wake delivery requires its first committed resume event",
        ));
    }
    conn.execute("UPDATE engine_run_wakes SET state='delivered',status=?2 WHERE wake_id=?1 AND state='pending'",
        params![wake_id, serde_json::to_string(&delivered)?])?;
    Ok(())
}

pub(crate) fn fail_pending_wakes(conn: &Connection, run_id: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE engine_run_wakes SET state='failed',status=?2 WHERE run_id=?1 AND state='pending'",
        params![
            run_id,
            serde_json::to_string(&WakeStatus::Failed {
                reason: reason.into()
            })?
        ],
    )?;
    Ok(())
}

pub fn interrupt_running(conn: &Connection) -> Result<usize> {
    require_transaction(conn)?;
    let ids: Vec<String> = conn
        .prepare("SELECT run_id FROM engine_runs WHERE status='running' ORDER BY run_id")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &ids {
        let mut run = required(conn, id)?;
        run.status = RunStatus::Interrupted;
        save(conn, &run)?;
        append_event(
            conn,
            id,
            RunEventKind::Interrupted,
            &json!({"reason": "process interrupted"}),
        )?;
    }
    Ok(ids.len())
}

fn redact_value(value: &mut Value, text: &str) {
    fn redact_string(value: &str, text: &str) -> String {
        let replacement = if "[redacted]".contains(text) {
            ""
        } else {
            "[redacted]"
        };
        let mut value = value.to_owned();
        while value.contains(text) {
            value = value.replace(text, replacement);
        }
        value
    }
    match value {
        Value::String(value) => *value = redact_string(value, text),
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| redact_value(value, text)),
        Value::Object(values) => {
            *values = std::mem::take(values)
                .into_iter()
                .map(|(key, mut value)| {
                    redact_value(&mut value, text);
                    (redact_string(&key, text), value)
                })
                .collect();
        }
        _ => {}
    }
}

/// Remove forgotten text from arguments, outputs and display payloads for this
/// bot only, recursively in decoded JSON (including escaped strings/keys).
/// Also sanitize persisted task summaries; IDs, event sequence and kinds remain.
/// Returns the number of changed events, not the number of substring matches.
pub fn redact_payloads(conn: &Connection, bot_id: &str, text: &str) -> Result<usize> {
    require_transaction(conn)?;
    if text.is_empty() {
        return Ok(0);
    }
    let ids: Vec<String> = conn
        .prepare("SELECT run_id FROM engine_runs WHERE bot_id=?1")?
        .query_map([bot_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = 0;
    for id in ids {
        for mut event in events(conn, &id, 0)? {
            let old = event.payload.clone();
            redact_value(&mut event.payload, text);
            if old != event.payload {
                conn.execute(
                    "UPDATE engine_run_events SET payload=?3 WHERE run_id=?1 AND seq=?2",
                    params![id, event.seq, serde_json::to_string(&event.payload)?],
                )?;
                changed += 1;
            }
        }
        let mut run = required(conn, &id)?;
        if let Some(TaskStatus::Done { summary }) = &mut run.task_status {
            let mut value = Value::String(summary.clone());
            redact_value(&mut value, text);
            *summary = value.as_str().unwrap().to_owned();
            conn.execute(
                "UPDATE engine_runs SET record=?2 WHERE run_id=?1",
                params![id, serde_json::to_string(&run)?],
            )?;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{with_tx, Error, RunBudgets, RunKind};
    use crate::model;

    fn sample() -> Run {
        Run {
            run_id: "run".into(),
            bot_id: "bot".into(),
            conversation_id: "conversation".into(),
            trigger: Trigger::UserMessage {
                message_id: "message".into(),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Running,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 1000,
        }
    }

    fn database() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        migrate(&conn)?;
        with_tx(&conn, |conn| model::start(conn, &sample()))?;
        Ok(conn)
    }

    #[test]
    fn store_migration_is_idempotent_and_keeps_user_version() -> Result<()> {
        let conn = database()?;
        conn.pragma_update(None, "user_version", 42)?;
        migrate(&conn)?;
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
            42
        );
        assert_eq!(events(&conn, "run", 0)?.len(), 1);
        assert_eq!(
            conn.query_row(
                "SELECT version FROM component_versions WHERE component='engine'",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            1
        );
        assert!(model::start(&conn, &sample()).is_err());
        Ok(())
    }

    #[test]
    fn store_late_tool_result_after_cancel_is_ignored() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| model::cancel(conn, "run", TurnEnd::Stopped))?;
        conn.execute_batch("CREATE TABLE effects(id TEXT PRIMARY KEY)")?;
        let late: Result<()> = with_tx(&conn, |conn| {
            conn.execute("INSERT INTO effects VALUES ('late')", [])?;
            append_event(
                conn,
                "run",
                RunEventKind::ToolResult,
                &json!({"call_id":"late"}),
            )?;
            Ok(())
        });
        assert!(late.is_err());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM effects", [], |row| row
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(events(&conn, "run", 0)?.len(), 2);
        let mut stale = sample();
        stale.status = RunStatus::Running;
        assert!(with_tx(&conn, |conn| save(conn, &stale)).is_err());
        assert_eq!(required(&conn, "run")?.status, RunStatus::Cancelled);
        Ok(())
    }

    #[test]
    fn store_interrupted_run_resumes_after_last_committed_event() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            append_event(
                conn,
                "run",
                RunEventKind::ToolResult,
                &json!({"call_id":"committed"}),
            )?;
            save_budget_usage(
                conn,
                "run",
                &BudgetUsage {
                    model_calls: 2,
                    tool_calls: 1,
                    elapsed_ms: 50,
                },
            )
        })?;
        let interrupted: Result<()> = with_tx(&conn, |conn| {
            append_event(conn, "run", RunEventKind::TextDelta, &json!("uncommitted"))?;
            Err(invalid("simulated process failure before commit"))
        });
        assert!(interrupted.is_err());
        with_tx(&conn, |conn| {
            assert_eq!(interrupt_running(conn)?, 1);
            assert_eq!(interrupt_running(conn)?, 0);
            assert_eq!(required(conn, "run")?.status, RunStatus::Interrupted);
            let run = model::resume(conn, "run")?;
            assert_eq!(run.run_id, "run");
            assert_eq!(model::resume(conn, "run")?, run);
            let tail = events(conn, "run", 2)?;
            assert_eq!(
                tail.iter().map(|event| event.seq).collect::<Vec<_>>(),
                vec![3, 4]
            );
            assert_eq!(tail[0].kind, RunEventKind::Interrupted);
            assert_eq!(tail[1].kind, RunEventKind::Resumed);
            assert_eq!(events(conn, "run", 0)?.len(), 4);
            assert_eq!(
                budget_usage(conn, "run", 2000)?,
                BudgetUsage {
                    model_calls: 2,
                    tool_calls: 1,
                    elapsed_ms: 1000
                }
            );
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_wake_resumes_same_run_once() -> Result<()> {
        let conn = database()?;
        let wake = with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingHumanInput;
            save(conn, &run)?;
            append_event(
                conn,
                "run",
                RunEventKind::Suspended,
                &json!({"request":"form"}),
            )?;
            assert!(model::resume(conn, "run").is_err());
            let wake = enqueue_wake(conn, "run", WakeCause::Form, "form")?;
            assert_eq!(enqueue_wake(conn, "run", WakeCause::Form, "form")?, wake);
            assert_eq!(pending_wakes(conn)?.len(), 1);
            assert!(mark_wake_delivered(conn, &wake.wake_id, 2).is_err());
            Ok::<_, Error>(wake)
        })?;
        let rollback: Result<()> = with_tx(&conn, |conn| {
            model::resume(conn, "run")?;
            assert!(matches!(
                wake_status(conn, &wake.wake_id)?,
                WakeStatus::Delivered {
                    first_event_seq: 3,
                    ..
                }
            ));
            Err(invalid("simulated crash before resume commits"))
        });
        assert!(rollback.is_err());
        assert_eq!(wake_status(&conn, &wake.wake_id)?, WakeStatus::Pending);
        assert_eq!(
            required(&conn, "run")?.status,
            RunStatus::AwaitingHumanInput
        );
        with_tx(&conn, |conn| {
            assert_eq!(model::resume(conn, "run")?.run_id, "run");
            assert_eq!(model::resume(conn, "run")?.run_id, "run");
            assert!(pending_wakes(conn)?.is_empty());
            mark_wake_delivered(conn, &wake.wake_id, 3)?;
            assert_eq!(
                enqueue_wake(conn, "run", WakeCause::Form, "form")?.status,
                WakeStatus::Delivered {
                    run_id: "run".into(),
                    first_event_seq: 3
                }
            );
            assert_eq!(events(conn, "run", 2)?.len(), 1);
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_wake_cannot_bypass_approval_and_cancel_fails_pending() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingApproval;
            save(conn, &run)?;
            let wake = enqueue_wake(conn, "run", WakeCause::Child, "child")?;
            assert!(model::resume(conn, "run").is_err());
            model::cancel(conn, "run", TurnEnd::Stopped)?;
            assert!(matches!(
                wake_status(conn, &wake.wake_id)?,
                WakeStatus::Failed { .. }
            ));
            assert!(matches!(
                enqueue_wake(conn, "run", WakeCause::Approval, "approval")?.status,
                WakeStatus::Failed { .. }
            ));
            assert!(pending_wakes(conn)?.is_empty());
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_child_wake_waits_through_unrelated_approval_resume() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingApproval;
            save(conn, &run)?;
            let child = enqueue_wake(conn, "run", WakeCause::Child, "child")?;
            let approval = enqueue_wake(conn, "run", WakeCause::Approval, "approval")?;
            model::resume(conn, "run")?;
            assert!(matches!(
                wake_status(conn, &approval.wake_id)?,
                WakeStatus::Delivered { .. }
            ));
            assert_eq!(wake_status(conn, &child.wake_id)?, WakeStatus::Pending);
            run = required(conn, "run")?;
            run.status = RunStatus::AwaitingChildren;
            save(conn, &run)?;
            model::resume(conn, "run")?;
            assert!(matches!(
                wake_status(conn, &child.wake_id)?,
                WakeStatus::Delivered {
                    first_event_seq: 3,
                    ..
                }
            ));
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_budget_usage_is_monotone_and_transactional() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            save_budget_usage(
                conn,
                "run",
                &BudgetUsage {
                    model_calls: 2,
                    tool_calls: 1,
                    elapsed_ms: 10,
                },
            )
        })?;
        assert!(with_tx(&conn, |conn| save_budget_usage(
            conn,
            "run",
            &BudgetUsage::default()
        ))
        .is_err());
        let failed: Result<()> = with_tx(&conn, |conn| {
            save_budget_usage(
                conn,
                "run",
                &BudgetUsage {
                    model_calls: 3,
                    tool_calls: 1,
                    elapsed_ms: 20,
                },
            )?;
            Err(invalid("call did not commit"))
        });
        assert!(failed.is_err());
        assert_eq!(
            budget_usage(&conn, "run", 0)?,
            BudgetUsage {
                model_calls: 2,
                tool_calls: 1,
                elapsed_ms: 10
            }
        );
        Ok(())
    }

    #[test]
    fn store_redact_payloads_removes_text_from_events() -> Result<()> {
        let conn = database()?;
        let text = "private \"fact\"\n🦀";
        with_tx(&conn, |conn| {
            for kind in [RunEventKind::ToolCall, RunEventKind::ToolResult] {
                append_event(
                    conn,
                    "run",
                    kind,
                    &json!({"nested":[text, {text: format!("before {text} after")}], "safe": 7}),
                )?;
            }
            let mut other = sample();
            other.run_id = "other-run".into();
            other.bot_id = "other-bot".into();
            other.trigger = Trigger::Occurrence {
                occurrence_id: "occurrence".into(),
            };
            other.kind = RunKind::Task;
            other.task_status = Some(TaskStatus::Working);
            model::start(conn, &other)?;
            append_event(conn, "other-run", RunEventKind::ToolResult, &json!(text))?;
            let mut completed = required(conn, "run")?;
            completed.kind = RunKind::Task;
            completed.task_status = Some(TaskStatus::Working);
            save(conn, &completed)?;
            completed.task_status = Some(TaskStatus::Done {
                summary: text.into(),
            });
            completed.status = RunStatus::Finished;
            completed.turn_end = Some(TurnEnd::Finished);
            save(conn, &completed)?;
            assert_eq!(redact_payloads(conn, "bot", text)?, 2);
            assert_eq!(redact_payloads(conn, "bot", text)?, 0);
            assert_eq!(redact_payloads(conn, "bot", "")?, 0);
            for event in events(conn, "run", 1)? {
                assert_eq!(event.payload["safe"], 7);
                assert!(!event.payload.to_string().contains("private"));
                assert_eq!(event.payload["nested"][0], "[redacted]");
            }
            assert_eq!(events(conn, "other-run", 1)?[0].payload, text);
            assert_eq!(
                required(conn, "run")?.task_status,
                Some(TaskStatus::Done {
                    summary: "[redacted]".into()
                })
            );
            assert_eq!(
                events(conn, "run", 0)?
                    .iter()
                    .map(|event| event.seq)
                    .collect::<Vec<_>>(),
                vec![1, 2, 3]
            );
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_trigger_kinds_are_independent_and_tasks_keep_status_on_supersede() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            for (id, trigger) in [
                (
                    "occurrence-run",
                    Trigger::Occurrence {
                        occurrence_id: "message".into(),
                    },
                ),
                (
                    "delegation-run",
                    Trigger::Delegation {
                        delegation_id: "message".into(),
                    },
                ),
            ] {
                let mut run = sample();
                run.run_id = id.into();
                run.trigger = trigger;
                run.kind = RunKind::Task;
                run.task_status = Some(TaskStatus::Working);
                assert_eq!(model::start(conn, &run)?, run);
                assert_eq!(model::start(conn, &run)?, run);
            }
            let mut finished = required(conn, "run")?;
            finished.kind = RunKind::Task;
            finished.task_status = Some(TaskStatus::Working);
            finished.status = RunStatus::Finished;
            finished.turn_end = Some(TurnEnd::Finished);
            save(conn, &finished)?;
            let mut next = sample();
            next.run_id = "regenerated".into();
            next.trigger = Trigger::UserMessage {
                message_id: "message".into(),
                attempt: 1,
            };
            model::start(conn, &next)?;
            let old = required(conn, "run")?;
            assert_eq!(old.task_status, Some(TaskStatus::Working));
            assert_eq!(old.turn_end, Some(TurnEnd::Superseded));
            Ok::<_, Error>(())
        })
    }
}
