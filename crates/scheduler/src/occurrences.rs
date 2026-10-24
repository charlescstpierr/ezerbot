//! Durable scheduler outbox. The caller owns transactions: materialize in one
//! transaction, then create/get the engine run by `Trigger::Occurrence` and
//! `attach_run` in another. Unexpired scheduled intent and all manual intent are
//! replayed after a crash; the engine's unique trigger closes the create/attach
//! gap. Pause/confirmation is a cutoff, not a reason to catch up.
//!
//! `reports/cron-detail.md` CR16 only proves client-side manual cooldown, not
//! durable deduplication. Nonces, unique scheduled slots and the Q4 catch-up
//! policy below are local guarantees, not claims about Grok's server.
//!
//! Catch-up (docs/specs/routines.md, « Rattrapage »). A slot's identity is the
//! routine, its schedule version and the planned instant: the unique
//! `scheduler_scheduled_slot` index claims it once, so a second wake, a second
//! tick or a future remote executor finds it claimed and never starts it again.
//! Three kinds of due slot exist at a tick:
//! - ordinary: no gap since the last tick and no deliberate absence. The 60-minute
//!   window applies, and only the newest slot is runnable;
//! - gap: the host did not tick for longer than [`TICK_GAP_MS`] while it was meant
//!   to run (Mac off or asleep, agent crashed). Every slot due in the gap queues
//!   as a pending, `catch_up` occurrence and starts in chronological order;
//! - deliberate absence: the app was quit while the background agent was off
//!   ([`begin_absence`]). The limited rule applies: the newest slot if it falls in
//!   the last 60 minutes, the others are missed.
//!
//! A group (docs/specs/routines.md, « Groupe ») makes each simple trigger its own
//! source of slots: the identity of a slot also names the member, whose version
//! and cursor live in `scheduler_members`. Editing one member leaves the pending
//! slots of another untouched.
//!
//! Time is always passed in, never read, so the "was meant to run" fact and the
//! clock are injectable.
use crate::{cron, routine};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock, PoisonError},
};

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
    /// The simple trigger of a group that started this occurrence; empty outside a group.
    #[serde(default)]
    pub member_id: String,
}

/// At most this many individual missed slots plus one newest slot are inserted
/// per routine/tick. Remaining missed slots are preserved as compact ranges.
/// This bounds only the limited rule; a gap queues every slot it contains.
pub const MAX_MISSED_PER_TICK: usize = 100;

/// A host that ticks every 500 ms and an agent that passes every minute never
/// leave a longer silence while meant to run. Longer means the Mac was off or asleep.
pub const TICK_GAP_MS: i64 = 120_000;
/// How long a closed deliberate absence is remembered, for slots told after it ended.
const ABSENCE_KEPT_MS: i64 = 30 * 24 * 3_600_000;
/// The heartbeat is written at most this often, so an idle app does not take the
/// write lock on every 500 ms tick.
const HEARTBEAT_EVERY_MS: i64 = 5_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissedRange {
    pub routine_id: String,
    #[serde(default)]
    pub member_id: String,
    pub schedule_version: u64,
    pub from_inclusive: i64,
    pub until_exclusive: i64,
    /// Snapshot of the schedule, so even ranges from before an edit are auditable.
    pub trigger: cron::Schedule,
    pub time_zone: String,
}

const COLUMNS: &str = "occurrence_id, routine_id, schedule_version, scheduled_at_utc,
    manual_nonce, status, run_id, reason, member_id";

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
            catch_up INTEGER NOT NULL DEFAULT 0 CHECK (catch_up IN (0, 1)),
            member_id TEXT NOT NULL DEFAULT '',
            slot_key TEXT NOT NULL DEFAULT '',
            CHECK (status NOT IN ('running', 'done') OR run_id IS NOT NULL),
            CHECK (status != 'failed' OR reason IS NOT NULL)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS scheduler_manual_nonce
            ON scheduler_occurrences(routine_id, manual_nonce) WHERE manual_nonce IS NOT NULL;
         CREATE INDEX IF NOT EXISTS scheduler_pending ON scheduler_occurrences(status);
         CREATE INDEX IF NOT EXISTS scheduler_history
            ON scheduler_occurrences(routine_id, scheduled_at_utc DESC, occurrence_id DESC);
         CREATE TABLE IF NOT EXISTS scheduler_missed_ranges (
            routine_id TEXT NOT NULL REFERENCES scheduler_routines(routine_id),
            member_id TEXT NOT NULL DEFAULT '',
            schedule_version INTEGER NOT NULL,
            from_inclusive INTEGER NOT NULL,
            until_exclusive INTEGER NOT NULL CHECK (until_exclusive > from_inclusive),
            trigger TEXT NOT NULL,
            time_zone TEXT NOT NULL,
            PRIMARY KEY(routine_id, member_id, schedule_version, from_inclusive)
         );
         CREATE INDEX IF NOT EXISTS scheduler_range_history
            ON scheduler_missed_ranges(routine_id, from_inclusive DESC, schedule_version DESC);
         CREATE TABLE IF NOT EXISTS scheduler_presence (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_tick_at INTEGER,
            absent_since INTEGER
         );
         INSERT OR IGNORE INTO scheduler_presence(id) VALUES (1);
         CREATE TABLE IF NOT EXISTS scheduler_absences (
            from_ms INTEGER NOT NULL, until_ms INTEGER NOT NULL
         );
         INSERT INTO component_versions(component, version)
            VALUES ('scheduler.occurrences', 1) ON CONFLICT(component) DO NOTHING;",
    )?;
    // Upgrade databases made before catch-up existed: their pending slots keep
    // the ordinary window.
    let has_catch_up = conn
        .prepare("PRAGMA table_info(scheduler_occurrences)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "catch_up");
    if !has_catch_up {
        conn.execute_batch(
            "ALTER TABLE scheduler_occurrences
             ADD COLUMN catch_up INTEGER NOT NULL DEFAULT 0 CHECK (catch_up IN (0, 1));",
        )?;
    }
    // Upgrade databases made before groups existed: every old slot belongs to
    // the routine itself (empty member), and its identity gains the member.
    let columns = |table: &str| -> Result<Vec<String>> {
        Ok(conn
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    };
    if !columns("scheduler_occurrences")?
        .iter()
        .any(|name| name == "member_id")
    {
        conn.execute_batch(
            "ALTER TABLE scheduler_occurrences ADD COLUMN member_id TEXT NOT NULL DEFAULT '';",
        )?;
    }
    if !columns("scheduler_missed_ranges")?
        .iter()
        .any(|name| name == "member_id")
    {
        conn.execute_batch(
            "DROP INDEX IF EXISTS scheduler_range_history;
             ALTER TABLE scheduler_missed_ranges RENAME TO scheduler_missed_ranges_old;
             CREATE TABLE scheduler_missed_ranges (
                routine_id TEXT NOT NULL REFERENCES scheduler_routines(routine_id),
                member_id TEXT NOT NULL DEFAULT '',
                schedule_version INTEGER NOT NULL,
                from_inclusive INTEGER NOT NULL,
                until_exclusive INTEGER NOT NULL CHECK (until_exclusive > from_inclusive),
                trigger TEXT NOT NULL,
                time_zone TEXT NOT NULL,
                PRIMARY KEY(routine_id, member_id, schedule_version, from_inclusive)
             );
             INSERT INTO scheduler_missed_ranges
                (routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone)
                SELECT routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone
                FROM scheduler_missed_ranges_old;
             DROP TABLE scheduler_missed_ranges_old;
             CREATE INDEX scheduler_range_history
                ON scheduler_missed_ranges(routine_id, from_inclusive DESC, schedule_version DESC);",
        )?;
    }
    // Upgrade databases made before a slot could be told by the outside: the
    // identity of a slot gains its key, empty for every slot the clock counted.
    if !columns("scheduler_occurrences")?
        .iter()
        .any(|name| name == "slot_key")
    {
        conn.execute_batch(
            "ALTER TABLE scheduler_occurrences ADD COLUMN slot_key TEXT NOT NULL DEFAULT '';",
        )?;
    }
    let keyed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index'
         AND name='scheduler_scheduled_member_slot' AND sql LIKE '%slot_key%')",
        [],
        |row| row.get(0),
    )?;
    if !keyed {
        conn.execute_batch(
            "DROP INDEX IF EXISTS scheduler_scheduled_slot;
             DROP INDEX IF EXISTS scheduler_scheduled_member_slot;
             CREATE UNIQUE INDEX scheduler_scheduled_member_slot
                ON scheduler_occurrences(routine_id, member_id, schedule_version, scheduled_at_utc, slot_key)
                WHERE manual_nonce IS NULL;",
        )?;
    }
    routine::check_schema(conn, "scheduler.occurrences")
}

/// SQL for the version an occurrence has to match to still be current: the
/// routine's for a lone trigger, the member's own inside a group (0, never
/// current, once the member has left). `o` and `r` are the aliases of the
/// occurrence and of its routine.
pub fn current_version_sql(o: &str, r: &str) -> String {
    format!(
        "(CASE WHEN {o}.member_id='' THEN {r}.schedule_version ELSE COALESCE((SELECT m.version
         FROM scheduler_members m WHERE m.routine_id={o}.routine_id AND m.member_id={o}.member_id
         AND m.live=1), 0) END)"
    )
}

/// Whether the trigger that made `occurrence` is still the one it was made by.
pub fn is_current(conn: &Connection, occurrence: &Occurrence) -> Result<bool> {
    let current: Option<u64> = if occurrence.member_id.is_empty() {
        conn.query_row(
            "SELECT schedule_version FROM scheduler_routines WHERE routine_id=?1 AND deleted=0",
            [&occurrence.routine_id],
            |row| row.get(0),
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT m.version FROM scheduler_members m JOIN scheduler_routines r
             ON r.routine_id=m.routine_id AND r.deleted=0
             WHERE m.routine_id=?1 AND m.member_id=?2 AND m.live=1",
            params![occurrence.routine_id, occurrence.member_id],
            |row| row.get(0),
        )
        .optional()?
    };
    Ok(current == Some(occurrence.schedule_version))
}

/// Materialize due slots. Ordinary and absence slots keep bounded individual
/// history plus compact missed ranges, and only one newest, unexpired scheduled
/// slot per routine is runnable. Slots due in a gap all queue (see the module
/// doc). The cursor is the last slot (not tick time), retaining unphased interval
/// anchors across polls. Caller transaction is mandatory: per-routine savepoints
/// must not commit one.
pub fn materialize_due(
    conn: &Connection,
    now_ms: i64,
    policy: &CatchUpPolicy,
) -> Result<Vec<Occurrence>> {
    materialize(conn, now_ms, policy)?;
    pending(conn)
}

/// [`materialize_due`] without reading the whole queue back: a host that only
/// dispatches the head of each queue uses [`pending_heads`] afterwards.
pub fn materialize(conn: &Connection, now_ms: i64, policy: &CatchUpPolicy) -> Result<()> {
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
    // The window never expires a queued catch-up slot: it was owed, not late.
    conn.execute(
        &format!("UPDATE scheduler_occurrences SET status='missed' WHERE status='pending' AND manual_nonce IS NULL
         AND ((catch_up=0 AND scheduled_at_utc < ?1) OR EXISTS (
            SELECT 1 FROM scheduler_routines r WHERE r.routine_id=scheduler_occurrences.routine_id
            AND (scheduled_at_utc <= r.active_since OR scheduler_occurrences.schedule_version!={})
         ))", current_version_sql("scheduler_occurrences", "r")), [earliest],
    )?;
    let presence = Presence::read(conn)?;
    for id in ids {
        conn.execute_batch("SAVEPOINT scheduler_materialize_one")?;
        match materialize_one(conn, &id, now_ms, earliest, &presence) {
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
    // Only the newest ordinary slot stays runnable. A queued catch-up slot is
    // never superseded: every slot of a gap runs, in order.
    conn.execute(
        "UPDATE scheduler_occurrences SET status='missed' WHERE status='pending' AND manual_nonce IS NULL AND catch_up=0
         AND EXISTS (SELECT 1 FROM scheduler_occurrences newer WHERE newer.routine_id=scheduler_occurrences.routine_id
            AND newer.member_id=scheduler_occurrences.member_id
            AND newer.status='pending' AND newer.manual_nonce IS NULL AND newer.catch_up=0
            AND newer.scheduled_at_utc > scheduler_occurrences.scheduled_at_utc)", [],
    )?;
    Ok(())
}

/// What the host knows about itself between ticks. Both fields are written by
/// the host (see [`record_tick`] and [`begin_absence`]), never inferred here
/// from a sleep log.
struct Presence {
    last_tick_at: Option<i64>,
    absent_since: Option<i64>,
}

impl Presence {
    fn read(conn: &Connection) -> Result<Self> {
        Ok(conn
            .query_row(
                "SELECT last_tick_at, absent_since FROM scheduler_presence WHERE id=1",
                [],
                |row| {
                    Ok(Self {
                        last_tick_at: row.get(0)?,
                        absent_since: row.get(1)?,
                    })
                },
            )
            .optional()?
            .unwrap_or(Self {
                last_tick_at: None,
                absent_since: None,
            }))
    }
}

/// The user quit the app while the background agent was off: nothing was asked
/// to run until the next tick. Slots due after `at_ms` follow the limited rule.
/// The earliest start wins, so a repeated call cannot shorten an absence.
pub fn begin_absence(conn: &Connection, at_ms: i64) -> Result<()> {
    conn.execute(
        "UPDATE scheduler_presence SET absent_since=MIN(COALESCE(absent_since, ?1), ?1) WHERE id=1",
        [at_ms],
    )?;
    Ok(())
}

/// The host ticked at `now_ms`. Call it after [`materialize`] for the same tick:
/// the gap and the absence are read from the previous values. It ends a
/// deliberate absence, and is written at most every few seconds otherwise.
pub fn record_tick(conn: &Connection, now_ms: i64) -> Result<()> {
    let stale: bool = conn.query_row(
        "SELECT COALESCE(absent_since IS NOT NULL OR last_tick_at IS NULL
            OR ABS(?1 - last_tick_at) >= ?2, 1) FROM scheduler_presence WHERE id=1",
        params![now_ms, HEARTBEAT_EVERY_MS],
        |row| row.get(0),
    )?;
    if stale {
        // The absence ends here, but a slot told later can still fall inside it.
        conn.execute(
            "INSERT INTO scheduler_absences(from_ms, until_ms)
             SELECT absent_since, MAX(?1, absent_since) FROM scheduler_presence
             WHERE id=1 AND absent_since IS NOT NULL",
            [now_ms],
        )?;
        conn.execute(
            "DELETE FROM scheduler_absences WHERE until_ms < ?1",
            [now_ms.saturating_sub(ABSENCE_KEPT_MS)],
        )?;
        conn.execute(
            "UPDATE scheduler_presence SET last_tick_at=?1, absent_since=NULL WHERE id=1",
            [now_ms],
        )?;
    }
    Ok(())
}

/// A pulse from a host that is busy and not materializing, such as a background
/// pass waiting on a long run. It never advances the last tick across a gap: the
/// slots of a sleep during the run must still be seen as a gap by the next
/// [`materialize`], which [`record_tick`] alone is entitled to close.
pub fn heartbeat(conn: &Connection, now_ms: i64) -> Result<()> {
    let silent: bool = conn.query_row(
        "SELECT COALESCE(?1 - last_tick_at > ?2, 0) FROM scheduler_presence WHERE id=1",
        params![now_ms, TICK_GAP_MS],
        |row| row.get(0),
    )?;
    if silent {
        return Ok(());
    }
    record_tick(conn, now_ms)
}

/// One stream of slots: a lone trigger, or one simple trigger of a group. Each
/// has its own schedule version and its own cursor.
struct Source {
    member_id: String,
    version: u64,
    trigger: cron::Schedule,
    cursor: i64,
}

fn sources(conn: &Connection, routine: &routine::Routine) -> Result<Vec<Source>> {
    match &routine.trigger {
        trigger if trigger.is_delivered() => Ok(vec![]),
        cron::Schedule::Group { .. } => {
            let mut query = conn.prepare(
                "SELECT member_id, version, trigger, materialized_after FROM scheduler_members
                 WHERE routine_id=?1 AND live=1 ORDER BY rowid",
            )?;
            let rows = query.query_map([&routine.routine_id], |row| {
                Ok(Source {
                    member_id: row.get(0)?,
                    version: row.get(1)?,
                    trigger: routine::json(row, 2)?,
                    cursor: row.get(3)?,
                })
            })?;
            Ok(rows
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|source| !source.trigger.is_delivered())
                .collect())
        }
        trigger => Ok(vec![Source {
            member_id: String::new(),
            version: routine.schedule_version,
            trigger: trigger.clone(),
            cursor: conn.query_row(
                "SELECT materialized_after FROM scheduler_routines WHERE routine_id=?1",
                [&routine.routine_id],
                |row| row.get(0),
            )?,
        }]),
    }
}

fn materialize_one(
    conn: &Connection,
    id: &str,
    now_ms: i64,
    earliest: i64,
    presence: &Presence,
) -> Result<()> {
    let routine = routine::require(conn, id)?;
    for source in sources(conn, &routine)? {
        materialize_source(conn, &routine, &source, now_ms, earliest, presence)?;
    }
    Ok(())
}

fn materialize_source(
    conn: &Connection,
    routine: &routine::Routine,
    source: &Source,
    now_ms: i64,
    earliest: i64,
    presence: &Presence,
) -> Result<()> {
    let cursor = source.cursor;
    // A consumed one-shot is normal exhaustion, not a broken routine.
    if matches!(source.trigger, cron::Schedule::Once { at } if at <= cursor) {
        return Ok(());
    }
    if source.member_id.is_empty() {
        routine::validate(routine, cursor)?;
    } else {
        routine::validate_trigger(&source.trigger, &routine.time_zone, cursor)?;
    }
    let Some(first) = cron::next_after(&source.trigger, &routine.time_zone, cursor)? else {
        return Ok(());
    };
    if first > now_ms {
        return Ok(());
    }
    let (trigger, zone) = (&source.trigger, routine.time_zone.as_str());
    // Time splits into ordinary, gap and absence, in that order. A slot is in
    // the gap if it is after the last tick and the host stayed silent too long.
    let absent_from = presence.absent_since.unwrap_or(i64::MAX);
    let gap_from = presence
        .last_tick_at
        .filter(|tick| now_ms.saturating_sub(*tick) > TICK_GAP_MS)
        .unwrap_or(i64::MAX)
        .min(absent_from);
    let ordinary_until = gap_from.min(now_ms);
    let gap_until = absent_from.min(now_ms);
    let (mut last, mut next) = (first, Some(first));
    if first <= ordinary_until {
        let latest = latest_due(trigger, zone, first, ordinary_until)?;
        settle_limited(conn, routine, source, first, latest, earliest)?;
        (last, next) = (latest, cron::next_after(trigger, zone, latest)?);
    }
    while let Some(slot) = next.filter(|slot| *slot <= gap_until) {
        insert_slot(conn, routine, source, slot, "pending", true)?;
        (last, next) = (slot, cron::next_after(trigger, zone, slot)?);
    }
    if let Some(slot) = next.filter(|slot| *slot <= now_ms) {
        last = latest_due(trigger, zone, slot, now_ms)?;
        settle_limited(conn, routine, source, slot, last, earliest)?;
    }
    if source.member_id.is_empty() {
        conn.execute(
            "UPDATE scheduler_routines SET materialized_after=?2 WHERE routine_id=?1 AND schedule_version=?3",
            params![routine.routine_id, last, source.version],
        )?;
    } else {
        conn.execute(
            "UPDATE scheduler_members SET materialized_after=?3 WHERE routine_id=?1 AND member_id=?2 AND version=?4",
            params![routine.routine_id, source.member_id, last, source.version],
        )?;
    }
    Ok(())
}

/// The earliest slot that any trigger of `routine` has yet to materialize, from
/// each trigger's own cursor. A host that only wants to know whether to wake up
/// asks this instead of running a tick.
pub fn next_slot(conn: &Connection, routine: &routine::Routine) -> Result<Option<i64>> {
    let mut first: Option<i64> = None;
    for source in sources(conn, routine)? {
        if let Some(next) = cron::next_after(&source.trigger, &routine.time_zone, source.cursor)? {
            first = Some(first.map_or(next, |current| current.min(next)));
        }
    }
    Ok(first)
}

/// A trigger's future slots, walked once and kept so that a read is a binary search.
struct Plan {
    from: i64,
    until: i64,
    slots: Vec<i64>,
}
/// How far past the read that walked it a plan reaches: a plan answers a 7-day question for a week.
const PLAN_HORIZON_MS: i64 = 14 * 86_400_000;
/// Slots walked from a cursor to the horizon, a long-closed app's backlog included. An interval is at
/// least a minute, so the horizon alone holds at most 20 160.
const MAX_WALKED_SLOTS: u64 = 100_000;
const MAX_PLANS: usize = 512;
type PlanKey = (String, String, String, String, Option<i64>);
static PLANS: OnceLock<Mutex<HashMap<PlanKey, Plan>>> = OnceLock::new();

/// How many slots the triggers of `routine` plan in `(now_ms, now_ms + span_ms]` (`span_ms` at most
/// a week), each trigger counted on its own cursor as [`materialize`] does: a group's members are
/// separate streams, and an interval keeps its anchor. A paused routine plans none. `None` when a
/// trigger's backlog is too long to walk. The walk is the scheduler's own `next_after`, so weekdays and zone
/// changes count as they will run; it is kept per trigger, and an interval is keyed by its anchor
/// rather than its cursor, which moves with every slot.
pub fn planned(
    conn: &Connection,
    routine: &routine::Routine,
    now_ms: i64,
    span_ms: i64,
) -> Result<Option<u64>> {
    if !routine.enabled {
        return Ok(Some(0));
    }
    let plans = PLANS.get_or_init(Default::default);
    let lock = || plans.lock().unwrap_or_else(PoisonError::into_inner);
    let mut planned = 0;
    for source in sources(conn, routine)? {
        let anchor = match &source.trigger {
            cron::Schedule::Every {
                interval_ms,
                phase_ms: None,
            } => Some(
                source
                    .cursor
                    .rem_euclid(i64::try_from(*interval_ms).unwrap_or(i64::MAX)),
            ),
            _ => None,
        };
        let key = (
            routine.routine_id.clone(),
            source.member_id.clone(),
            serde_json::to_string(&source.trigger)?,
            routine.time_zone.clone(),
            anchor,
        );
        let usable = |plan: &Plan| plan.from <= now_ms && now_ms + span_ms <= plan.until;
        if !lock().get(&key).is_some_and(usable) {
            let Some(plan) = walk(routine, &source, now_ms)? else {
                return Ok(None);
            };
            let mut plans = lock();
            if plans.len() >= MAX_PLANS {
                plans.clear();
            }
            plans.insert(key.clone(), plan);
        }
        let plans = lock();
        let slots = &plans[&key].slots;
        let (before, until) = (
            slots.partition_point(|slot| *slot <= now_ms),
            slots.partition_point(|slot| *slot <= now_ms + span_ms),
        );
        planned += (until - before) as u64;
    }
    Ok(Some(planned))
}

fn walk(routine: &routine::Routine, source: &Source, now_ms: i64) -> Result<Option<Plan>> {
    let until = now_ms + PLAN_HORIZON_MS;
    let (mut at, mut walked, mut slots) = (source.cursor, 0_u64, Vec::new());
    while let Some(next) =
        cron::next_after(&source.trigger, &routine.time_zone, at)?.filter(|next| *next <= until)
    {
        walked += 1;
        if walked > MAX_WALKED_SLOTS {
            return Ok(None);
        }
        if next > now_ms {
            slots.push(next);
        }
        at = next;
    }
    Ok(Some(Plan {
        from: now_ms,
        until,
        slots,
    }))
}

/// The limited rule on `first..=latest`: only `latest` can run, and only inside
/// the window. Every earlier slot is Missed, individually up to the bound and as
/// a compact range beyond it.
fn settle_limited(
    conn: &Connection,
    routine: &routine::Routine,
    source: &Source,
    first: i64,
    latest: i64,
    earliest: i64,
) -> Result<()> {
    let mut slot = first;
    for _ in 0..MAX_MISSED_PER_TICK {
        if slot == latest {
            break;
        }
        insert_slot(conn, routine, source, slot, "missed", false)?;
        slot = cron::next_after(&source.trigger, &routine.time_zone, slot)?
            .ok_or_else(|| Error::InvalidInput("schedule exhausted before latest slot".into()))?;
    }
    if slot < latest {
        conn.execute(
            "INSERT INTO scheduler_missed_ranges
             (routine_id, member_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT DO NOTHING",
            params![
                routine.routine_id,
                source.member_id,
                source.version,
                slot,
                latest,
                serde_json::to_string(&source.trigger)?,
                routine.time_zone
            ],
        )?;
    }
    insert_slot(
        conn,
        routine,
        source,
        latest,
        if latest >= earliest {
            "pending"
        } else {
            "missed"
        },
        false,
    )
}

fn insert_slot(
    conn: &Connection,
    routine: &routine::Routine,
    source: &Source,
    slot: i64,
    status: &str,
    catch_up: bool,
) -> Result<()> {
    claim_slot(
        conn,
        &routine.routine_id,
        &source.member_id,
        source.version,
        slot,
        "",
        status,
        catch_up,
    )?;
    Ok(())
}

/// Claim the identity of a slot: routine, trigger, version, instant and key. `true` when
/// this call made it, `false` when it was already claimed and nothing changed.
#[allow(clippy::too_many_arguments)]
fn claim_slot(
    conn: &Connection,
    routine_id: &str,
    member_id: &str,
    version: u64,
    slot: i64,
    key: &str,
    status: &str,
    catch_up: bool,
) -> Result<bool> {
    Ok(conn.execute(
        "INSERT INTO scheduler_occurrences
         (occurrence_id, routine_id, member_id, schedule_version, scheduled_at_utc, slot_key, status, catch_up)
         VALUES ('occ_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(routine_id, member_id, schedule_version, scheduled_at_utc, slot_key) WHERE manual_nonce IS NULL DO NOTHING",
        params![routine_id, member_id, version, slot, key, status, catch_up],
    )? == 1)
}

/// A slot told by the outside rather than counted by the clock: an agenda event that
/// begins. `key` names what begins, so that two events at one instant are two slots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Told {
    pub key: String,
    pub at_ms: i64,
}

/// A told slot as the scheduler holds it, and whether this call made it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub occurrence: Occurrence,
    /// The key it was told with.
    pub slot_key: String,
    pub fresh: bool,
}

/// Whether the planner was not asked to run at `at_ms`: the app was quit with the
/// background agent off, an absence that is open or that a later tick closed.
fn in_absence(conn: &Connection, at_ms: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM scheduler_absences WHERE from_ms <= ?1 AND ?1 < until_ms)
             OR EXISTS(SELECT 1 FROM scheduler_presence WHERE id=1 AND absent_since <= ?1)",
        [at_ms],
        |row| row.get(0),
    )?)
}

/// Slots told to one simple trigger (`member_id`, empty for a lone trigger) of a
/// routine, as the catch-up policy sees them (docs/specs/routines.md, « Rattrapage »).
/// A slot is claimed once by its routine, trigger version, instant and key, so a
/// second delivery, a second tick or a future executor never starts it twice.
/// - A slot the planner was meant to run for (no deliberate absence at that instant)
///   is owed: it queues as a pending, `catch_up` occurrence and starts in order.
/// - Slots inside a deliberate absence follow the limited rule: only the newest can
///   run, and only inside the late window; the others are Missed.
///
/// The clock and the absences are what the host recorded, never read here.
pub fn deliver_slots(
    conn: &Connection,
    routine_id: &str,
    member_id: &str,
    told: &[Told],
    now_ms: i64,
    policy: &CatchUpPolicy,
) -> Result<Vec<Delivered>> {
    let routine = routine::require(conn, routine_id)?;
    // A start from before the trigger was active is not owed: a pause, or a trigger added
    // or edited since, is a cut, as for a slot the clock counts. The cursor of a lone
    // trigger is the routine's, that of a member is its own.
    let (since, cursor): (i64, i64) = conn.query_row(
        "SELECT active_since, materialized_after FROM scheduler_routines WHERE routine_id=?1",
        [routine_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (version, activated): (u64, i64) = if member_id.is_empty() {
        (routine.schedule_version, cursor)
    } else {
        conn.query_row(
            "SELECT version, materialized_after FROM scheduler_members WHERE routine_id=?1 AND member_id=?2 AND live=1",
            params![routine_id, member_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| Error::InvalidInput("unknown trigger".into()))?
    };
    let activated = activated.max(since);
    let earliest = now_ms.saturating_sub(i64::from(policy.late_minutes.max(1)) * 60_000);
    let mut sorted: Vec<&Told> = told.iter().filter(|slot| slot.at_ms > activated).collect();
    sorted.sort_by(|a, b| (a.at_ms, &a.key).cmp(&(b.at_ms, &b.key)));
    let mut absent = Vec::new();
    let mut delivered = Vec::new();
    for slot in sorted {
        if in_absence(conn, slot.at_ms)? {
            absent.push(slot);
            continue;
        }
        let fresh = claim_slot(
            conn, routine_id, member_id, version, slot.at_ms, &slot.key, "pending", true,
        )?;
        delivered.push((slot, fresh));
    }
    let newest = absent.len().saturating_sub(1);
    for (place, slot) in absent.iter().enumerate() {
        let runnable = place == newest && slot.at_ms >= earliest;
        let fresh = claim_slot(
            conn,
            routine_id,
            member_id,
            version,
            slot.at_ms,
            &slot.key,
            if runnable { "pending" } else { "missed" },
            false,
        )?;
        delivered.push((slot, fresh));
    }
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_occurrences WHERE routine_id=?1 AND member_id=?2
         AND schedule_version=?3 AND scheduled_at_utc=?4 AND slot_key=?5 AND manual_nonce IS NULL"
    ))?;
    delivered
        .into_iter()
        .map(|(slot, fresh)| {
            Ok(Delivered {
                occurrence: query.query_row(
                    params![routine_id, member_id, version, slot.at_ms, slot.key],
                    from_row,
                )?,
                slot_key: slot.key.clone(),
                fresh,
            })
        })
        .collect()
}

// Intervals jump arithmetically, preserving their original phase/anchor. Cron
// uses monotonic binary search, bounded by 64 next_after calls regardless of
// closure length or late-window size; no minute-by-minute scan of the backlog.
fn latest_due(schedule: &cron::Schedule, zone: &str, first: i64, now: i64) -> Result<i64> {
    match schedule {
        cron::Schedule::Once { .. }
        | cron::Schedule::Webhook
        | cron::Schedule::Mail
        | cron::Schedule::Github { .. }
        | cron::Schedule::Calendar { .. }
        | cron::Schedule::Group { .. } => Ok(first),
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
        "SELECT routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone, member_id
         FROM scheduler_missed_ranges WHERE routine_id=?1 ORDER BY from_inclusive, member_id",
    )?;
    let rows = query.query_map([routine_id], |row| {
        Ok(MissedRange {
            routine_id: row.get(0)?,
            schedule_version: row.get(1)?,
            from_inclusive: row.get(2)?,
            until_exclusive: row.get(3)?,
            trigger: routine::json(row, 4)?,
            time_zone: row.get(5)?,
            member_id: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The nonce of an occurrence that an event outside the clock started: a webhook
/// call, a new message or a GitHub event. It is automatic intent, so pause and
/// edits cut it off, unlike a manual launch.
pub fn is_event_nonce(nonce: &str) -> bool {
    nonce.starts_with("webhook:")
        || nonce.starts_with("mail:")
        || nonce.starts_with("github:")
        || nonce.starts_with("calendar:")
}

/// [`is_event_nonce`] as SQL, for the nonce column `column`.
pub fn event_nonce_sql(column: &str) -> String {
    format!(
        "({column} LIKE 'webhook:%' OR {column} LIKE 'mail:%' OR {column} LIKE 'github:%' OR {column} LIKE 'calendar:%')"
    )
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
    run_member(conn, routine_id, "", nonce, now_ms)
}

/// [`run_now`] for a call that belongs to one simple trigger of a group (a
/// webhook), so that history names it and its own edits cut it off.
pub fn run_member(
    conn: &Connection,
    routine_id: &str,
    member_id: &str,
    nonce: &str,
    now_ms: i64,
) -> Result<Occurrence> {
    if nonce.trim().is_empty() || nonce.starts_with("scheduler-error:") {
        return Err(Error::InvalidInput("manual run requires a nonce".into()));
    }
    let routine = routine::require(conn, routine_id)?;
    let version = if member_id.is_empty() {
        routine.schedule_version
    } else {
        conn.query_row(
            "SELECT version FROM scheduler_members WHERE routine_id=?1 AND member_id=?2 AND live=1",
            params![routine_id, member_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| Error::InvalidInput("unknown trigger".into()))?
    };
    conn.execute(
        "INSERT INTO scheduler_occurrences
         (occurrence_id, routine_id, member_id, schedule_version, scheduled_at_utc, manual_nonce, status)
         VALUES ('occ_' || lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5, 'pending')
         ON CONFLICT(routine_id, manual_nonce) WHERE manual_nonce IS NOT NULL DO NOTHING",
        params![routine_id, member_id, version, now_ms, nonce],
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
            |r| Ok((from_row(r)?, r.get::<_, i64>(9)?)),
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
    let mut stmt = conn.prepare("SELECT routine_id,schedule_version,from_inclusive,until_exclusive,trigger,time_zone,rowid,member_id
        FROM scheduler_missed_ranges WHERE routine_id=?1 AND rowid<=?2 AND (?3=0 OR
        (from_inclusive,schedule_version,member_id)<(SELECT from_inclusive,schedule_version,member_id FROM scheduler_missed_ranges WHERE rowid=?3))
        ORDER BY from_inclusive DESC,schedule_version DESC,member_id DESC LIMIT ?4")?;
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
                        member_id: r.get(7)?,
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
    let event = event_nonce_sql("scheduler_occurrences.manual_nonce");
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_occurrences WHERE status='pending'
         AND routine_id IN (SELECT routine_id FROM scheduler_routines WHERE deleted=0
                            AND (enabled=1 OR (scheduler_occurrences.manual_nonce IS NOT NULL
                                 AND NOT {event})))
         ORDER BY scheduled_at_utc, occurrence_id"
    ))?;
    let rows = query.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What [`pending`] returns, minus the slots queued behind an older scheduled
/// slot of the same routine. A routine runs one occurrence at a time, oldest
/// first, so a host dispatches only these and the queue costs one row per
/// routine per tick, however long it is. Manual launches are never queued.
pub fn pending_heads(conn: &Connection) -> Result<Vec<Occurrence>> {
    let event = event_nonce_sql("o.manual_nonce");
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_occurrences o WHERE status='pending'
         AND routine_id IN (SELECT routine_id FROM scheduler_routines WHERE deleted=0
                            AND (enabled=1 OR (o.manual_nonce IS NOT NULL
                                 AND NOT {event})))
         AND (manual_nonce IS NOT NULL OR NOT EXISTS (
            SELECT 1 FROM scheduler_occurrences older WHERE older.routine_id=o.routine_id
            AND older.status='pending' AND older.manual_nonce IS NULL
            AND (older.scheduled_at_utc, older.occurrence_id) < (o.scheduled_at_utc, o.occurrence_id)))
         ORDER BY scheduled_at_utc, occurrence_id"
    ))?;
    let rows = query.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Scheduled occurrences of an active routine that have not started yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    pub waiting: u64,
    /// Planned instant of the next one to start.
    pub next_at: Option<i64>,
}

/// The queue the panel shows. Slots of a paused, deleted or edited routine are
/// not waiting any more, even before the host has retired them.
pub fn queue(conn: &Connection, routine_id: &str) -> Result<Queue> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*), MIN(o.scheduled_at_utc) FROM scheduler_occurrences o
         JOIN scheduler_routines r ON r.routine_id=o.routine_id
         WHERE o.routine_id=?1 AND o.status='pending' AND o.manual_nonce IS NULL
         AND r.enabled=1 AND r.deleted=0 AND o.schedule_version={}",
            current_version_sql("o", "r")
        ),
        [routine_id],
        |row| {
            Ok(Queue {
                waiting: row.get(0)?,
                next_at: row.get(1)?,
            })
        },
    )?)
}

/// A pause, a schedule edit or a deletion stops what has not started: the queued
/// catch-up slots become Missed, and the history says so. Ordinary pending slots
/// keep their existing treatment (the host fails them as "paused or edited").
pub fn retire_catch_up(conn: &Connection, routine_id: &str) -> Result<usize> {
    Ok(conn.execute(
        &format!(
            "UPDATE scheduler_occurrences SET status='missed'
         WHERE routine_id=?1 AND status='pending' AND catch_up=1 AND manual_nonce IS NULL
         AND NOT EXISTS (SELECT 1 FROM scheduler_routines r WHERE r.routine_id=?1 AND r.deleted=0
            AND r.enabled=1 AND {}=scheduler_occurrences.schedule_version)",
            current_version_sql("scheduler_occurrences", "r")
        ),
        [routine_id],
    )?)
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
        member_id: row.get(8)?,
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

    fn host_tick(conn: &Connection, now: i64) -> Vec<Occurrence> {
        with_tx(conn, |conn| {
            materialize(conn, now, &CatchUpPolicy::default())?;
            record_tick(conn, now)?;
            pending_heads(conn)
        })
        .unwrap()
    }

    fn statuses(conn: &Connection, routine: &Routine) -> Vec<(i64, OccurrenceStatus)> {
        list(conn, &routine.routine_id)
            .unwrap()
            .into_iter()
            .map(|o| (o.scheduled_at_utc, o.status))
            .collect()
    }

    #[test]
    fn a_gap_queues_every_slot_in_order_and_a_second_wake_adds_none() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        let heads = host_tick(&conn, 10 * 60_000 + 5_000); // the host was silent for 9 minutes
        let all = list(&conn, &routine.routine_id).unwrap();
        let slots: Vec<i64> = all.iter().map(|o| o.scheduled_at_utc).collect();
        assert_eq!(slots, (1..=10).map(|n| n * 60_000).collect::<Vec<_>>());
        assert!(all
            .iter()
            .skip(1)
            .all(|o| o.status == OccurrenceStatus::Pending));
        // One routine runs one occurrence at a time, oldest first.
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].scheduled_at_utc, 60_000);
        let again = host_tick(&conn, 10 * 60_000 + 6_000);
        assert_eq!(again, heads);
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 10);
        attach_run(&conn, &heads[0].occurrence_id, "run-1").unwrap();
        assert_eq!(
            host_tick(&conn, 10 * 60_000 + 7_000)[0].scheduled_at_utc,
            120_000
        );
    }

    #[test]
    fn queued_slots_neither_expire_nor_are_superseded() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        host_tick(&conn, 4 * 60_000); // 3 minutes of silence: 120 s, 180 s and 240 s are owed
                                      // Hours later the window would have expired an ordinary slot.
        let waiting = tick(&conn, 4 * 60_000 + 5 * 3_600_000);
        for owed in [120_000, 180_000, 240_000] {
            assert!(waiting.iter().any(|o| o.scheduled_at_utc == owed), "{owed}");
        }
        let all = statuses(&conn, &routine);
        assert!(all[1..4]
            .iter()
            .all(|(_, status)| *status == OccurrenceStatus::Pending));
    }

    #[test]
    fn no_gap_keeps_the_ordinary_window_and_newest_only() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        host_tick(&conn, 90_000); // 30 s later: not a gap
        let heads = host_tick(&conn, 190_000); // 100 s of silence is not a gap either
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].scheduled_at_utc, 180_000);
        assert!(statuses(&conn, &routine)
            .iter()
            .filter(|(at, _)| *at < 180_000)
            .all(|(_, status)| *status == OccurrenceStatus::Missed));
    }

    #[test]
    fn a_deliberate_absence_runs_only_the_newest_slot_inside_the_window() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        begin_absence(&conn, 61_000).unwrap();
        begin_absence(&conn, 3_600_000).unwrap(); // a later call cannot shorten it
        let now = 3 * 3_600_000 + 5_000;
        let heads = host_tick(&conn, now);
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].scheduled_at_utc, 3 * 3_600_000);
        let all = statuses(&conn, &routine);
        assert!(all[..all.len() - 1]
            .iter()
            .all(|(_, status)| *status == OccurrenceStatus::Missed));
        let absent: Option<i64> = conn
            .query_row("SELECT absent_since FROM scheduler_presence", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(absent, None, "the first tick ends the absence");
        // The gap after it is a real gap again.
        let later = host_tick(&conn, now + 10 * 60_000);
        assert_eq!(later.len(), 1, "{later:?}");
    }

    #[test]
    fn an_absence_does_not_swallow_slots_that_were_due_before_it_began() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        // The Mac slept from 1 min to 6 min; the app was then quit with the agent off.
        begin_absence(&conn, 6 * 60_000).unwrap();
        host_tick(&conn, 10 * 60_000);
        let all = statuses(&conn, &routine);
        let at = |t: i64| all.iter().find(|(slot, _)| *slot == t).unwrap().1.clone();
        assert_eq!(at(2 * 60_000), OccurrenceStatus::Pending);
        assert_eq!(at(6 * 60_000), OccurrenceStatus::Pending);
        assert_eq!(at(7 * 60_000), OccurrenceStatus::Missed);
        assert_eq!(at(10 * 60_000), OccurrenceStatus::Pending);
    }

    #[test]
    fn the_queue_counts_waiting_slots_and_a_pause_retires_them_as_missed() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        let heads = host_tick(&conn, 6 * 60_000); // the 60 s slot, then five owed slots
        attach_run(&conn, &heads[0].occurrence_id, "run-1").unwrap();
        assert_eq!(
            queue(&conn, &routine.routine_id).unwrap(),
            Queue {
                waiting: 5,
                next_at: Some(120_000)
            }
        );
        let heads = pending_heads(&conn).unwrap();
        attach_run(&conn, &heads[0].occurrence_id, "run-2").unwrap();
        assert_eq!(queue(&conn, &routine.routine_id).unwrap().waiting, 4);
        routine::set_enabled(&conn, &routine.routine_id, false, 6 * 60_000).unwrap();
        assert_eq!(queue(&conn, &routine.routine_id).unwrap().waiting, 0);
        assert_eq!(retire_catch_up(&conn, &routine.routine_id).unwrap(), 4);
        assert_eq!(retire_catch_up(&conn, &routine.routine_id).unwrap(), 0);
        let all = statuses(&conn, &routine);
        assert!(matches!(all[0].1, OccurrenceStatus::Running { .. }));
        assert!(matches!(all[1].1, OccurrenceStatus::Running { .. }));
        assert!(all[2..]
            .iter()
            .all(|(_, status)| *status == OccurrenceStatus::Missed));
        // Resuming does not bring the paused period back.
        routine::set_enabled(&conn, &routine.routine_id, true, 20 * 60_000).unwrap();
        assert!(host_tick(&conn, 20 * 60_000 + 1_000).is_empty());
    }

    #[test]
    fn a_one_shot_due_in_the_gap_is_queued_once() {
        let (conn, mut routine) = setup();
        routine.trigger = cron::Schedule::Once { at: 2 * 3_600_000 };
        routine::update(&conn, &routine, 0).unwrap();
        host_tick(&conn, 1_000);
        let heads = host_tick(&conn, 4 * 3_600_000);
        assert_eq!(heads.len(), 1);
        assert_eq!(heads[0].scheduled_at_utc, 2 * 3_600_000);
        attach_run(&conn, &heads[0].occurrence_id, "run-1").unwrap();
        assert!(host_tick(&conn, 4 * 3_600_000 + 1_000).is_empty());
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 1);
    }

    #[test]
    fn a_busy_pulse_never_hides_a_gap_from_the_next_materialization() {
        let (conn, routine) = setup();
        host_tick(&conn, 60_000);
        heartbeat(&conn, 90_000).unwrap(); // a normal pulse advances
        let last = |conn: &Connection| -> i64 {
            conn.query_row("SELECT last_tick_at FROM scheduler_presence", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(last(&conn), 90_000);
        heartbeat(&conn, 10 * 60_000).unwrap(); // the Mac slept through the pass
        assert_eq!(last(&conn), 90_000);
        heartbeat(&conn, 10 * 60_000 + 1_000).unwrap();
        assert_eq!(last(&conn), 90_000);
        host_tick(&conn, 10 * 60_000 + 2_000);
        let owed = statuses(&conn, &routine);
        assert!(owed
            .iter()
            .filter(|(at, _)| (120_000..=600_000).contains(at))
            .all(|(_, status)| *status == OccurrenceStatus::Pending));
        assert_eq!(last(&conn), 10 * 60_000 + 2_000);
    }

    #[test]
    fn the_heartbeat_is_throttled_and_a_database_without_catch_up_upgrades() {
        let (conn, _) = setup();
        let last = |conn: &Connection| -> i64 {
            conn.query_row("SELECT last_tick_at FROM scheduler_presence", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        record_tick(&conn, 10_000).unwrap();
        record_tick(&conn, 12_000).unwrap();
        assert_eq!(last(&conn), 10_000);
        record_tick(&conn, 15_000).unwrap();
        assert_eq!(last(&conn), 15_000);
        conn.execute_batch("ALTER TABLE scheduler_occurrences DROP COLUMN catch_up")
            .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(pending(&conn).unwrap().len(), 0);
    }

    fn group_of(members: Vec<(&str, cron::Schedule)>) -> cron::Schedule {
        cron::Schedule::Group {
            members: members
                .into_iter()
                .map(|(id, trigger)| cron::Member {
                    id: id.into(),
                    trigger,
                })
                .collect(),
        }
    }
    fn every(minutes: u64) -> cron::Schedule {
        cron::Schedule::Every {
            interval_ms: minutes * 60_000,
            phase_ms: None,
        }
    }
    fn grouped(members: Vec<(&str, cron::Schedule)>) -> (Connection, Routine) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        let mut input = fixture();
        input.trigger = group_of(members);
        let routine = routine::create(&conn, &input).unwrap();
        (conn, routine)
    }

    #[test]
    fn a_group_needs_two_flat_simple_triggers_with_distinct_ids() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        let refused = |trigger: cron::Schedule| {
            let mut input = fixture();
            input.trigger = trigger;
            routine::create(&conn, &input).is_err()
        };
        assert!(refused(group_of(vec![("a", every(1))])));
        assert!(refused(group_of(vec![])));
        assert!(refused(group_of(vec![("a", every(1)), ("a", every(2))])));
        assert!(refused(group_of(vec![("a", every(1)), ("", every(2))])));
        let nested = group_of(vec![("c", every(1)), ("d", cron::Schedule::Webhook)]);
        assert!(refused(group_of(vec![("a", every(1)), ("b", nested)])));
        assert!(!refused(group_of(vec![
            ("a", every(1)),
            ("b", cron::Schedule::Webhook)
        ])));
        assert_eq!(routine::list(&conn, "bot-1").unwrap().len(), 1);
    }

    #[test]
    fn two_triggers_due_at_the_same_instant_give_two_attributed_occurrences() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(1))]);
        let due = tick(&conn, 60_000);
        assert_eq!(due.len(), 2);
        let mut members: Vec<_> = due.iter().map(|o| o.member_id.as_str()).collect();
        members.sort();
        assert_eq!(members, ["a", "b"]);
        // A second tick, or a second host, finds both identities claimed.
        assert_eq!(tick(&conn, 60_000).len(), 2);
        assert_eq!(list(&conn, &routine.routine_id).unwrap().len(), 2);
    }

    #[test]
    fn editing_one_trigger_leaves_the_pending_slots_of_the_others() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(2))]);
        let due = tick(&conn, 120_000);
        assert_eq!(due.len(), 2);
        let mut edited = routine.clone();
        edited.trigger = group_of(vec![("a", every(3)), ("b", every(2))]);
        let edited = routine::update(&conn, &edited, 120_000).unwrap();
        assert_eq!(edited.schedule_version, 2);
        let version = |id: &str| -> u64 {
            conn.query_row(
                "SELECT version FROM scheduler_members WHERE routine_id=?1 AND member_id=?2",
                params![routine.routine_id, id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!((version("a"), version("b")), (2, 1));
        let stale = |member: &str| {
            let o = due.iter().find(|o| o.member_id == member).unwrap();
            !is_current(&conn, o).unwrap()
        };
        assert!(stale("a"));
        assert!(!stale("b"));
        assert_eq!(queue(&conn, &routine.routine_id).unwrap().waiting, 1);
        let pending: Vec<_> = tick(&conn, 120_000)
            .into_iter()
            .map(|o| o.member_id)
            .collect();
        assert_eq!(pending, ["b"]);
    }

    #[test]
    fn a_member_that_leaves_the_group_never_starts_again() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(1))]);
        let due = tick(&conn, 60_000);
        let mut edited = routine.clone();
        edited.trigger = group_of(vec![("a", every(1)), ("c", every(1))]);
        routine::update(&conn, &edited, 60_000).unwrap();
        let b = due.iter().find(|o| o.member_id == "b").unwrap();
        assert!(!is_current(&conn, b).unwrap());
        let next: Vec<_> = tick(&conn, 120_000)
            .into_iter()
            .filter(|o| o.status == OccurrenceStatus::Pending)
            .map(|o| o.member_id)
            .collect();
        assert!(next.iter().all(|member| member == "a" || member == "c"));
        assert!(next.contains(&"c".to_string()));
    }

    #[test]
    fn a_group_catches_up_each_trigger_from_its_own_cursor() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(2))]);
        // The Mac slept for ten minutes with the planner meant to run.
        record_tick(&conn, 0).unwrap();
        let queued = tick(&conn, 600_000);
        let slots = |member: &str| -> Vec<i64> {
            queued
                .iter()
                .filter(|o| o.member_id == member && o.status == OccurrenceStatus::Pending)
                .map(|o| o.scheduled_at_utc)
                .collect()
        };
        assert_eq!(slots("a").len(), 10);
        assert_eq!(slots("b").len(), 5);
        // Pausing stops what has not started, for every trigger.
        routine::set_enabled(&conn, &routine.routine_id, false, 600_000).unwrap();
        assert_eq!(retire_catch_up(&conn, &routine.routine_id).unwrap(), 15);
    }

    #[test]
    fn a_database_from_before_groups_keeps_its_slots_and_missed_ranges() {
        let (conn, routine) = setup();
        tick(&conn, 7_230_000);
        let before = list(&conn, &routine.routine_id).unwrap();
        let ranges = missed_ranges(&conn, &routine.routine_id).unwrap();
        assert_eq!(ranges.len(), 1);
        conn.execute_batch(
            "DROP INDEX scheduler_scheduled_member_slot;
             ALTER TABLE scheduler_occurrences DROP COLUMN member_id;
             CREATE UNIQUE INDEX scheduler_scheduled_slot
                ON scheduler_occurrences(routine_id, schedule_version, scheduled_at_utc)
                WHERE manual_nonce IS NULL;",
        )
        .unwrap();
        // The pre-group primary key cannot be changed in place: recreate the old table.
        conn.execute_batch(
            "DROP INDEX scheduler_range_history;
             ALTER TABLE scheduler_missed_ranges RENAME TO keep;
             CREATE TABLE scheduler_missed_ranges (
                routine_id TEXT NOT NULL, schedule_version INTEGER NOT NULL,
                from_inclusive INTEGER NOT NULL, until_exclusive INTEGER NOT NULL,
                trigger TEXT NOT NULL, time_zone TEXT NOT NULL,
                PRIMARY KEY(routine_id, schedule_version, from_inclusive));
             INSERT INTO scheduler_missed_ranges
                SELECT routine_id, schedule_version, from_inclusive, until_exclusive, trigger, time_zone FROM keep;
             DROP TABLE keep;",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(list(&conn, &routine.routine_id).unwrap(), before);
        assert_eq!(missed_ranges(&conn, &routine.routine_id).unwrap(), ranges);
        // The new identity still claims a slot once.
        assert_eq!(tick(&conn, 7_230_000).len(), 1);
        assert_eq!(
            list(&conn, &routine.routine_id).unwrap().len(),
            before.len()
        );
    }

    #[test]
    fn a_database_from_before_told_slots_keeps_its_slots_and_claims_them_once() {
        let (conn, routine) = setup();
        tick(&conn, 7_230_000);
        let before = list(&conn, &routine.routine_id).unwrap();
        conn.execute_batch(
            "DROP INDEX scheduler_scheduled_member_slot;
             ALTER TABLE scheduler_occurrences DROP COLUMN slot_key;
             DROP TABLE scheduler_absences;
             CREATE UNIQUE INDEX scheduler_scheduled_member_slot
                ON scheduler_occurrences(routine_id, member_id, schedule_version, scheduled_at_utc)
                WHERE manual_nonce IS NULL;",
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(list(&conn, &routine.routine_id).unwrap(), before);
        // The keyed identity still claims a clock slot once.
        tick(&conn, 7_230_000);
        assert_eq!(
            list(&conn, &routine.routine_id).unwrap().len(),
            before.len()
        );
        // And an absence is remembered from now on.
        begin_absence(&conn, 1).unwrap();
        with_tx(&conn, |conn| record_tick(conn, 9_000_000)).unwrap();
        assert!(in_absence(&conn, 5).unwrap() && !in_absence(&conn, 9_000_001).unwrap());
    }

    #[test]
    fn a_calendar_trigger_needs_at_least_one_case_each_once() {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        let refused = |cases: &[cron::CalendarCase]| {
            let mut input = fixture();
            input.trigger = calendar(cases);
            routine::create(&conn, &input).is_err()
        };
        assert!(refused(&[]));
        assert!(refused(&[
            cron::CalendarCase::Begins,
            cron::CalendarCase::Begins
        ]));
        assert!(!refused(&[
            cron::CalendarCase::Created,
            cron::CalendarCase::Begins
        ]));
    }

    #[test]
    fn a_fired_one_shot_of_a_group_does_not_block_editing_the_rest() {
        let once = cron::Schedule::Once { at: 100_000 };
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", once.clone())]);
        // The one-shot is past by the time the routine is renamed.
        let mut edited = routine.clone();
        edited.name = "Renamed".into();
        let edited = routine::update(&conn, &edited, 500_000).unwrap();
        assert_eq!(edited.name, "Renamed");
        // Changing another member is fine too, and leaves the spent one alone.
        let mut slower = edited.clone();
        slower.trigger = group_of(vec![("a", every(2)), ("b", once)]);
        routine::update(&conn, &slower, 500_001).unwrap();
        // A new or rescheduled one-shot in the past is still refused.
        let mut past = slower.clone();
        past.trigger = group_of(vec![
            ("a", every(2)),
            ("b", cron::Schedule::Once { at: 200_000 }),
        ]);
        past.schedule_version = 3;
        assert!(routine::update(&conn, &past, 500_002).is_err());
    }

    #[test]
    fn a_member_remembers_its_trigger_at_each_version() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(5))]);
        let mut edited = routine.clone();
        edited.trigger = group_of(vec![("a", every(2)), ("b", every(5))]);
        routine::update(&conn, &edited, 1_000).unwrap();
        let at = |member: &str, version: u64| {
            routine::member_trigger(&conn, &routine.routine_id, member, version).unwrap()
        };
        assert_eq!(at("a", 1), Some(every(1)));
        assert_eq!(at("a", 2), Some(every(2)));
        assert_eq!(at("b", 1), Some(every(5)));
        assert_eq!(at("b", 2), None);
    }

    #[test]
    fn a_timezone_edit_only_restarts_the_triggers_that_follow_the_zone() {
        let daily = cron::parse("0 9 * * *").unwrap();
        let pinned = cron::parse("CRON_TZ=UTC 0 9 * * *").unwrap();
        let (conn, routine) = grouped(vec![
            ("every", every(1)),
            ("cron", daily),
            ("pinned", pinned),
            ("hook", cron::Schedule::Webhook),
        ]);
        let mut edited = routine.clone();
        edited.time_zone = "America/Montreal".into();
        routine::update(&conn, &edited, 1_000).unwrap();
        let version = |id: &str| -> u64 {
            conn.query_row(
                "SELECT version FROM scheduler_members WHERE routine_id=?1 AND member_id=?2",
                params![routine.routine_id, id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            [
                version("every"),
                version("cron"),
                version("pinned"),
                version("hook")
            ],
            [1, 2, 1, 1]
        );
    }

    #[test]
    fn a_mail_member_has_no_clock_slot_and_leaves_the_others_alone() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("mail", cron::Schedule::Mail)]);
        tick(&conn, 7_230_000);
        assert!(list(&conn, &routine.routine_id)
            .unwrap()
            .iter()
            .all(|o| o.member_id == "a"));
        assert!(missed_ranges(&conn, &routine.routine_id)
            .unwrap()
            .iter()
            .all(|r| r.member_id == "a"));
        // A message-only routine is never due, whatever the gap.
        let (conn, routine) = grouped(vec![
            ("m1", cron::Schedule::Mail),
            ("m2", cron::Schedule::Mail),
        ]);
        tick(&conn, 7_230_000);
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
        assert!(missed_ranges(&conn, &routine.routine_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn two_members_with_the_same_schedule_keep_their_own_missed_ranges() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("b", every(1))]);
        tick(&conn, 7_230_000);
        let ranges = missed_ranges(&conn, &routine.routine_id).unwrap();
        let mut members: Vec<_> = ranges.iter().map(|r| r.member_id.as_str()).collect();
        members.sort();
        assert_eq!(members, ["a", "b"]);
        assert_eq!(ranges[0].from_inclusive, ranges[1].from_inclusive);
        assert_eq!(ranges[0].schedule_version, ranges[1].schedule_version);
        let page = missed_ranges_page(&conn, &routine.routine_id, None).unwrap();
        assert_eq!(page.items.len(), 2);
    }

    fn calendar(cases: &[cron::CalendarCase]) -> cron::Schedule {
        cron::Schedule::Calendar {
            cases: cases.to_vec(),
        }
    }
    fn told(key: &str, at_ms: i64) -> Told {
        Told {
            key: key.into(),
            at_ms,
        }
    }
    fn slots(conn: &Connection, routine: &Routine) -> Vec<(i64, &'static str)> {
        list(conn, &routine.routine_id)
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o.scheduled_at_utc,
                    match o.status {
                        OccurrenceStatus::Pending => "pending",
                        OccurrenceStatus::Missed => "missed",
                        _ => "other",
                    },
                )
            })
            .collect()
    }
    fn begins() -> cron::Schedule {
        calendar(&[cron::CalendarCase::Begins])
    }

    #[test]
    fn a_calendar_trigger_has_no_clock_slot_and_leaves_the_others_alone() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("c", begins())]);
        tick(&conn, 7_230_000);
        assert!(list(&conn, &routine.routine_id)
            .unwrap()
            .iter()
            .all(|o| o.member_id == "a"));
        let (conn, routine) = grouped(vec![("c1", begins()), ("c2", begins())]);
        tick(&conn, 7_230_000);
        assert!(list(&conn, &routine.routine_id).unwrap().is_empty());
    }

    #[test]
    fn a_told_slot_is_claimed_once_by_its_key_and_instant() {
        let (conn, routine) = grouped(vec![("a", every(1)), ("c", begins())]);
        let id = &routine.routine_id;
        let policy = CatchUpPolicy::default();
        let first = deliver_slots(
            &conn,
            id,
            "c",
            &[told("x", 5_000), told("y", 5_000)],
            6_000,
            &policy,
        )
        .unwrap();
        assert_eq!(first.len(), 2);
        assert!(first.iter().all(|d| d.fresh));
        // A second delivery of the same facts finds both claimed and starts nothing.
        let again = deliver_slots(
            &conn,
            id,
            "c",
            &[told("x", 5_000), told("y", 5_000)],
            7_000,
            &policy,
        )
        .unwrap();
        assert!(again.iter().all(|d| !d.fresh));
        assert_eq!(
            again
                .iter()
                .map(|d| &d.occurrence.occurrence_id)
                .collect::<Vec<_>>(),
            first
                .iter()
                .map(|d| &d.occurrence.occurrence_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(slots(&conn, &routine).len(), 2);
        // The same key at another instant is a new slot.
        let moved = deliver_slots(&conn, id, "c", &[told("x", 9_000)], 9_500, &policy).unwrap();
        assert!(moved[0].fresh);
        assert_eq!(slots(&conn, &routine).len(), 3);
        assert!(first[0].occurrence.manual_nonce.is_none());
        assert_eq!(first[0].occurrence.member_id, "c");
    }

    #[test]
    fn a_told_slot_outside_any_absence_queues_as_catch_up_in_order() {
        let (conn, routine) = grouped(vec![("a", every(60)), ("c", begins())]);
        let late = 3 * 3_600_000;
        deliver_slots(
            &conn,
            &routine.routine_id,
            "c",
            &[told("b", 2 * 3_600_000), told("a", 3_600_000)],
            late,
            &CatchUpPolicy::default(),
        )
        .unwrap();
        assert_eq!(
            slots(&conn, &routine),
            vec![(3_600_000, "pending"), (2 * 3_600_000, "pending")]
        );
        // An hour-old slot is owed, not late: the window never expires it, and the
        // routine runs them one at a time, oldest first.
        tick(&conn, late + 1_000);
        let heads = |conn: &Connection| -> Vec<i64> {
            pending_heads(conn)
                .unwrap()
                .into_iter()
                .filter(|o| o.member_id == "c")
                .map(|o| o.scheduled_at_utc)
                .collect()
        };
        assert_eq!(heads(&conn), vec![3_600_000]);
        assert!(queue(&conn, &routine.routine_id).unwrap().waiting >= 2);
    }

    #[test]
    fn a_told_slot_in_a_deliberate_absence_follows_the_limited_rule() {
        const HOUR: i64 = 3_600_000;
        let (conn, routine) = grouped(vec![("a", every(600)), ("c", begins())]);
        let id = &routine.routine_id;
        with_tx(&conn, |conn| record_tick(conn, 0)).unwrap();
        begin_absence(&conn, HOUR / 2).unwrap();
        // The first tick after the reopening closes the absence; its record stays.
        with_tx(&conn, |conn| record_tick(conn, 3 * HOUR)).unwrap();
        let policy = CatchUpPolicy::default();
        deliver_slots(
            &conn,
            id,
            "c",
            &[
                told("old", HOUR),
                told("recent", 5 * HOUR / 2),
                told("older", HOUR / 2 + 1),
            ],
            3 * HOUR + 10_000,
            &policy,
        )
        .unwrap();
        assert_eq!(
            slots(&conn, &routine),
            vec![
                (HOUR / 2 + 1, "missed"),
                (HOUR, "missed"),
                (5 * HOUR / 2, "pending")
            ]
        );
        // A start after the absence ended is owed in full, whatever its age.
        deliver_slots(
            &conn,
            id,
            "c",
            &[told("owed", 3 * HOUR + 1_000)],
            4 * HOUR + 30 * 60_000,
            &policy,
        )
        .unwrap();
        let owed = list(&conn, id).unwrap();
        assert_eq!(owed.last().unwrap().scheduled_at_utc, 3 * HOUR + 1_000);
        assert_eq!(owed.last().unwrap().status, OccurrenceStatus::Pending);
        // The newest of an absence still expires when it is older than the window.
        let (conn, routine) = grouped(vec![("a", every(600)), ("c", begins())]);
        with_tx(&conn, |conn| record_tick(conn, 0)).unwrap();
        begin_absence(&conn, 1).unwrap();
        with_tx(&conn, |conn| record_tick(conn, 5 * HOUR)).unwrap();
        deliver_slots(
            &conn,
            &routine.routine_id,
            "c",
            &[told("stale", HOUR)],
            5 * HOUR + 1,
            &policy,
        )
        .unwrap();
        assert_eq!(slots(&conn, &routine), vec![(HOUR, "missed")]);
    }

    #[test]
    fn an_open_absence_counts_before_the_tick_that_closes_it() {
        const HOUR: i64 = 3_600_000;
        let (conn, routine) = grouped(vec![("a", every(600)), ("c", begins())]);
        with_tx(&conn, |conn| record_tick(conn, 0)).unwrap();
        begin_absence(&conn, 1).unwrap();
        deliver_slots(
            &conn,
            &routine.routine_id,
            "c",
            &[told("one", HOUR), told("two", 2 * HOUR)],
            2 * HOUR + 1_000,
            &CatchUpPolicy::default(),
        )
        .unwrap();
        assert_eq!(
            slots(&conn, &routine),
            vec![(HOUR, "missed"), (2 * HOUR, "pending")]
        );
    }

    #[test]
    fn a_start_from_before_the_trigger_was_active_is_not_owed() {
        let (conn, routine) = grouped(vec![("a", every(60)), ("c", begins())]);
        let id = &routine.routine_id;
        let policy = CatchUpPolicy::default();
        // Paused at 10 000 and resumed at 50 000: what began between is not caught up.
        routine::set_enabled(&conn, id, false, 10_000).unwrap();
        routine::set_enabled(&conn, id, true, 50_000).unwrap();
        let told_after_resume = [told("during", 30_000), told("after", 60_000)];
        let delivered = deliver_slots(&conn, id, "c", &told_after_resume, 70_000, &policy).unwrap();
        assert_eq!(
            delivered
                .iter()
                .map(|d| d.occurrence.scheduled_at_utc)
                .collect::<Vec<_>>(),
            vec![60_000]
        );
        assert_eq!(slots(&conn, &routine), vec![(60_000, "pending")]);
        // A trigger added by an edit starts from the edit, even while the watch kept running.
        let mut edited = routine::get(&conn, id).unwrap().unwrap();
        edited.trigger = group_of(vec![
            ("a", every(60)),
            ("c", begins()),
            (
                "d",
                calendar(&[cron::CalendarCase::Created, cron::CalendarCase::Begins]),
            ),
        ]);
        routine::update(&conn, &edited, 200_000).unwrap();
        let before_edit = [told("old", 150_000), told("new", 250_000)];
        let added = deliver_slots(&conn, id, "d", &before_edit, 300_000, &policy).unwrap();
        assert_eq!(
            added
                .iter()
                .map(|d| d.occurrence.scheduled_at_utc)
                .collect::<Vec<_>>(),
            vec![250_000]
        );
    }

    #[test]
    fn a_told_slot_obeys_the_cutoff_of_its_trigger() {
        let (conn, routine) = grouped(vec![("a", every(60)), ("c", begins())]);
        let id = &routine.routine_id;
        deliver_slots(
            &conn,
            id,
            "c",
            &[told("x", 5_000)],
            6_000,
            &CatchUpPolicy::default(),
        )
        .unwrap();
        // A pause is a cut: what has not started becomes Missed.
        routine::set_enabled(&conn, id, false, 7_000).unwrap();
        assert_eq!(retire_catch_up(&conn, id).unwrap(), 1);
        assert_eq!(slots(&conn, &routine), vec![(5_000, "missed")]);
        assert!(deliver_slots(
            &conn,
            id,
            "nobody",
            &[told("y", 1)],
            8_000,
            &CatchUpPolicy::default()
        )
        .is_err());
    }

    const WEEK: i64 = 7 * 86_400_000;
    const SATURDAY_NOON: i64 = 1_792_843_200_000;
    const MONDAY_NOON: i64 = 1_793_016_000_000;
    const WEDNESDAY_NOON: i64 = 1_793_188_800_000;

    /// A routine alone in a fresh database; the plans are kept per routine id across tests.
    fn planned_routine(
        id: &str,
        trigger: cron::Schedule,
        zone: &str,
        since: i64,
    ) -> (Connection, Routine) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        let routine = routine::create(
            &conn,
            &Routine {
                routine_id: id.into(),
                conversation_id: format!("thread-{id}"),
                trigger,
                time_zone: zone.into(),
                modified_at: since,
                ..fixture()
            },
        )
        .unwrap();
        (conn, routine)
    }

    /// The scheduler's own walk, slot by slot: what `planned` must agree with.
    fn walked(routine: &Routine, after: i64) -> u64 {
        let (mut at, mut count) = (after, 0);
        while let Some(next) = cron::next_after(&routine.trigger, &routine.time_zone, at)
            .unwrap()
            .filter(|next| *next <= after + WEEK)
        {
            count += 1;
            at = next;
        }
        count
    }

    fn cron(expression: &str) -> cron::Schedule {
        cron::parse(expression).unwrap()
    }

    #[test]
    fn planned_counts_weekdays_and_zone_changes_as_the_walk_does() {
        let (conn, weekdays) =
            planned_routine("plan-weekdays", cron("* * * * 1-5"), "UTC", SATURDAY_NOON);
        for now in [SATURDAY_NOON, MONDAY_NOON] {
            assert_eq!(planned(&conn, &weekdays, now, WEEK).unwrap(), Some(7200));
        }
        // The week of the clock change in Toronto (2026-11-01).
        let (conn, dst) = planned_routine(
            "plan-dst",
            cron("* * * * *"),
            "America/Toronto",
            WEDNESDAY_NOON,
        );
        let expected = walked(&dst, WEDNESDAY_NOON);
        assert_ne!(expected, 10_080, "the week must hold the clock change");
        assert_eq!(
            planned(&conn, &dst, WEDNESDAY_NOON, WEEK).unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn planned_counts_each_group_member_and_keeps_an_interval_anchor() {
        let daily = |id: &str| crate::cron::Member {
            id: id.into(),
            trigger: cron("0 9 * * *"),
        };
        let group = cron::Schedule::Group {
            members: vec![daily("first"), daily("second")],
        };
        let (conn, routine) = planned_routine("plan-group", group, "UTC", SATURDAY_NOON);
        assert_eq!(
            planned(&conn, &routine, SATURDAY_NOON, WEEK).unwrap(),
            Some(14)
        );
        // Every ten days, last due nine days ago: the next slot is tomorrow.
        let every = cron::Schedule::Every {
            interval_ms: 10 * 86_400_000,
            phase_ms: None,
        };
        let (conn, routine) =
            planned_routine("plan-anchor", every, "UTC", SATURDAY_NOON - 9 * 86_400_000);
        assert_eq!(
            planned(&conn, &routine, SATURDAY_NOON, WEEK).unwrap(),
            Some(1)
        );
        // A backlog too long to walk, and a paused routine plans nothing.
        let (conn, mut routine) = planned_routine(
            "plan-dense",
            cron::Schedule::Every {
                interval_ms: 60_000,
                phase_ms: None,
            },
            "UTC",
            SATURDAY_NOON - 100 * 86_400_000,
        );
        assert_eq!(planned(&conn, &routine, SATURDAY_NOON, WEEK).unwrap(), None);
        routine.enabled = false;
        assert_eq!(
            planned(&conn, &routine, SATURDAY_NOON, WEEK).unwrap(),
            Some(0)
        );
    }
}
