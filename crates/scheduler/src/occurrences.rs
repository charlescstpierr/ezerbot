//! Durable scheduler outbox. The caller owns transactions: materialize in one
//! transaction, then create/get the engine run by `Trigger::Occurrence` and
//! `attach_run` in another. Pending rows are replayed until attached, including
//! after a crash; the engine's unique trigger closes the create/attach gap.
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
         INSERT INTO component_versions(component, version)
            VALUES ('scheduler.occurrences', 1) ON CONFLICT(component) DO NOTHING;",
    )?;
    Ok(())
}

/// Materialize every due slot, but only the newest slot within the inclusive
/// late window is runnable. All other newly discovered slots remain visible as
/// Missed. Progress is a durable schedule cursor, not the tick time: unphased
/// `@every` stays anchored and frequent polling cannot postpone it forever.
/// Replays return *all* pending intent, even if its late window has since passed.
pub fn materialize_due(
    conn: &Connection,
    now_ms: i64,
    policy: &CatchUpPolicy,
) -> Result<Vec<Occurrence>> {
    let ids = {
        let mut query = conn.prepare(
            "SELECT routine_id, materialized_after FROM scheduler_routines
             WHERE enabled=1 AND deleted=0 ORDER BY routine_id",
        )?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let earliest = now_ms.saturating_sub(i64::from(policy.late_minutes) * 60_000);
    for (id, mut cursor) in ids {
        let routine = routine::require(conn, &id)?;
        while let Some(slot) = cron::next_after(&routine.trigger, &routine.time_zone, cursor)? {
            if slot > now_ms {
                break;
            }
            // One-slot lookahead keeps memory bounded even after a long closure.
            let has_later = cron::next_after(&routine.trigger, &routine.time_zone, slot)?
                .is_some_and(|next| next <= now_ms);
            let status = if !has_later && slot >= earliest {
                "pending"
            } else {
                "missed"
            };
            conn.execute(
                "INSERT INTO scheduler_occurrences
                 (occurrence_id, routine_id, schedule_version, scheduled_at_utc, status)
                 VALUES ('occ_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4)
                 ON CONFLICT DO NOTHING",
                params![id, routine.schedule_version, slot, status],
            )?;
            cursor = slot;
            conn.execute(
                "UPDATE scheduler_routines SET materialized_after=?2
                 WHERE routine_id=?1 AND schedule_version=?3",
                params![id, cursor, routine.schedule_version],
            )?;
        }
    }
    pending(conn)
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
    if nonce.trim().is_empty() {
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

    #[test]
    fn crash_between_occurrence_and_run_does_not_duplicate() {
        // A tiny trigger-keyed fake run store isolates this slice from the
        // engine-model slice. The wrapper integration verifies the real engine.
        let (conn, _) = setup();
        let token: String = conn
            .query_row("SELECT hex(randomblob(16))", [], |row| row.get(0))
            .unwrap();
        let path = std::env::temp_dir().join(format!("grokbot-scheduler-{token}.sqlite"));
        let id;
        {
            let disk = Connection::open(&path).unwrap();
            crate::migrate(&disk).unwrap();
            routine::create(&disk, &fixture()).unwrap();
            disk.execute_batch(
                "CREATE TABLE fake_runs (id INTEGER PRIMARY KEY, occurrence TEXT UNIQUE)",
            )
            .unwrap();
            id = tick(&disk, 60_000)[0].occurrence_id.clone();
        } // Crash after occurrence commit, before run creation.
        {
            let disk = Connection::open(&path).unwrap();
            assert_eq!(tick(&disk, 60_000)[0].occurrence_id, id);
            disk.execute("INSERT INTO fake_runs(occurrence) VALUES (?1)", [&id])
                .unwrap();
        } // Also simulate a run committed before attachment.
        {
            let disk = Connection::open(&path).unwrap();
            let replay = tick(&disk, 7_200_000);
            assert!(replay
                .iter()
                .any(|occurrence| occurrence.occurrence_id == id));
            with_tx(&disk, |conn| -> Result<()> {
                conn.execute(
                    "INSERT INTO fake_runs(occurrence) VALUES (?1) ON CONFLICT DO NOTHING",
                    [&id],
                )?;
                let run: i64 = conn.query_row(
                    "SELECT id FROM fake_runs WHERE occurrence=?1",
                    [&id],
                    |row| row.get(0),
                )?;
                attach_run(conn, &id, &format!("run-{run}"))
            })
            .unwrap();
            attach_run(&disk, &id, "run-1").unwrap();
            assert!(attach_run(&disk, &id, "run-other").is_err());
            assert_eq!(
                disk.query_row("SELECT COUNT(*) FROM fake_runs", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        std::fs::remove_file(path).unwrap();
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
        assert_eq!(history.len(), 120);
        assert!(history[..119]
            .iter()
            .all(|item| item.status == OccurrenceStatus::Missed));
        assert_eq!(tick(&conn, 7_230_000), runnable);
        attach_run(&conn, &runnable[0].occurrence_id, "catch-up").unwrap();
        assert!(tick(&conn, 7_230_000).is_empty());
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
        routine::set_enabled(&conn, &routine.routine_id, false).unwrap();
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
