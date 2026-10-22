//! Durable scheduler outbox. The caller owns transactions: materialize in one
//! transaction, then create/get the engine run by `Trigger::Occurrence` and
//! `attach_run` in another. Unexpired scheduled intent and all manual intent are
//! replayed after a crash; the engine's unique trigger closes the create/attach
//! gap. Pause/confirmation is a cutoff, not a reason to catch up.
//!
//! `reports/cron-detail.md` CR16 only proves client-side manual cooldown, not
//! durable deduplication. Nonces, unique scheduled slots and the Q4 catch-up
//! policy below are local guarantees, not claims about Grok's server.
use crate::{cron, routine};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpPolicy {
    /// Minimum effective window: one minute, so a 500ms host tick need not land
    /// exactly on the scheduled millisecond. The default is still 60 minutes.
    pub late_minutes: u32,
}
impl Default for CatchUpPolicy {
    fn default() -> Self {
        Self { late_minutes: 60 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OccurrenceStatus {
    Pending,
    Running { run_id: String },
    Done { run_id: String },
    Missed,
    Failed { reason: String },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    pub occurrence_id: String,
    pub routine_id: String,
    pub schedule_version: u64,
    pub scheduled_at_utc: i64,
    pub manual_nonce: Option<String>,
    pub status: OccurrenceStatus,
}

/// At most this many individual missed slots plus one newest slot are inserted
/// per routine/tick. Remaining missed slots are preserved as compact ranges.
pub const MAX_MISSED_PER_TICK: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedRange {
    pub routine_id: String,
    pub schedule_version: u64,
    pub from_inclusive: i64,
    pub until_exclusive: i64,
    /// Snapshot of the schedule, so even ranges from before an edit are auditable.
    pub trigger: cron::Schedule,
    pub time_zone: String,
}

const COLUMNS: &str = "occurrence_id, routine_id, schedule_version, scheduled_at_utc,
    manual_nonce, status, run_id, reason";

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS component_versions (
            component TEXT PRIMARY KEY, version INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS scheduler_occurrences (
            occurrence_id TEXT PRIMARY KEY,
            routine_id TEXT NOT NULL REFERENCES scheduler_routines(routine_id),
            schedule_version INTEGER NOT NULL CHECK (schedule_version > 0),
            scheduled_at_utc INTEGER NOT NULL,
            manual_nonce TEXT,
            status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'missed', 'failed')),
            run_id TEXT UNIQUE,
            reason TEXT,
            CHECK (status NOT IN ('running', 'done') OR run_id IS NOT NULL),
            CHECK (status != 'failed' OR reason IS NOT NULL)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS scheduler_scheduled_slot
            ON scheduler_occurrences(routine_id, schedule_version, scheduled_at_utc)
            WHERE manual_nonce IS NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS scheduler_manual_nonce
            ON scheduler_occurrences(routine_id, manual_nonce) WHERE manual_nonce IS NOT NULL;
         CREATE INDEX IF NOT EXISTS scheduler_pending ON scheduler_occurrences(status);
         CREATE INDEX IF NOT EXISTS scheduler_history
            ON scheduler_occurrences(routine_id, scheduled_at_utc DESC, occurrence_id DESC);
         CREATE TABLE IF NOT EXISTS scheduler_missed_ranges (
            routine_id TEXT NOT NULL REFERENCES scheduler_routines(routine_id),
            schedule_version INTEGER NOT NULL,
            from_inclusive INTEGER NOT NULL,
            until_exclusive INTEGER NOT NULL CHECK (until_exclusive > from_inclusive),
            trigger TEXT NOT NULL,
            time_zone TEXT NOT NULL,
            PRIMARY KEY(routine_id, schedule_version, from_inclusive)
         );
         CREATE INDEX IF NOT EXISTS scheduler_range_history
            ON scheduler_missed_ranges(routine_id, from_inclusive DESC, schedule_version DESC);
         INSERT INTO component_versions(component, version)
            VALUES ('scheduler.occurrences', 1) ON CONFLICT(component) DO NOTHING;",
    )?;
    routine::check_schema(conn, "scheduler.occurrences")
}

/// Materialize bounded individual history plus compact missed ranges. Only one
/// newest, unexpired scheduled slot per routine is runnable. The cursor is the
/// last slot (not tick time), retaining unphased interval anchors across polls.
/// Caller transaction is mandatory: per-routine savepoints must not commit one.
pub fn materialize_due(
    conn: &Connection,
    now_ms: i64,
    policy: &CatchUpPolicy,
) -> Result<Vec<Occurrence>> {
    if conn.is_autocommit() {
        return Err(Error::InvalidInput(
            "materialization requires a caller-owned transaction".into(),
        ));
    }
    let ids = {
        let mut query = conn.prepare(
            "SELECT routine_id FROM scheduler_routines WHERE enabled=1 AND deleted=0 ORDER BY routine_id",
        )?;
        let rows = query.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let earliest = now_ms.saturating_sub(i64::from(policy.late_minutes.max(1)) * 60_000);
    conn.execute(
        "UPDATE scheduler_occurrences SET status='missed' WHERE status='pending' AND manual_nonce IS NULL
         AND (scheduled_at_utc < ?1 OR EXISTS (
            SELECT 1 FROM scheduler_routines r WHERE r.routine_id=scheduler_occurrences.routine_id
            AND (scheduled_at_utc <= r.active_since OR scheduler_occurrences.schedule_version!=r.schedule_version)
         ))", [earliest],
    )?;
    for id in ids {
        conn.execute_batch("SAVEPOINT scheduler_materialize_one")?;
        match materialize_one(conn, &id, now_ms, earliest) {
            Ok(()) => conn.execute_batch("RELEASE scheduler_materialize_one")?,
            Err(error) => {
                conn.execute_batch(
                    "ROLLBACK TO scheduler_materialize_one; RELEASE scheduler_materialize_one",
                )?;
                // Bad DTOs/timezones are quarantined, not allowed to poison the
                // fleet. Genuine storage failures remain loud if recording the
                // pause/error fails; they cannot safely be swallowed.
                quarantine(conn, &id, now_ms, &error.to_string())?;
            }
        }
    }
    conn.execute(
        "UPDATE scheduler_occurrences SET status='missed' WHERE status='pending' AND manual_nonce IS NULL
         AND EXISTS (SELECT 1 FROM scheduler_occurrences newer WHERE newer.routine_id=scheduler_occurrences.routine_id
            AND newer.status='pending' AND newer.manual_nonce IS NULL
            AND newer.scheduled_at_utc > scheduler_occurrences.scheduled_at_utc)", [],
    )?;
    pending(conn)
}

fn materialize_one(conn: &Connection, id: &str, now_ms: i64, earliest: i64) -> Result<()> {
    let routine = routine::require(conn, id)?;
    let cursor: i64 = conn.query_row(
        "SELECT materialized_after FROM scheduler_routines WHERE routine_id=?1",
        [id],
        |row| row.get(0),
    )?;
    // A consumed one-shot is normal exhaustion, not a broken routine.
    if matches!(routine.trigger, cron::Schedule::Once { at } if at <= cursor)
        || routine.trigger == cron::Schedule::Webhook
    {
        return Ok(());
    }
    routine::validate(&routine, cursor)?;
    let Some(first) = cron::next_after(&routine.trigger, &routine.time_zone, cursor)? else {
        return Ok(());
    };
    if first > now_ms {
        return Ok(());
    }
    let latest = latest_due(&routine.trigger, &routine.time_zone, first, now_ms)?;
    let mut slot = first;
    for _ in 0..MAX_MISSED_PER_TICK {
        if slot == latest {
            break;
        }
        insert_slot(conn, &routine, slot, "missed")?;
        slot = cron::next_after(&routine.trigger, &routine.time_zone, slot)?
            .ok_or_else(|| Error::InvalidInput("schedule exhausted before latest slot".into()))?;
    }
    if slot < latest {
        conn.execute(
            "INSERT INTO scheduler_missed_ranges
             (routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
            params![
                id,
                routine.schedule_version,
                slot,
                latest,
                serde_json::to_string(&routine.trigger)?,
                routine.time_zone
            ],
        )?;
    }
    insert_slot(
        conn,
        &routine,
        latest,
        if latest >= earliest {
            "pending"
        } else {
            "missed"
        },
    )?;
    conn.execute(
        "UPDATE scheduler_routines SET materialized_after=?2 WHERE routine_id=?1 AND schedule_version=?3",
        params![id, latest, routine.schedule_version],
    )?;
    Ok(())
}

fn insert_slot(
    conn: &Connection,
    routine: &routine::Routine,
    slot: i64,
    status: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO scheduler_occurrences
         (occurrence_id, routine_id, schedule_version, scheduled_at_utc, status)
         VALUES ('occ_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4)
         ON CONFLICT(routine_id, schedule_version, scheduled_at_utc) WHERE manual_nonce IS NULL DO NOTHING",
        params![routine.routine_id, routine.schedule_version, slot, status],
    )?;
    Ok(())
}

// Intervals jump arithmetically, preserving their original phase/anchor. Cron
// uses monotonic binary search, bounded by 64 next_after calls regardless of
// closure length or late-window size; no minute-by-minute scan of the backlog.
fn latest_due(schedule: &cron::Schedule, zone: &str, first: i64, now: i64) -> Result<i64> {
    match schedule {
        cron::Schedule::Once { .. } | cron::Schedule::Webhook => Ok(first),
        cron::Schedule::Every { interval_ms, .. } => {
            let first = i128::from(first);
            let interval = i128::from(*interval_ms);
            Ok((first + (i128::from(now) - first) / interval * interval) as i64)
        }
        cron::Schedule::Cron { .. } => {
            let (mut low, mut high) = (first, now);
            while low < high {
                let mid = (i128::from(low) + (i128::from(high) - i128::from(low) + 1) / 2) as i64;
                match cron::next_after(schedule, zone, mid - 1)? {
                    Some(next) if next <= now => low = next,
                    _ => high = mid - 1,
                }
            }
            Ok(low)
        }
    }
}

fn quarantine(conn: &Connection, id: &str, now_ms: i64, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE scheduler_routines SET enabled=0 WHERE routine_id=?1",
        [id],
    )?;
    conn.execute(
        "INSERT INTO scheduler_occurrences
         (occurrence_id, routine_id, schedule_version, scheduled_at_utc, manual_nonce, status, reason)
         SELECT 'occ_' || lower(hex(randomblob(16))), routine_id, schedule_version, ?2,
            'scheduler-error:' || ?2, 'failed', ?3 FROM scheduler_routines WHERE routine_id=?1
         ON CONFLICT DO NOTHING", params![id, now_ms, reason],
    )?;
    Ok(())
}

/// Compressed slots omitted from individual history are explicitly Missed.
/// Interpret [from_inclusive, until_exclusive) using the captured schedule.
pub fn missed_ranges(conn: &Connection, routine_id: &str) -> Result<Vec<MissedRange>> {
    let mut query = conn.prepare(
        "SELECT routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone
         FROM scheduler_missed_ranges WHERE routine_id=?1 ORDER BY from_inclusive",
    )?;
    let rows = query.query_map([routine_id], |row| {
        Ok(MissedRange {
            routine_id: row.get(0)?,
            schedule_version: row.get(1)?,
            from_inclusive: row.get(2)?,
            until_exclusive: row.get(3)?,
            trigger: routine::json(row, 4)?,
            time_zone: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Manual runs are allowed on paused routines, independently of automatic
/// slots. Nonces are unique for the routine's entire lifetime, across edits.
/// Distinct nonces at the same millisecond are distinct user intentions.
pub fn run_now(
    conn: &Connection,
    routine_id: &str,
    nonce: &str,
    now_ms: i64,
) -> Result<Occurrence> {
    if nonce.trim().is_empty() || nonce.starts_with("scheduler-error:") {
        return Err(Error::InvalidInput("manual run requires a nonce".into()));
    }
    let routine = routine::require(conn, routine_id)?;
    conn.execute(
        "INSERT INTO scheduler_occurrences
         (occurrence_id, routine_id, schedule_version, scheduled_at_utc, manual_nonce, status)
         VALUES ('occ_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, 'pending')
         ON CONFLICT(routine_id, manual_nonce) WHERE manual_nonce IS NOT NULL DO NOTHING",
        params![routine_id, routine.schedule_version, now_ms, nonce],
    )?;
    Ok(conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM scheduler_occurrences WHERE routine_id=?1 AND manual_nonce=?2"
        ),
        params![routine_id, nonce],
        from_row,
    )?)
}

pub fn get(conn: &Connection, occurrence_id: &str) -> Result<Option<Occurrence>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM scheduler_occurrences WHERE occurrence_id=?1"),
            [occurrence_id],
            from_row,
        )
        .optional()?)
}

/// Insertion watermark plus keyset position. New (even backdated) insertions
/// cannot shift a page already being traversed. Status changes remain visible.
#[derive(Clone, Copy, Debug)]
pub struct HistoryCursor {
    pub snapshot: i64,
    pub after: i64,
}

pub struct HistoryPage<T> {
    pub items: Vec<T>,
    pub cursor: HistoryCursor,
    pub next: Option<HistoryCursor>,
}

pub const HISTORY_PAGE_SIZE: usize = 30;

fn page_cursor(
    conn: &Connection,
    table: &str,
    id: &str,
    cursor: Option<HistoryCursor>,
) -> Result<HistoryCursor> {
    let cursor = match cursor {
        Some(c) => c,
        None => HistoryCursor {
            snapshot: conn.query_row(
                &format!("SELECT COALESCE(MAX(rowid),0) FROM {table}"),
                [],
                |r| r.get(0),
            )?,
            after: 0,
        },
    };
    if cursor.snapshot < 0
        || cursor.after < 0
        || cursor.after > cursor.snapshot
        || (cursor.after != 0
            && !conn.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE rowid=?1 AND routine_id=?2)"),
                params![cursor.after, id],
                |r| r.get::<_, bool>(0),
            )?)
    {
        return Err(Error::InvalidInput(
            "Curseur d’historique invalide ; rouvrez l’historique.".into(),
        ));
    }
    Ok(cursor)
}

pub fn history_page(
    conn: &Connection,
    id: &str,
    cursor: Option<HistoryCursor>,
) -> Result<HistoryPage<Occurrence>> {
    let cursor = page_cursor(conn, "scheduler_occurrences", id, cursor)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS},rowid FROM scheduler_occurrences
        WHERE routine_id=?1 AND rowid<=?2 AND (?3=0 OR (scheduled_at_utc,occurrence_id)<
        (SELECT scheduled_at_utc,occurrence_id FROM scheduler_occurrences WHERE rowid=?3))
        ORDER BY scheduled_at_utc DESC,occurrence_id DESC LIMIT ?4"
    ))?;
    let mut rows = stmt
        .query_map(
            params![id, cursor.snapshot, cursor.after, HISTORY_PAGE_SIZE + 1],
            |r| Ok((from_row(r)?, r.get::<_, i64>(8)?)),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = rows.len() > HISTORY_PAGE_SIZE;
    rows.truncate(HISTORY_PAGE_SIZE);
    let next = if more {
        rows.last().map(|(_, after)| HistoryCursor {
            after: *after,
            ..cursor
        })
    } else {
        None
    };
    Ok(HistoryPage {
        items: rows.into_iter().map(|(item, _)| item).collect(),
        cursor,
        next,
    })
}

pub fn missed_ranges_page(
    conn: &Connection,
    id: &str,
    cursor: Option<HistoryCursor>,
) -> Result<HistoryPage<MissedRange>> {
    let cursor = page_cursor(conn, "scheduler_missed_ranges", id, cursor)?;
    let mut stmt = conn.prepare("SELECT routine_id,schedule_version,from_inclusive,until_exclusive,trigger,time_zone,rowid
        FROM scheduler_missed_ranges WHERE routine_id=?1 AND rowid<=?2 AND (?3=0 OR
        (from_inclusive,schedule_version)<(SELECT from_inclusive,schedule_version FROM scheduler_missed_ranges WHERE rowid=?3))
        ORDER BY from_inclusive DESC,schedule_version DESC LIMIT ?4")?;
    let mut rows = stmt
        .query_map(
            params![id, cursor.snapshot, cursor.after, HISTORY_PAGE_SIZE + 1],
            |r| {
                Ok((
                    MissedRange {
                        routine_id: r.get(0)?,
                        schedule_version: r.get(1)?,
                        from_inclusive: r.get(2)?,
                        until_exclusive: r.get(3)?,
                        trigger: routine::json(r, 4)?,
                        time_zone: r.get(5)?,
                    },
                    r.get::<_, i64>(6)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = rows.len() > HISTORY_PAGE_SIZE;
    rows.truncate(HISTORY_PAGE_SIZE);
    let next = if more {
        rows.last().map(|(_, after)| HistoryCursor {
            after: *after,
            ..cursor
        })
    } else {
        None
    };
    Ok(HistoryPage {
        items: rows.into_iter().map(|(item, _)| item).collect(),
        cursor,
        next,
    })
}

pub fn list(conn: &Connection, routine_id: &str) -> Result<Vec<Occurrence>> {
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_occurrences WHERE routine_id=?1
         ORDER BY scheduled_at_utc, occurrence_id"
    ))?;
    let rows = query.query_map([routine_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Pausing suppresses automatic dispatch, not explicitly requested manual runs.
/// Deleting suppresses both while preserving their audit records.
pub fn pending(conn: &Connection) -> Result<Vec<Occurrence>> {
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_occurrences WHERE status='pending'
         AND routine_id IN (SELECT routine_id FROM scheduler_routines WHERE deleted=0
                            AND (enabled=1 OR scheduler_occurrences.manual_nonce IS NOT NULL))
         ORDER BY scheduled_at_utc, occurrence_id"
    ))?;
    let rows = query.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Attach only the run obtained from the engine's unique occurrence trigger.
/// A different run can never replace an existing attachment. Call in the same
/// transaction as run creation. Manual runs do not move the scheduled watermark.
pub fn attach_run(conn: &Connection, occurrence_id: &str, run_id: &str) -> Result<()> {
    if run_id.trim().is_empty() {
        return Err(Error::InvalidInput("empty run ID".into()));
    }
    let occurrence = require(conn, occurrence_id)?;
    match &occurrence.status {
        OccurrenceStatus::Running { run_id: old } | OccurrenceStatus::Done { run_id: old }
            if old == run_id => {}
        OccurrenceStatus::Pending => {
            if conn.execute(
                "UPDATE scheduler_occurrences SET status='running', run_id=?2
                 WHERE occurrence_id=?1 AND status='pending'",
                params![occurrence_id, run_id],
            )? == 0
            {
                return Err(Error::InvalidInput("occurrence attachment changed".into()));
            }
        }
        _ => {
            return Err(Error::InvalidInput(
                "occurrence is terminal or already attached".into(),
            ))
        }
    }
    if occurrence.manual_nonce.is_none() {
        conn.execute(
            "UPDATE scheduler_routines SET last_served_at=MAX(COALESCE(last_served_at, ?2), ?2)
             WHERE routine_id=?1",
            params![occurrence.routine_id, occurrence.scheduled_at_utc],
        )?;
    }
    Ok(())
}

/// Called by the wrapper's on_run_event hook; terminal updates are idempotent
/// but must refer to the attached run. A late completion cannot revive a failure.
pub fn mark_done(conn: &Connection, occurrence_id: &str, run_id: &str) -> Result<()> {
    if conn.execute(
        "UPDATE scheduler_occurrences SET status='done'
         WHERE occurrence_id=?1 AND run_id=?2 AND status IN ('running', 'done')",
        params![occurrence_id, run_id],
    )? == 0
    {
        return Err(Error::InvalidInput(
            "completion does not match an active run".into(),
        ));
    }
    Ok(())
}

pub fn mark_failed(conn: &Connection, occurrence_id: &str, reason: &str) -> Result<()> {
    let occurrence = require(conn, occurrence_id)?;
    if occurrence.status
        == (OccurrenceStatus::Failed {
            reason: reason.into(),
        })
    {
        return Ok(());
    }
    if conn.execute(
        "UPDATE scheduler_occurrences SET status='failed', reason=?2
         WHERE occurrence_id=?1 AND status IN ('pending', 'running')",
        params![occurrence_id, reason],
    )? == 0
    {
        return Err(Error::InvalidInput("occurrence is already terminal".into()));
    }
    Ok(())
}

fn require(conn: &Connection, id: &str) -> Result<Occurrence> {
    get(conn, id)?.ok_or_else(|| Error::InvalidInput("unknown occurrence".into()))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Occurrence> {
    Ok(Occurrence {
        occurrence_id: row.get(0)?,
        routine_id: row.get(1)?,
        schedule_version: row.get(2)?,
        scheduled_at_utc: row.get(3)?,
        manual_nonce: row.get(4)?,
        status: match row.get::<_, String>(5)?.as_str() {
            "pending" => OccurrenceStatus::Pending,
            "running" => OccurrenceStatus::Running {
                run_id: row.get(6)?,
            },
            "done" => OccurrenceStatus::Done {
                run_id: row.get(6)?,
            },
            "missed" => OccurrenceStatus::Missed,
            "failed" => OccurrenceStatus::Failed {
                reason: row.get(7)?,
            },
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
    })
}

#[cfg(test)]
mod occurrences_tests {
    use super::*;
    use crate::routine::{routine_tests::fixture, Routine};
    use grokbot_engine::contracts::with_tx;

    fn setup() -> (Connection, Routine) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        crate::migrate(&conn).unwrap();
        let routine = routine::create(&conn, &fixture()).unwrap();
        (conn, routine)
    }

    fn tick(conn: &Connection, now: i64) -> Vec<Occurrence> {
        with_tx(conn, |conn| {
            materialize_due(conn, now, &CatchUpPolicy::default())
        })
        .unwrap()
    }

    #[test]
    fn materialize_twice_creates_one_occurrence() {
        let (conn, routine) = setup();
        let first = tick(&conn, 60_000);
        assert_eq!(first.len(), 1);
        assert_eq!(tick(&conn, 60_000), first);
        assert_eq!(list(&conn, &routine.routine_id).unwrap(), first);
        attach_run(&conn, &first[0].occurrence_id, "run-1").unwrap();
        assert!(tick(&conn, 60_000).is_empty());
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 1);
    }

    struct TestDatabase(std::path::PathBuf);

    impl TestDatabase {
        fn new() -> Self {
            let mut bytes = [0; 16];
            getrandom::getrandom(&mut bytes).unwrap();
            let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            Self(std::env::temp_dir().join(format!("grokbot-scheduler-{token}.sqlite")))
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }

    #[test]
    fn crash_between_occurrence_and_run_does_not_duplicate() {
        use grokbot_engine::{
            contracts::{RunBudgets, RunKind, TaskStatus, Trigger},
            model::{self, Run, RunStatus},
            store,
        };
        let path = TestDatabase::new();
        let id;
        {
            let disk = Connection::open(&path.0).unwrap();
            crate::migrate(&disk).unwrap();
            store::migrate(&disk).unwrap();
            routine::create(&disk, &fixture()).unwrap();
            id = tick(&disk, 60_000)[0].occurrence_id.clone();
        } // Crash after occurrence commit, before run creation.
        let mut proposed = Run {
            run_id: "run-1".into(),
            bot_id: "bot-1".into(),
            conversation_id: "routine-thread-1".into(),
            trigger: Trigger::Occurrence {
                occurrence_id: id.clone(),
            },
            kind: RunKind::Task,
            status: RunStatus::Queued,
            task_status: Some(TaskStatus::Working),
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 60_000,
        };
        {
            let disk = Connection::open(&path.0).unwrap();
            assert_eq!(tick(&disk, 60_000)[0].occurrence_id, id);
            let rolled_back: Result<()> = with_tx(&disk, |conn| {
                let run = model::start(conn, &proposed)?;
                attach_run(conn, &id, &run.run_id)?;
                Err(Error::InvalidInput(
                    "crash before atomic run/attachment commit".into(),
                ))
            });
            assert!(rolled_back.is_err());
            assert!(store::latest_run_for_trigger(&disk, &proposed.trigger)
                .unwrap()
                .is_none());
            assert_eq!(
                get(&disk, &id).unwrap().unwrap().status,
                OccurrenceStatus::Pending
            );
            with_tx(&disk, |conn| model::start(conn, &proposed)).unwrap();
        } // Also recover a run committed before attachment (within late window).
        {
            let disk = Connection::open(&path.0).unwrap();
            assert_eq!(tick(&disk, 60_500)[0].occurrence_id, id);
            proposed.run_id = "run-retry".into();
            with_tx(&disk, |conn| -> Result<()> {
                let recovered = store::latest_run_for_trigger(conn, &proposed.trigger)?.unwrap();
                assert_eq!(model::start(conn, &proposed)?.run_id, recovered.run_id);
                attach_run(conn, &id, &recovered.run_id)
            })
            .unwrap();
            attach_run(&disk, &id, "run-1").unwrap();
            assert!(attach_run(&disk, &id, "run-other").is_err());
            assert_eq!(
                disk.query_row(
                    "SELECT COUNT(*) FROM engine_runs WHERE trigger_kind='occurrence'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
    }

    #[test]
    fn edit_does_not_refire_already_served_slot() {
        let (conn, mut routine) = setup();
        routine.trigger = cron::parse("* * * * *").unwrap();
        routine = routine::update(&conn, &routine, 0).unwrap();
        let first = tick(&conn, 60_000).remove(0);
        attach_run(&conn, &first.occurrence_id, "run-1").unwrap();
        // This includes the old slot, but the new version's cutoff excludes it.
        routine.time_zone = "Etc/UTC".into();
        routine = routine::update(&conn, &routine, 60_000).unwrap();
        assert!(tick(&conn, 60_000).is_empty());
        let next = tick(&conn, 120_000).remove(0);
        assert_eq!(next.schedule_version, routine.schedule_version);
        assert_eq!(next.scheduled_at_utc, 120_000);
        // Clock skew must not move the cutoff behind the last served slot.
        attach_run(&conn, &next.occurrence_id, "run-2").unwrap();
        routine.trigger = cron::parse("*/2 * * * *").unwrap();
        routine::update(&conn, &routine, 90_000).unwrap();
        assert!(tick(&conn, 120_000).is_empty());
        assert_eq!(tick(&conn, 240_000)[0].scheduled_at_utc, 240_000);
    }

    #[test]
    fn missed_while_closed_runs_once_within_late_window() {
        let (conn, routine) = setup();
        let runnable = tick(&conn, 7_230_000);
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].scheduled_at_utc, 7_200_000);
        let history = list(&conn, &routine.routine_id).unwrap();
        assert_eq!(history.len(), MAX_MISSED_PER_TICK + 1);
        assert_eq!(missed_ranges(&conn, &routine.routine_id).unwrap().len(), 1);
        assert!(history[..MAX_MISSED_PER_TICK]
            .iter()
            .all(|item| item.status == OccurrenceStatus::Missed));
        assert_eq!(tick(&conn, 7_230_000), runnable);
        attach_run(&conn, &runnable[0].occurrence_id, "catch-up").unwrap();
        assert!(tick(&conn, 7_230_000).is_empty());
    }

    #[test]
    fn webhook_routines_never_materialize_a_slot_and_stay_enabled() {
        let (conn, mut routine) = setup();
        routine.trigger = cron::Schedule::Webhook;
        routine::update(&conn, &routine, 0).unwrap();
        assert!(tick(&conn, 100_000_000_000).is_empty());
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
        assert!(missed_ranges(&conn, &routine.routine_id)
            .unwrap()
            .is_empty());
        assert!(
            routine::get(&conn, &routine.routine_id)
                .unwrap()
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn once_trigger_fires_once() {
        let (conn, mut routine) = setup();
        routine.trigger = cron::Schedule::Once { at: 100 };
        routine::update(&conn, &routine, 0).unwrap();
        assert!(tick(&conn, 99).is_empty());
        let only = tick(&conn, 100).remove(0);
        attach_run(&conn, &only.occurrence_id, "once").unwrap();
        mark_done(&conn, &only.occurrence_id, "once").unwrap();
        mark_done(&conn, &only.occurrence_id, "once").unwrap();
        assert!(tick(&conn, 100_000_000).is_empty());
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 1);
    }

    #[test]
    fn run_now_same_nonce_runs_once() {
        let (conn, mut routine) = setup();
        routine::set_enabled(&conn, &routine.routine_id, false, 0).unwrap();
        routine = routine::get(&conn, &routine.routine_id).unwrap().unwrap();
        let manual = run_now(&conn, &routine.routine_id, "click-1", 60_000).unwrap();
        assert_eq!(tick(&conn, 60_000), vec![manual.clone()]);
        routine.trigger = cron::Schedule::Once { at: 100_000 };
        routine.enabled = false;
        routine::update(&conn, &routine, 70_000).unwrap();
        assert_eq!(
            run_now(&conn, &routine.routine_id, "click-1", 80_000).unwrap(),
            manual
        );
        attach_run(&conn, &manual.occurrence_id, "manual").unwrap();
        assert_eq!(
            routine::get(&conn, &routine.routine_id)
                .unwrap()
                .unwrap()
                .last_served_at,
            None
        );
        assert!(matches!(
            run_now(&conn, &routine.routine_id, "click-1", 90_000)
                .unwrap()
                .status,
            OccurrenceStatus::Running { .. }
        ));
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 1);
        assert!(run_now(&conn, &routine.routine_id, "", 0).is_err());
    }

    #[test]
    fn manual_and_scheduled_identity_are_independent() {
        let (conn, routine) = setup();
        let first = run_now(&conn, &routine.routine_id, "a", 60_000).unwrap();
        let second = run_now(&conn, &routine.routine_id, "b", 60_000).unwrap();
        let pending = tick(&conn, 60_000);
        assert_eq!(pending.len(), 3);
        assert_ne!(first.occurrence_id, second.occurrence_id);
        attach_run(&conn, &first.occurrence_id, "run-a").unwrap();
        assert!(attach_run(&conn, &second.occurrence_id, "run-a").is_err());
        mark_failed(&conn, &first.occurrence_id, "cancelled").unwrap();
        mark_failed(&conn, &first.occurrence_id, "cancelled").unwrap();
        assert!(mark_done(&conn, &first.occurrence_id, "run-a").is_err());
        assert!(attach_run(&conn, &first.occurrence_id, "run-a").is_err());
        routine::delete(&conn, &routine.routine_id).unwrap();
        assert!(tick(&conn, 120_000).is_empty());
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 3);
    }

    #[test]
    fn interval_anchor_survives_frequent_ticks_and_metadata_edits() {
        let (conn, mut routine) = setup();
        for now in [1, 20_000, 59_999] {
            assert!(tick(&conn, now).is_empty());
        }
        routine.name = "New name".into();
        routine::update(&conn, &routine, 59_999).unwrap();
        let occurrence = tick(&conn, 60_000).remove(0);
        assert_eq!(occurrence.scheduled_at_utc, 60_000);
        attach_run(&conn, &occurrence.occurrence_id, "run-1").unwrap();
        assert_eq!(tick(&conn, 120_001)[0].scheduled_at_utc, 120_000);
    }

    #[test]
    fn late_window_boundary_and_expired_once_are_visible() {
        for (now, expected) in [
            (3_600_100, OccurrenceStatus::Pending),
            (3_600_101, OccurrenceStatus::Missed),
        ] {
            let (conn, mut routine) = setup();
            routine.trigger = cron::Schedule::Once { at: 100 };
            routine::update(&conn, &routine, 0).unwrap();
            tick(&conn, now);
            assert_eq!(
                list(&conn, &routine.routine_id).unwrap()[0].status,
                expected
            );
        }
    }

    #[test]
    fn resume_does_not_fire_or_miss_slots_from_pause() {
        let (conn, routine) = setup();
        let mut edit = routine.clone();
        edit.trigger = cron::parse("0 9 * * *").unwrap();
        routine::update(&conn, &edit, 0).unwrap();
        routine::set_enabled(&conn, &routine.routine_id, false, 0).unwrap();
        routine::set_enabled(&conn, &routine.routine_id, true, 34_200_000).unwrap(); // 09:30
        assert!(tick(&conn, 34_200_000).is_empty());
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
        assert!(missed_ranges(&conn, &routine.routine_id)
            .unwrap()
            .is_empty());
        let next = tick(&conn, 118_800_000); // tomorrow 09:00
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].scheduled_at_utc, 118_800_000);
    }

    #[test]
    fn q5_confirmation_reanchors_interval_without_catching_up() {
        let (conn, _) = setup();
        let mut input = fixture();
        input.routine_id = "model".into();
        input.conversation_id = "model-thread".into();
        input.origin = routine::Origin::Model;
        input.trigger = cron::parse("@every 10m").unwrap();
        routine::create(&conn, &input).unwrap();
        routine::set_enabled(&conn, "routine-1", false, 0).unwrap();
        routine::set_enabled(&conn, "model", true, 1_800_000).unwrap();
        assert!(tick(&conn, 1_800_000).is_empty());
        assert!(list(&conn, "model").unwrap().is_empty());
        // Idempotent enable must not shift the interval again.
        routine::set_enabled(&conn, "model", true, 2_000_000).unwrap();
        let next = tick(&conn, 2_400_000);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].scheduled_at_utc, 2_400_000);
        assert!(missed_ranges(&conn, "model").unwrap().is_empty());
    }

    #[test]
    fn long_closure_materialization_is_bounded() {
        for expression in ["* * * * *", "@every 1m", "@every 1m/30s"] {
            let (conn, mut routine) = setup();
            routine.trigger = cron::parse(expression).unwrap();
            routine::update(&conn, &routine, 0).unwrap();
            let now = 30 * 86_400_000 + 45_000;
            // Also bound work for a much larger-than-default late window.
            let runnable = with_tx(&conn, |conn| {
                materialize_due(
                    conn,
                    now,
                    &CatchUpPolicy {
                        late_minutes: u32::MAX,
                    },
                )
            })
            .unwrap();
            assert_eq!(runnable.len(), 1);
            let history = list(&conn, &routine.routine_id).unwrap();
            assert!(history.len() <= MAX_MISSED_PER_TICK + 1);
            let ranges = missed_ranges(&conn, &routine.routine_id).unwrap();
            assert_eq!(ranges.len(), 1);
            assert_eq!(ranges[0].trigger, routine.trigger);
            assert_eq!(ranges[0].until_exclusive, runnable[0].scheduled_at_utc);
            assert_eq!(
                ranges[0].from_inclusive,
                cron::next_after(
                    &routine.trigger,
                    "UTC",
                    history[MAX_MISSED_PER_TICK - 1].scheduled_at_utc
                )
                .unwrap()
                .unwrap()
            );
            attach_run(&conn, &runnable[0].occurrence_id, "catch-up").unwrap();
            assert!(tick(&conn, now).is_empty());
            assert_eq!(
                list(&conn, &routine.routine_id).unwrap().len(),
                history.len()
            );
            assert_eq!(missed_ranges(&conn, &routine.routine_id).unwrap(), ranges);
        }
    }

    #[test]
    fn latest_slot_search_matches_cron_sequence_across_dst() {
        for (expression, start, end) in [
            (
                "TZ=America/New_York 0,30 2 * * *",
                "2026-03-01T00:00:00Z",
                "2026-03-10T12:00:00Z",
            ),
            (
                "TZ=America/New_York 0,30 1 * * *",
                "2026-10-25T00:00:00Z",
                "2026-11-05T12:00:00Z",
            ),
            (
                "0 9 1-7 * 1",
                "2026-01-01T00:00:00Z",
                "2026-02-01T00:00:00Z",
            ),
        ] {
            let schedule = cron::parse(expression).unwrap();
            let start = start.parse::<jiff::Timestamp>().unwrap().as_millisecond();
            let end = end.parse::<jiff::Timestamp>().unwrap().as_millisecond();
            let first = cron::next_after(&schedule, "UTC", start).unwrap().unwrap();
            let mut expected = first;
            while let Some(next) = cron::next_after(&schedule, "UTC", expected).unwrap() {
                if next > end {
                    break;
                }
                expected = next;
            }
            assert_eq!(latest_due(&schedule, "UTC", first, end).unwrap(), expected);
        }
    }

    #[test]
    fn bad_routine_does_not_block_others() {
        for assignment in ["time_zone='Gone/Zone'", "trigger='not json'", "body='{}'"] {
            let (conn, routine) = setup();
            let mut healthy = fixture();
            healthy.routine_id = "z".into();
            healthy.conversation_id = "healthy-thread".into();
            routine::create(&conn, &healthy).unwrap();
            // Controlled corruption of one row; production inputs stay parameterized.
            conn.execute(
                &format!("UPDATE scheduler_routines SET {assignment} WHERE routine_id=?1"),
                [&routine.routine_id],
            )
            .unwrap();
            let runnable = tick(&conn, 60_000);
            assert_eq!(runnable.len(), 1);
            assert_eq!(runnable[0].routine_id, "z");
            let enabled: bool = conn
                .query_row(
                    "SELECT enabled FROM scheduler_routines WHERE routine_id=?1",
                    [&routine.routine_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!enabled);
            let history = list(&conn, &routine.routine_id).unwrap();
            assert!(
                matches!(&history[0].status, OccurrenceStatus::Failed { reason } if !reason.is_empty())
            );
            tick(&conn, 60_000);
            assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 1);
        }
    }

    #[test]
    fn bad_routine_partial_materialization_rolls_back_to_savepoint() {
        let (conn, routine) = setup();
        let mut healthy = fixture();
        healthy.routine_id = "z".into();
        healthy.conversation_id = "healthy-thread".into();
        routine::create(&conn, &healthy).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_one_pending BEFORE INSERT ON scheduler_occurrences
            WHEN NEW.routine_id='routine-1' AND NEW.status='pending'
            BEGIN SELECT RAISE(ABORT, 'simulate failed final insert'); END",
        )
        .unwrap();
        assert_eq!(tick(&conn, 120_000)[0].routine_id, "z");
        let history = list(&conn, &routine.routine_id).unwrap();
        assert_eq!(history.len(), 1);
        assert!(matches!(history[0].status, OccurrenceStatus::Failed { .. }));
        let cursor: i64 = conn
            .query_row(
                "SELECT materialized_after FROM scheduler_routines WHERE routine_id=?1",
                [&routine.routine_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, 0);
    }

    #[test]
    fn expired_or_superseded_pending_is_missed_but_manual_intent_survives() {
        let (conn, routine) = setup();
        let first = tick(&conn, 60_000).remove(0);
        let newer = tick(&conn, 120_000).remove(0);
        assert_eq!(
            get(&conn, &first.occurrence_id).unwrap().unwrap().status,
            OccurrenceStatus::Missed
        );
        let manual = run_now(&conn, &routine.routine_id, "manual", 120_000).unwrap();
        let pending = tick(&conn, 7_200_000);
        assert_eq!(pending.len(), 2);
        assert!(pending
            .iter()
            .any(|item| item.occurrence_id == manual.occurrence_id));
        assert_eq!(
            get(&conn, &newer.occurrence_id).unwrap().unwrap().status,
            OccurrenceStatus::Missed
        );
        routine::set_enabled(&conn, &routine.routine_id, false, 7_200_000).unwrap();
        routine::set_enabled(&conn, &routine.routine_id, true, 8_000_000).unwrap();
        assert_eq!(tick(&conn, 8_000_000), vec![manual]);
    }

    #[test]
    fn zero_late_window_tolerates_host_tick_delay() {
        let (conn, _) = setup();
        let pending = with_tx(&conn, |conn| {
            materialize_due(conn, 60_500, &CatchUpPolicy { late_minutes: 0 })
        })
        .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].scheduled_at_utc, 60_000);
    }

    #[test]
    fn occurrence_migration_rejects_unknown_schema() {
        let (conn, _) = setup();
        conn.execute(
            "UPDATE component_versions SET version=2 WHERE component='scheduler.occurrences'",
            [],
        )
        .unwrap();
        assert!(migrate(&conn).is_err());
    }

    #[test]
    fn materialization_rejects_autocommit_without_starting_a_transaction() {
        let (conn, routine) = setup();
        assert!(materialize_due(&conn, 60_000, &CatchUpPolicy::default()).is_err());
        assert!(conn.is_autocommit());
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
    }

    #[test]
    fn rollback_includes_occurrences_and_cursor() {
        let (conn, routine) = setup();
        let result: Result<()> = with_tx(&conn, |conn| {
            materialize_due(conn, 60_000, &CatchUpPolicy::default())?;
            Err(Error::InvalidInput("simulate failure".into()))
        });
        assert!(result.is_err());
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
        assert_eq!(tick(&conn, 60_000).len(), 1);
    }
}
