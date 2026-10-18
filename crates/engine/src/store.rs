//! Durable runs, append-only events and idempotent wakes (all tables engine-prefixed).
//! The caller owns transactions: no function here starts or commits one. Multi-write
//! operations reject autocommit rather than risk partially committing a lifecycle change.
//! REA `reports/server-orchestration-residual.md` SR01–SR02 establishes pending wake
//! reconciliation, not a server delivery guarantee. Our receipt is committed with Resumed.
//! Privacy redaction is the sole exception to append-only event payloads.
//! On SQLITE_BUSY or a uniqueness race, callers must roll back and retry the
//! whole transaction, never reuse a stale WAL snapshot (see model::start).
use crate::contracts::{
    Result, RunEvent, RunEventKind, SuspendCause, TaskStatus, TokenUsage, Trigger, TurnEnd,
    WakeCause, WakeStatus,
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
        CREATE INDEX IF NOT EXISTS engine_runs_conversation ON engine_runs(conversation_id, status);
        CREATE INDEX IF NOT EXISTS engine_runs_status ON engine_runs(status);
        CREATE TABLE IF NOT EXISTS engine_run_waits (
            run_id TEXT PRIMARY KEY REFERENCES engine_runs(run_id), refs TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS engine_run_events (
            run_id TEXT NOT NULL REFERENCES engine_runs(run_id), seq INTEGER NOT NULL CHECK(seq > 0),
            kind TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(run_id, seq)
        );
        CREATE TABLE IF NOT EXISTS engine_tool_admissions (
            run_id TEXT NOT NULL REFERENCES engine_runs(run_id), call_id TEXT NOT NULL,
            digest TEXT NOT NULL, authorized_by TEXT,
            PRIMARY KEY(run_id, call_id)
        );
        CREATE TABLE IF NOT EXISTS engine_run_wakes (
            wake_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES engine_runs(run_id),
            cause TEXT NOT NULL, reference TEXT NOT NULL, state TEXT NOT NULL,
            status TEXT NOT NULL, after_seq INTEGER NOT NULL,
            UNIQUE(run_id, cause, reference)
        );
        CREATE TABLE IF NOT EXISTS engine_external_tools (
            run_id TEXT NOT NULL REFERENCES engine_runs(run_id), call_id TEXT NOT NULL,
            authorized_by TEXT, outcome TEXT, PRIMARY KEY(run_id,call_id)
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
    if !(1..=2).contains(&version) {
        return Err(invalid(format!(
            "unsupported engine schema version {version}"
        )));
    }
    if version < 2 {
        // DDL and version marker commit together, including upgrades of existing workspaces.
        conn.execute_batch("SAVEPOINT engine_usage_migration;
            CREATE TABLE engine_response_usage (
                run_id TEXT NOT NULL REFERENCES engine_runs(run_id),
                response INTEGER NOT NULL CHECK(response > 0),
                input_tokens INTEGER NOT NULL CHECK(input_tokens >= 0),
                output_tokens INTEGER NOT NULL CHECK(output_tokens >= 0),
                cached_input_tokens INTEGER NOT NULL CHECK(cached_input_tokens >= 0 AND cached_input_tokens <= input_tokens),
                PRIMARY KEY(run_id, response)
            );
            UPDATE component_versions SET version=2 WHERE component='engine';
            RELEASE engine_usage_migration;")?;
    }
    Ok(())
}

/// One row per provider response, so duplicate telemetry cannot double-count a run.
pub fn save_token_usage(
    conn: &Connection,
    run_id: &str,
    response: u32,
    usage: &TokenUsage,
) -> Result<()> {
    require_transaction(conn)?;
    conn.execute(
        "INSERT INTO engine_response_usage (run_id,response,input_tokens,output_tokens,cached_input_tokens)
        VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT(run_id,response) DO UPDATE SET input_tokens=excluded.input_tokens,
        output_tokens=excluded.output_tokens,cached_input_tokens=excluded.cached_input_tokens",
        params![
            run_id,
            response,
            usage.input_tokens,
            usage.output_tokens,
            usage.cached_input_tokens
        ],
    )?;
    Ok(())
}

/// Unknown for legacy/simulated runs or providers which omit usage (not a fabricated zero).
pub fn token_usage(conn: &Connection, run_id: &str) -> Result<Option<TokenUsage>> {
    Ok(conn
        .query_row(
            "SELECT SUM(input_tokens),SUM(output_tokens),SUM(cached_input_tokens)
        FROM engine_response_usage WHERE run_id=?1 HAVING COUNT(*)>0",
            [run_id],
            |row| {
                Ok(TokenUsage {
                    input_tokens: row.get(0)?,
                    output_tokens: row.get(1)?,
                    cached_input_tokens: row.get(2)?,
                })
            },
        )
        .optional()?)
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

pub fn required(conn: &Connection, run_id: &str) -> Result<Run> {
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

/// Latest attempt of this trigger, ignoring the attempt on the supplied DTO.
pub fn latest_run_for_trigger(conn: &Connection, trigger: &Trigger) -> Result<Option<Run>> {
    let (kind, reference, _) = trigger_key(trigger);
    latest_attempt(conn, kind, reference)
}

pub fn runs_for_conversation(conn: &Connection, conversation_id: &str) -> Result<Vec<Run>> {
    let mut statement =
        conn.prepare("SELECT record FROM engine_runs WHERE conversation_id=?1 ORDER BY rowid")?;
    let rows = statement.query_map([conversation_id], |row| row.get::<_, String>(0))?;
    rows.map(|record| Ok(serde_json::from_str(&record?)?))
        .collect()
}

pub fn runs_with_status(conn: &Connection, status: RunStatus) -> Result<Vec<Run>> {
    let mut statement =
        conn.prepare("SELECT record FROM engine_runs WHERE status=?1 ORDER BY rowid")?;
    let rows = statement.query_map([tag(&status)?], |row| row.get::<_, String>(0))?;
    rows.map(|record| Ok(serde_json::from_str(&record?)?))
        .collect()
}

pub(crate) fn insert(conn: &Connection, run: &Run) -> Result<()> {
    require_transaction(conn)?;
    let (kind, reference, attempt) = trigger_key(&run.trigger);
    conn.execute(
        "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record,usage)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![run.run_id, run.bot_id, run.conversation_id, kind, reference, attempt,
            tag(&run.status)?, serde_json::to_string(run)?, serde_json::to_string(&UsageClock {
                usage: BudgetUsage::default(),
                active_since: (run.status == RunStatus::Running).then_some(run.started_at),
            })?],
    )?;
    Ok(())
}

/// Update an existing run, never its identity/trigger. Creation goes through
/// model::start so uniqueness and regeneration cannot be bypassed by an upsert.
/// Append the matching lifecycle event in the same caller-owned transaction.
pub fn save(conn: &Connection, run: &Run) -> Result<()> {
    save_at(conn, run, now_ms())
}

/// Clock-injected save. Only Running intervals consume duration. An Interrupted
/// transition preserves the last checkpoint rather than charging unknown downtime.
pub fn save_at(conn: &Connection, run: &Run, now_ms: i64) -> Result<()> {
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
    if superseded.task_status == Some(TaskStatus::NeedsInput) {
        superseded.task_status = Some(TaskStatus::Working);
    }
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
    if old.status != run.status {
        let mut clock = read_clock(conn, &run.run_id)?;
        if old.status == RunStatus::Running && run.status != RunStatus::Interrupted {
            clock.usage = clock.at(now_ms);
        }
        clock.active_since = (run.status == RunStatus::Running).then_some(now_ms);
        write_clock(conn, &run.run_id, &clock)?;
        conn.execute(
            "DELETE FROM engine_run_waits WHERE run_id=?1",
            [&run.run_id],
        )?;
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
    if kind == RunEventKind::Suspended {
        if let Some(cause) = payload.get("cause") {
            set_awaiting(conn, run_id, &serde_json::from_value(cause.clone())?)?;
        }
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

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[derive(Default, Serialize, Deserialize)]
struct UsageClock {
    #[serde(flatten)]
    usage: BudgetUsage,
    /// Optional for pre-clock records: recovery preserves only their checkpoint.
    #[serde(default)]
    active_since: Option<i64>,
}

impl UsageClock {
    fn at(&self, now_ms: i64) -> BudgetUsage {
        let mut usage = self.usage.clone();
        if let Some(since) = self.active_since {
            usage.elapsed_ms = usage
                .elapsed_ms
                .saturating_add(now_ms.saturating_sub(since).max(0) as u64);
        }
        usage
    }
}

fn read_clock(conn: &Connection, run_id: &str) -> Result<UsageClock> {
    let encoded: String = conn.query_row(
        "SELECT usage FROM engine_runs WHERE run_id=?1",
        [run_id],
        |row| row.get(0),
    )?;
    Ok(serde_json::from_str(&encoded)?)
}

fn write_clock(conn: &Connection, run_id: &str, clock: &UsageClock) -> Result<()> {
    conn.execute(
        "UPDATE engine_runs SET usage=?2 WHERE run_id=?1",
        params![run_id, serde_json::to_string(clock)?],
    )?;
    Ok(())
}

/// Checkpoint plus current Running interval only. Queue, human/child waits and
/// offline time never consume execution budget. Backward clocks are clamped.
pub fn budget_usage(conn: &Connection, run_id: &str, now_ms: i64) -> Result<BudgetUsage> {
    let run = required(conn, run_id)?;
    let clock = read_clock(conn, run_id)?;
    Ok(if run.status == RunStatus::Running {
        clock.at(now_ms)
    } else {
        clock.usage
    })
}

pub fn save_budget_usage(conn: &Connection, run_id: &str, usage: &BudgetUsage) -> Result<()> {
    save_budget_usage_at(conn, run_id, usage, now_ms())
}

pub fn save_budget_usage_at(
    conn: &Connection,
    run_id: &str,
    usage: &BudgetUsage,
    now_ms: i64,
) -> Result<()> {
    require_transaction(conn)?;
    let run = required(conn, run_id)?;
    let old = read_clock(conn, run_id)?;
    if run.status != RunStatus::Running
        || old.usage.model_calls > usage.model_calls
        || old.usage.tool_calls > usage.tool_calls
        || old.usage.elapsed_ms > usage.elapsed_ms
    {
        return Err(invalid("budget usage must be monotone on a running run"));
    }
    let mut checkpoint = usage.clone();
    checkpoint.elapsed_ms = checkpoint.elapsed_ms.max(old.at(now_ms).elapsed_ms);
    write_clock(
        conn,
        run_id,
        &UsageClock {
            usage: checkpoint,
            active_since: Some(old.active_since.unwrap_or(now_ms).max(now_ms)),
        },
    )
}

/// Persist the exact resolution references expected by a suspension. The loop's
/// Suspended{cause} event invokes this automatically, so existing callers do not
/// need to change Run DTO construction. Direct state writers must call it too.
pub fn set_awaiting(conn: &Connection, run_id: &str, cause: &SuspendCause) -> Result<()> {
    require_transaction(conn)?;
    let run = required(conn, run_id)?;
    let references = match cause {
        SuspendCause::HumanInput { id }
            if matches!(
                run.status,
                RunStatus::AwaitingApproval | RunStatus::AwaitingHumanInput
            ) =>
        {
            vec![id.clone()]
        }
        SuspendCause::Children { ids } if run.status == RunStatus::AwaitingChildren => ids.clone(),
        _ => return Err(invalid("suspension cause does not match run status")),
    };
    if references.is_empty()
        || references.iter().any(String::is_empty)
        || references
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != references.len()
    {
        return Err(invalid("suspension requires nonempty unique references"));
    }
    conn.execute(
        "INSERT INTO engine_run_waits(run_id,refs) VALUES (?1,?2)
        ON CONFLICT(run_id) DO UPDATE SET refs=excluded.refs",
        params![run_id, serde_json::to_string(&references)?],
    )?;
    Ok(())
}

pub fn awaiting_references(conn: &Connection, run_id: &str) -> Result<Vec<String>> {
    let encoded: Option<String> = conn
        .query_row(
            "SELECT refs FROM engine_run_waits WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()?;
    encoded
        .map(|encoded| Ok(serde_json::from_str(&encoded)?))
        .unwrap_or_else(|| Ok(Vec::new()))
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

const REDACTED: &str = "[redacted]";

/// Find on the original UTF-8 string, including overlaps; never rescan inserted
/// markers. Existing markers are sentinels, making short-text redaction repeatable.
fn redaction_ranges(value: &str, text: &str) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut skip_until = 0;
    for (start, _) in value.char_indices() {
        if start < skip_until {
            continue;
        }
        if value[start..].starts_with(REDACTED) {
            skip_until = start + REDACTED.len();
            continue;
        }
        if !text.is_empty() && value[start..].starts_with(text) {
            let end = start + text.len();
            if let Some(last) = ranges.last_mut().filter(|last| start < last.1) {
                last.1 = last.1.max(end);
            } else {
                ranges.push((start, end));
            }
        }
    }
    ranges
}

fn redact_string(value: &str, text: &str) -> String {
    let mut redacted = String::new();
    let mut cursor = 0;
    for (start, end) in redaction_ranges(value, text) {
        redacted.push_str(&value[cursor..start]);
        redacted.push_str(REDACTED);
        cursor = end;
    }
    redacted.push_str(&value[cursor..]);
    redacted
}

fn redact_value(value: &mut Value, text: &str) {
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

fn update_payload(conn: &Connection, run_id: &str, seq: u64, payload: &Value) -> Result<()> {
    conn.execute(
        "UPDATE engine_run_events SET payload=?3 WHERE run_id=?1 AND seq=?2",
        params![run_id, seq, serde_json::to_string(payload)?],
    )?;
    Ok(())
}

fn delta_text(payload: &Value) -> &str {
    payload
        .as_str()
        .or_else(|| payload.get("text").and_then(Value::as_str))
        .unwrap_or_default()
}

/// At most one consecutive stream group is held, not the bot's whole journal.
/// Match across delta boundaries, put one marker in the first covered delta,
/// and remove only the covered bytes from subsequent deltas. Never change seqs.
fn redact_deltas(
    conn: &Connection,
    run_id: &str,
    deltas: &[(u64, Value)],
    text: &str,
) -> Result<usize> {
    let joined: String = deltas
        .iter()
        .map(|(_, payload)| delta_text(payload))
        .collect();
    let ranges = redaction_ranges(&joined, text);
    let mut offset = 0;
    let mut changed = 0;
    for (seq, payload) in deltas {
        let original = delta_text(payload);
        let end = offset + original.len();
        let mut redacted = String::new();
        let mut cursor = 0;
        for &(start, stop) in &ranges {
            if start >= end || stop <= offset {
                continue;
            }
            let local_start = start.saturating_sub(offset);
            let local_stop = stop.min(end) - offset;
            redacted.push_str(&original[cursor..local_start]);
            if start >= offset {
                redacted.push_str(REDACTED);
            }
            cursor = local_stop;
        }
        redacted.push_str(&original[cursor..]);
        let mut sanitized = payload.clone();
        let streamed = sanitized
            .as_object_mut()
            .and_then(|object| object.remove("text"));
        if sanitized.is_string() {
            sanitized = Value::String(redacted.clone());
        } else {
            redact_value(&mut sanitized, text);
        }
        if let Some(mut field) = streamed {
            if field.is_string() {
                field = Value::String(redacted);
            } else {
                redact_value(&mut field, text);
            }
            sanitized["text"] = field;
        }
        if sanitized != *payload {
            update_payload(conn, run_id, *seq, &sanitized)?;
            changed += 1;
        }
        offset = end;
    }
    Ok(changed)
}

/// Bot-scoped privacy exception to append-only payloads. SQL prefilters ordinary
/// JSON using both raw and escaped text. Streams need a separate sequential scan
/// because a forgotten phrase may span arbitrary delta boundaries. Task records
/// are rewritten only when their completed summary actually changes.
pub fn redact_payloads(conn: &Connection, bot_id: &str, text: &str) -> Result<usize> {
    require_transaction(conn)?;
    if text.is_empty() {
        return Ok(0);
    }
    let escaped = serde_json::to_string(text)?;
    let escaped = &escaped[1..escaped.len() - 1];
    let mut changed = 0;
    let mut statement = conn.prepare(
        "SELECT e.run_id,e.seq,e.payload FROM engine_run_events e
        JOIN engine_runs r ON r.run_id=e.run_id WHERE r.bot_id=?1 AND e.kind!='text_delta'
        AND (instr(e.payload,?2)>0 OR instr(e.payload,?3)>0)",
    )?;
    let mut rows = statement.query(params![bot_id, text, escaped])?;
    while let Some(row) = rows.next()? {
        let run_id: String = row.get(0)?;
        let seq: u64 = row.get(1)?;
        let mut payload: Value = serde_json::from_str(&row.get::<_, String>(2)?)?;
        let old = payload.clone();
        redact_value(&mut payload, text);
        if payload != old {
            update_payload(conn, &run_id, seq, &payload)?;
            changed += 1;
        }
    }
    let mut statement = conn.prepare(
        "SELECT e.run_id,e.seq,e.payload FROM engine_run_events e
        JOIN engine_runs r ON r.run_id=e.run_id WHERE r.bot_id=?1 AND e.kind='text_delta'
        ORDER BY e.run_id,e.seq",
    )?;
    let mut rows = statement.query([bot_id])?;
    let mut run_id = String::new();
    let mut deltas = Vec::new();
    let mut last_seq = 0;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let seq: u64 = row.get(1)?;
        if id != run_id || seq != last_seq + 1 {
            changed += redact_deltas(conn, &run_id, &deltas, text)?;
            deltas.clear();
        }
        run_id = id;
        last_seq = seq;
        deltas.push((
            seq,
            serde_json::from_str::<Value>(&row.get::<_, String>(2)?)?,
        ));
    }
    changed += redact_deltas(conn, &run_id, &deltas, text)?;
    let mut statement = conn.prepare(
        "SELECT run_id,record FROM engine_runs WHERE bot_id=?1
        AND (instr(record,?2)>0 OR instr(record,?3)>0)",
    )?;
    let mut rows = statement.query(params![bot_id, text, escaped])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let mut run: Run = serde_json::from_str(&row.get::<_, String>(1)?)?;
        if let Some(TaskStatus::Done { summary }) = &mut run.task_status {
            let sanitized = redact_string(summary, text);
            if sanitized != *summary {
                *summary = sanitized;
                conn.execute(
                    "UPDATE engine_runs SET record=?2 WHERE run_id=?1",
                    params![id, serde_json::to_string(&run)?],
                )?;
            }
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
            2
        );
        assert!(model::start(&conn, &sample()).is_err());
        Ok(())
    }

    #[test]
    fn usage_migrates_v1_preserves_runs_and_deduplicates_responses() -> Result<()> {
        let conn = database()?;
        conn.execute_batch("DROP TABLE engine_response_usage; UPDATE component_versions SET version=1 WHERE component='engine';")?;
        migrate(&conn)?;
        migrate(&conn)?;
        assert_eq!(events(&conn, "run", 0)?.len(), 1);
        assert_eq!(token_usage(&conn, "run")?, None);
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 10,
            cached_input_tokens: 80,
        };
        assert!(save_token_usage(&conn, "run", 1, &usage).is_err());
        with_tx(&conn, |conn| {
            save_token_usage(conn, "run", 1, &usage)?;
            save_token_usage(conn, "run", 1, &usage)?;
            save_token_usage(conn, "run", 2, &usage)
        })?;
        assert_eq!(
            token_usage(&conn, "run")?,
            Some(TokenUsage {
                input_tokens: 200,
                output_tokens: 20,
                cached_input_tokens: 160
            })
        );
        assert!(with_tx(&conn, |conn| save_token_usage(
            conn,
            "run",
            3,
            &TokenUsage {
                cached_input_tokens: 101,
                ..usage
            }
        ))
        .is_err());
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
            save_budget_usage_at(
                conn,
                "run",
                &BudgetUsage {
                    model_calls: 2,
                    tool_calls: 1,
                    elapsed_ms: 50,
                },
                1000,
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
            let run = model::resume_at(conn, "run", 2000)?;
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
                    elapsed_ms: 50
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
                &json!({"cause": SuspendCause::HumanInput { id: "form".into() }}),
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
            set_awaiting(
                conn,
                "run",
                &SuspendCause::HumanInput {
                    id: "approval".into(),
                },
            )?;
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
            set_awaiting(
                conn,
                "run",
                &SuspendCause::HumanInput {
                    id: "approval".into(),
                },
            )?;
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
            set_awaiting(
                conn,
                "run",
                &SuspendCause::Children {
                    ids: vec!["child".into()],
                },
            )?;
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
        let usage = BudgetUsage {
            model_calls: 2,
            tool_calls: 1,
            elapsed_ms: 10,
        };
        with_tx(&conn, |conn| {
            save_budget_usage_at(conn, "run", &usage, 1000)
        })?;
        assert!(with_tx(&conn, |conn| save_budget_usage_at(
            conn,
            "run",
            &BudgetUsage::default(),
            1000
        ))
        .is_err());
        let failed: Result<()> = with_tx(&conn, |conn| {
            save_budget_usage_at(
                conn,
                "run",
                &BudgetUsage {
                    model_calls: 3,
                    tool_calls: 1,
                    elapsed_ms: 20,
                },
                1000,
            )?;
            Err(invalid("call did not commit"))
        });
        assert!(failed.is_err());
        assert_eq!(budget_usage(&conn, "run", 0)?, usage);
        Ok(())
    }

    #[test]
    fn store_approval_wait_does_not_consume_duration_budget() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingApproval;
            save_at(conn, &run, 2000)?;
            set_awaiting(
                conn,
                "run",
                &SuspendCause::HumanInput {
                    id: "approval".into(),
                },
            )?;
            assert_eq!(budget_usage(conn, "run", 1_000_000)?.elapsed_ms, 1000);
            enqueue_wake(conn, "run", WakeCause::Approval, "approval")?;
            model::resume_at(conn, "run", 1_000_000)?;
            let usage = budget_usage(conn, "run", 1_000_500)?;
            assert_eq!(usage.elapsed_ms, 1500);
            assert!(!usage.exhausted(&run.budgets));
            save_budget_usage_at(conn, "run", &usage, 1_000_500)?;
            assert_eq!(budget_usage(conn, "run", 1_000_600)?.elapsed_ms, 1600);
            run = required(conn, "run")?;
            run.status = RunStatus::AwaitingChildren;
            save_at(conn, &run, 1_000_600)?;
            assert_eq!(budget_usage(conn, "run", 2_000_000)?.elapsed_ms, 1600);
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_queued_time_does_not_consume_duration_budget() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        migrate(&conn)?;
        with_tx(&conn, |conn| {
            let mut run = sample();
            run.status = RunStatus::Queued;
            model::start(conn, &run)?;
            assert_eq!(budget_usage(conn, "run", 1_000_000)?.elapsed_ms, 0);
            model::set_running_at(conn, "run", 1_000_000)?;
            assert_eq!(budget_usage(conn, "run", 1_000_100)?.elapsed_ms, 100);
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_interrupted_resume_keeps_unrelated_wakes_pending() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let wake = enqueue_wake(conn, "run", WakeCause::Child, "child-b")?;
            save_budget_usage_at(
                conn,
                "run",
                &BudgetUsage {
                    elapsed_ms: 50,
                    ..BudgetUsage::default()
                },
                1000,
            )?;
            interrupt_running(conn)?;
            assert_eq!(budget_usage(conn, "run", 1_000_000)?.elapsed_ms, 50);
            model::resume_at(conn, "run", 1_000_000)?;
            assert_eq!(wake_status(conn, &wake.wake_id)?, WakeStatus::Pending);
            assert_eq!(events(conn, "run", 2)?[0].payload["wake_ids"], json!([]));
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingChildren;
            save_at(conn, &run, 1_000_001)?;
            set_awaiting(
                conn,
                "run",
                &SuspendCause::Children {
                    ids: vec!["child-b".into()],
                },
            )?;
            model::resume_at(conn, "run", 2_000_000)?;
            assert!(matches!(
                wake_status(conn, &wake.wake_id)?,
                WakeStatus::Delivered { .. }
            ));
            assert_eq!(budget_usage(conn, "run", 2_000_000)?.elapsed_ms, 51);
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_suspensions_match_references_and_wait_for_all_children() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.status = RunStatus::AwaitingHumanInput;
            save_at(conn, &run, 1000)?;
            set_awaiting(
                conn,
                "run",
                &SuspendCause::HumanInput {
                    id: "form-a".into(),
                },
            )?;
            let stale = enqueue_wake(conn, "run", WakeCause::Secret, "secret-b")?;
            assert!(!model::ready_to_resume(conn, "run")?);
            assert!(model::resume_at(conn, "run", 2000).is_err());
            enqueue_wake(conn, "run", WakeCause::Form, "form-a")?;
            model::resume_at(conn, "run", 2000)?;
            assert_eq!(wake_status(conn, &stale.wake_id)?, WakeStatus::Pending);
            run = required(conn, "run")?;
            run.status = RunStatus::AwaitingChildren;
            save_at(conn, &run, 2000)?;
            set_awaiting(
                conn,
                "run",
                &SuspendCause::Children {
                    ids: vec!["a".into(), "b".into()],
                },
            )?;
            let a = enqueue_wake(conn, "run", WakeCause::Child, "a")?;
            assert!(!model::ready_to_resume(conn, "run")?);
            assert!(model::resume_at(conn, "run", 3000).is_err());
            assert_eq!(wake_status(conn, &a.wake_id)?, WakeStatus::Pending);
            let b = enqueue_wake(conn, "run", WakeCause::Child, "b")?;
            assert!(model::ready_to_resume(conn, "run")?);
            model::resume_at(conn, "run", 3000)?;
            for wake in [a, b] {
                assert!(matches!(
                    wake_status(conn, &wake.wake_id)?,
                    WakeStatus::Delivered { .. }
                ));
            }
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_redaction_preserves_short_matches_and_merges_overlaps() -> Result<()> {
        assert_eq!(
            redact_string("Fred a acheté du pain", "red"),
            "F[redacted] a acheté du pain"
        );
        assert_eq!(redact_string("aaa", "aa"), REDACTED);
        assert_eq!(redact_string("ééé", "éé"), REDACTED);
        let conn = database()?;
        with_tx(&conn, |conn| {
            append_event(
                conn,
                "run",
                RunEventKind::ToolResult,
                &json!("Fred a acheté du pain"),
            )?;
            assert_eq!(redact_payloads(conn, "bot", "red")?, 1);
            let before: i64 = conn.query_row("SELECT total_changes()", [], |row| row.get(0))?;
            assert_eq!(redact_payloads(conn, "bot", "red")?, 0);
            assert_eq!(
                conn.query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))?,
                before
            );
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_redaction_covers_text_split_across_deltas() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            append_event(
                conn,
                "run",
                RunEventKind::TextDelta,
                &json!({"text":"Noté : allergique aux ", "safe":true}),
            )?;
            append_event(
                conn,
                "run",
                RunEventKind::TextDelta,
                &json!({"text":"arachides."}),
            )?;
            assert_eq!(redact_payloads(conn, "bot", "allergique aux arachides")?, 2);
            let deltas = events(conn, "run", 1)?;
            assert_eq!(
                deltas[0].payload,
                json!({"text":"Noté : [redacted]", "safe":true})
            );
            assert_eq!(deltas[1].payload["text"], ".");
            assert_eq!((deltas[0].seq, deltas[1].seq), (2, 3));
            assert_eq!(redact_payloads(conn, "bot", "allergique aux arachides")?, 0);
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_cancelled_task_no_longer_needs_input() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let mut run = required(conn, "run")?;
            run.kind = RunKind::Task;
            run.task_status = Some(TaskStatus::Working);
            save(conn, &run)?;
            run.status = RunStatus::AwaitingHumanInput;
            run.task_status = Some(TaskStatus::NeedsInput);
            save_at(conn, &run, 1000)?;
            let cancelled = model::cancel(conn, "run", TurnEnd::Stopped)?;
            assert_eq!(cancelled.task_status, Some(TaskStatus::Working));
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_concurrent_start_requires_whole_transaction_retry() -> Result<()> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("engine-wal-{}-{nonce}.sqlite", std::process::id()));
        let first = Connection::open(&path)?;
        first.pragma_update(None, "journal_mode", "WAL")?;
        first.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&first)?;
        let second = Connection::open(&path)?;
        second.busy_timeout(std::time::Duration::from_secs(5))?;
        let stale = second.unchecked_transaction()?;
        stale.query_row("SELECT COUNT(*) FROM engine_runs", [], |row| {
            row.get::<_, i64>(0)
        })?;
        let created = with_tx(&first, |conn| model::start(conn, &sample()))?;
        let mut retry = sample();
        retry.run_id = "concurrent-candidate".into();
        let failure = model::start(&stale, &retry);
        assert!(
            matches!(failure, Err(Error::Sqlite(rusqlite::Error::SqliteFailure(error, _)))
            if error.code == rusqlite::ErrorCode::DatabaseBusy)
        );
        stale.rollback()?;
        assert_eq!(
            with_tx(&second, |conn| model::start(conn, &retry))?,
            created
        );
        assert_eq!(runs_for_conversation(&second, "conversation")?.len(), 1);
        drop(second);
        drop(first);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn store_delta_metadata_is_redacted_without_touching_split_markers() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            append_event(
                conn,
                "run",
                RunEventKind::TextDelta,
                &json!({"text":"[red", "metadata":"Fred"}),
            )?;
            append_event(
                conn,
                "run",
                RunEventKind::TextDelta,
                &json!({"text":"acted]"}),
            )?;
            assert_eq!(redact_payloads(conn, "bot", "red")?, 1);
            let deltas = events(conn, "run", 1)?;
            assert_eq!(
                deltas[0].payload,
                json!({"text":"[red", "metadata":"F[redacted]"})
            );
            assert_eq!(deltas[1].payload["text"], "acted]");
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn store_public_lookups_support_regenerate_stop_and_recovery() -> Result<()> {
        let conn = database()?;
        with_tx(&conn, |conn| {
            let trigger = sample().trigger;
            assert_eq!(
                latest_run_for_trigger(conn, &trigger)?.unwrap().run_id,
                "run"
            );
            let mut next = sample();
            next.run_id = "next".into();
            next.trigger = Trigger::UserMessage {
                message_id: "message".into(),
                attempt: 1,
            };
            model::start(conn, &next)?;
            assert_eq!(
                latest_run_for_trigger(conn, &trigger)?.unwrap().run_id,
                "next"
            );
            assert_eq!(runs_for_conversation(conn, "conversation")?.len(), 2);
            assert_eq!(
                runs_with_status(conn, RunStatus::Running)?[0].run_id,
                "next"
            );
            assert_eq!(
                runs_with_status(conn, RunStatus::Cancelled)?[0].run_id,
                "run"
            );
            Ok::<_, Error>(())
        })
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
