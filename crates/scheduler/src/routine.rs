//! Durable routines. The caller owns transactions, including migrations and edits.
//! `reports/cron-detail.md` CR01 permits an inline prompt without a skill. CR19
//! creates model routines paused/untrusted, then resumes after conversational
//! confirmation (Q5 steering); this store adds no separate approval gate.
use crate::cron::{self, Schedule};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, types::Type, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoutineBody {
    InlinePrompt { prompt: String },
    SkillInvocation { skill_id: String, version: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    User,
    Model,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub routine_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub name: String,
    pub body: RoutineBody,
    pub trigger: Schedule,
    pub time_zone: String,
    pub enabled: bool,
    pub origin: Origin,
    pub untrusted: bool,
    pub schedule_version: u64,
    pub modified_at: i64,
    pub last_served_at: Option<i64>,
}

const COLUMNS: &str = "routine_id, bot_id, conversation_id, name, body, trigger,
    time_zone, enabled, origin, untrusted, schedule_version, modified_at, last_served_at";

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS component_versions (
            component TEXT PRIMARY KEY, version INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS scheduler_routines (
            routine_id TEXT PRIMARY KEY,
            bot_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            body TEXT NOT NULL,
            trigger TEXT NOT NULL,
            time_zone TEXT NOT NULL,
            enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
            origin TEXT NOT NULL CHECK (origin IN ('user', 'model')),
            untrusted INTEGER NOT NULL CHECK (untrusted IN (0, 1)),
            schedule_version INTEGER NOT NULL CHECK (schedule_version > 0),
            modified_at INTEGER NOT NULL,
            last_served_at INTEGER,
            materialized_after INTEGER NOT NULL,
            active_since INTEGER NOT NULL,
            deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
         );
         CREATE INDEX IF NOT EXISTS scheduler_routines_bot ON scheduler_routines(bot_id);
         CREATE TABLE IF NOT EXISTS scheduler_members (
            routine_id TEXT NOT NULL REFERENCES scheduler_routines(routine_id),
            member_id TEXT NOT NULL,
            version INTEGER NOT NULL CHECK (version > 0),
            trigger TEXT NOT NULL,
            live INTEGER NOT NULL CHECK (live IN (0, 1)),
            materialized_after INTEGER NOT NULL,
            PRIMARY KEY (routine_id, member_id)
         );
         CREATE TABLE IF NOT EXISTS scheduler_member_versions (
            routine_id TEXT NOT NULL,
            member_id TEXT NOT NULL,
            version INTEGER NOT NULL,
            trigger TEXT NOT NULL,
            PRIMARY KEY (routine_id, member_id, version)
         );
         INSERT INTO component_versions(component, version)
            VALUES ('scheduler.routine', 1) ON CONFLICT(component) DO NOTHING;",
    )?;
    check_schema(conn, "scheduler.routine")?;
    // Upgrade databases made by the pre-review implementation of schema 1.
    let has_active_since = conn
        .prepare("PRAGMA table_info(scheduler_routines)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "active_since");
    if !has_active_since {
        conn.execute_batch(
            "ALTER TABLE scheduler_routines ADD COLUMN active_since INTEGER NOT NULL DEFAULT 0;
            UPDATE scheduler_routines SET active_since=modified_at;",
        )?;
    }
    Ok(())
}

pub(crate) fn check_schema(conn: &Connection, component: &str) -> Result<()> {
    let version: i64 = conn.query_row(
        "SELECT version FROM component_versions WHERE component=?1",
        [component],
        |row| row.get(0),
    )?;
    if version != 1 {
        return Err(Error::InvalidInput(format!(
            "unsupported {component} schema version {version}"
        )));
    }
    Ok(())
}

/// Caller supplies stable IDs and creation time; version/progress/provenance
/// flags are normalized here, not trusted from a model's DTO.
pub fn create(conn: &Connection, routine: &Routine) -> Result<Routine> {
    validate(routine, routine.modified_at)?;
    let mut routine = routine.clone();
    routine.schedule_version = 1;
    routine.last_served_at = None;
    routine.untrusted = routine.origin == Origin::Model;
    if routine.untrusted {
        routine.enabled = false;
    }
    conn.execute(
        "INSERT INTO scheduler_routines
         (routine_id, bot_id, conversation_id, name, body, trigger, time_zone,
          enabled, origin, untrusted, schedule_version, modified_at, materialized_after, active_since)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11, ?11)",
        params![
            routine.routine_id,
            routine.bot_id,
            routine.conversation_id,
            routine.name,
            serde_json::to_string(&routine.body)?,
            serde_json::to_string(&routine.trigger)?,
            routine.time_zone,
            routine.enabled,
            if routine.origin == Origin::Model {
                "model"
            } else {
                "user"
            },
            routine.untrusted,
            routine.modified_at,
        ],
    )?;
    sync_members(conn, &routine, false, routine.modified_at)?;
    Ok(routine)
}

/// Only a cron without a zone of its own follows the routine's time zone: an
/// interval, a one-shot or a webhook is unaffected when that zone changes.
fn uses_routine_zone(trigger: &Schedule) -> bool {
    matches!(trigger, Schedule::Cron { expression, time_zone: None }
        if !["CRON_TZ=", "TZ="].iter().any(|prefix| expression.trim_start().starts_with(prefix)))
}

/// Keep one row per simple trigger of a group: its own schedule version and its
/// own cursor, so editing one trigger never moves another's slots. A member
/// that leaves the group keeps its row (not live), so its version only grows.
/// The live members of a group whose trigger is the one already stored.
fn unchanged_members(conn: &Connection, routine: &Routine) -> Result<HashSet<String>> {
    let Schedule::Group { members } = &routine.trigger else {
        return Ok(HashSet::new());
    };
    let mut kept = HashSet::new();
    for member in members {
        let same: bool = conn
            .query_row(
                "SELECT trigger=?3 FROM scheduler_members WHERE routine_id=?1 AND member_id=?2 AND live=1",
                params![routine.routine_id, member.id, serde_json::to_string(&member.trigger)?],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if same {
            kept.insert(member.id.clone());
        }
    }
    Ok(kept)
}

/// The trigger of a group member as it was at `version`, for naming past occurrences.
pub fn member_trigger(
    conn: &Connection,
    routine_id: &str,
    member_id: &str,
    version: u64,
) -> Result<Option<Schedule>> {
    Ok(conn
        .query_row(
            "SELECT trigger FROM scheduler_member_versions
             WHERE routine_id=?1 AND member_id=?2 AND version=?3",
            params![routine_id, member_id, version],
            |row| json(row, 0),
        )
        .optional()?)
}

fn sync_members(
    conn: &Connection,
    routine: &Routine,
    zone_changed: bool,
    now_ms: i64,
) -> Result<()> {
    let members: &[cron::Member] = match &routine.trigger {
        Schedule::Group { members } => members,
        _ => &[],
    };
    for member in members {
        let text = serde_json::to_string(&member.trigger)?;
        let known: Option<(String, bool)> = conn
            .query_row(
                "SELECT trigger, live FROM scheduler_members WHERE routine_id=?1 AND member_id=?2",
                params![routine.routine_id, member.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match known {
            None => {
                conn.execute(
                    "INSERT INTO scheduler_members VALUES (?1, ?2, 1, ?3, 1, ?4)",
                    params![routine.routine_id, member.id, text, now_ms],
                )?;
            }
            Some((old, true))
                if old == text && !(zone_changed && uses_routine_zone(&member.trigger)) => {}
            Some(_) => {
                conn.execute(
                    "UPDATE scheduler_members SET version=version+1, trigger=?3, live=1,
                     materialized_after=MAX(materialized_after, ?4)
                     WHERE routine_id=?1 AND member_id=?2",
                    params![routine.routine_id, member.id, text, now_ms],
                )?;
            }
        }
        // History keeps naming the trigger as it was when an occurrence was made.
        conn.execute(
            "INSERT OR REPLACE INTO scheduler_member_versions
             SELECT routine_id, member_id, version, trigger FROM scheduler_members
             WHERE routine_id=?1 AND member_id=?2",
            params![routine.routine_id, member.id],
        )?;
    }
    let live = conn
        .prepare("SELECT member_id FROM scheduler_members WHERE routine_id=?1 AND live=1")?
        .query_map([&routine.routine_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in live
        .iter()
        .filter(|id| !members.iter().any(|m| &m.id == *id))
    {
        conn.execute(
            "UPDATE scheduler_members SET live=0 WHERE routine_id=?1 AND member_id=?2",
            params![routine.routine_id, id],
        )?;
    }
    Ok(())
}

/// A schedule edit starts strictly after the edit and the last served slot.
/// Metadata edits keep the cursor/interval anchor, and cannot forge progress or
/// change the owning bot, conversation, origin or enabled flag. Activation is
/// exclusively `set_enabled`, protected by conversational confirmation in the
/// wrapper. `modified_at` is also a monotonic edit token (including same-ms
/// writes and clock rollback); use the returned DTO on subsequent edits.
/// Compose this read/write in the caller's transaction.
pub fn update(conn: &Connection, routine: &Routine, now_ms: i64) -> Result<Routine> {
    let old = require(conn, &routine.routine_id)?;
    let now_ms = now_ms.max(old.modified_at);
    validate_keeping(routine, now_ms, &unchanged_members(conn, routine)?)?;
    if routine.bot_id != old.bot_id || routine.conversation_id != old.conversation_id {
        return Err(Error::InvalidInput(
            "routine owner/conversation is immutable".into(),
        ));
    }
    if routine.schedule_version != old.schedule_version || routine.modified_at != old.modified_at {
        return Err(Error::InvalidInput("stale routine edit".into()));
    }
    let modified_at = now_ms.max(
        old.modified_at
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("routine edit token exhausted".into()))?,
    );
    let changed = routine.trigger != old.trigger || routine.time_zone != old.time_zone;
    let version = old.schedule_version + u64::from(changed);
    if version > i64::MAX as u64 {
        return Err(Error::InvalidInput("schedule version exhausted".into()));
    }
    let mut updated = routine.clone();
    updated.enabled = old.enabled;
    updated.origin = old.origin;
    updated.untrusted = old.untrusted;
    updated.last_served_at = old.last_served_at;
    updated.schedule_version = version;
    updated.modified_at = modified_at;
    if conn.execute(
        "UPDATE scheduler_routines SET name=?2, body=?3, trigger=?4, time_zone=?5,
         schedule_version=?6, modified_at=?7,
         materialized_after=CASE WHEN ?8 THEN MAX(materialized_after, ?9, COALESCE(last_served_at, ?9))
                                ELSE materialized_after END
         WHERE routine_id=?1 AND deleted=0 AND schedule_version=?10 AND modified_at=?11",
        params![
            routine.routine_id,
            routine.name,
            serde_json::to_string(&routine.body)?,
            serde_json::to_string(&routine.trigger)?,
            routine.time_zone,
            version,
            modified_at,
            changed,
            now_ms,
            old.schedule_version,
            old.modified_at,
        ],
    )? != 1 {
        return Err(Error::InvalidInput("stale routine edit".into()));
    }
    sync_members(conn, routine, routine.time_zone != old.time_zone, now_ms)?;
    Ok(updated)
}

pub fn get(conn: &Connection, routine_id: &str) -> Result<Option<Routine>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM scheduler_routines WHERE routine_id=?1 AND deleted=0"),
            [routine_id],
            from_row,
        )
        .optional()?)
}

pub fn list(conn: &Connection, bot_id: &str) -> Result<Vec<Routine>> {
    let mut query = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM scheduler_routines WHERE bot_id=?1 AND deleted=0 ORDER BY routine_id"
    ))?;
    let rows = query.query_map([bot_id], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The wrapper checks conversational confirmation before model-origin resume.
/// Resume skips the explicit pause, including unconfirmed model-created slots.
/// Unphased intervals restart at resume time (CR10); catch-up is only for closure,
/// not pause. Idempotent enable does not reset an already active schedule.
pub fn set_enabled(conn: &Connection, routine_id: &str, enabled: bool, now_ms: i64) -> Result<()> {
    let was_enabled: Option<bool> = conn
        .query_row(
            "SELECT enabled FROM scheduler_routines WHERE routine_id=?1 AND deleted=0",
            [routine_id],
            |row| row.get(0),
        )
        .optional()?;
    if conn.execute(
        "UPDATE scheduler_routines SET
         active_since=CASE WHEN ?2=1 AND enabled=0
            THEN MAX(materialized_after, ?3, COALESCE(last_served_at, ?3)) ELSE active_since END,
         materialized_after=CASE WHEN ?2=1 AND enabled=0
            THEN MAX(materialized_after, ?3, COALESCE(last_served_at, ?3)) ELSE materialized_after END,
         modified_at=CASE WHEN enabled!=?2 THEN MAX(modified_at+1, ?3) ELSE modified_at END,
         enabled=?2 WHERE routine_id=?1 AND deleted=0 AND modified_at<9223372036854775807",
        params![routine_id, enabled, now_ms],
    )? == 0
    {
        return Err(Error::InvalidInput("unknown routine".into()));
    }
    // Resuming restarts every trigger of a group after the pause, like a lone schedule.
    if enabled && was_enabled == Some(false) {
        conn.execute(
            "UPDATE scheduler_members SET materialized_after=MAX(materialized_after, ?2,
                COALESCE((SELECT last_served_at FROM scheduler_routines WHERE routine_id=?1), ?2))
             WHERE routine_id=?1 AND live=1",
            params![routine_id, now_ms],
        )?;
    }
    Ok(())
}

/// Keep a tombstone so IDs/nonces cannot be recycled into duplicate runs and
/// occurrence history survives deletion. It no longer participates in ticks.
pub fn delete(conn: &Connection, routine_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE scheduler_routines SET deleted=1, enabled=0 WHERE routine_id=?1",
        [routine_id],
    )?;
    Ok(())
}

pub(crate) fn require(conn: &Connection, id: &str) -> Result<Routine> {
    get(conn, id)?.ok_or_else(|| Error::InvalidInput("unknown routine".into()))
}

pub(crate) fn validate(routine: &Routine, reference_ms: i64) -> Result<()> {
    validate_keeping(routine, reference_ms, &HashSet::new())
}

/// [`validate`], except that the group members named in `kept` are unchanged
/// since they were stored: a one-shot among them may have fired already, and that
/// must not stop an edit of the rest of the routine.
fn validate_keeping(routine: &Routine, reference_ms: i64, kept: &HashSet<String>) -> Result<()> {
    if [
        &routine.routine_id,
        &routine.bot_id,
        &routine.conversation_id,
        &routine.name,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
        || match &routine.body {
            RoutineBody::InlinePrompt { prompt } => prompt.trim().is_empty(),
            RoutineBody::SkillInvocation { skill_id, version } => {
                skill_id.trim().is_empty() || *version == 0
            }
        }
    {
        return Err(Error::InvalidInput(
            "routine requires IDs, name and body".into(),
        ));
    }
    jiff::tz::TimeZone::get(&routine.time_zone)
        .map_err(|error| Error::InvalidInput(error.to_string()))?;
    validate_group(&routine.trigger, &routine.time_zone, reference_ms, kept)
}

/// A group is a flat list of at least two simple triggers, each valid on its own.
/// Execution cadence is a local cost/safety policy, not a change to CR10's
/// parser grammar. One model run per minute is the fastest supported cadence.
pub(crate) fn validate_trigger(trigger: &Schedule, zone: &str, reference_ms: i64) -> Result<()> {
    validate_group(trigger, zone, reference_ms, &HashSet::new())
}

fn validate_group(
    trigger: &Schedule,
    zone: &str,
    reference_ms: i64,
    kept: &HashSet<String>,
) -> Result<()> {
    match trigger {
        Schedule::Webhook => Ok(()),
        Schedule::Group { members } => {
            if members.len() < 2 {
                return Err(Error::InvalidInput(
                    "a group needs at least two triggers".into(),
                ));
            }
            let mut ids = HashSet::new();
            for member in members {
                if member.id.trim().is_empty() || !ids.insert(member.id.as_str()) {
                    return Err(Error::InvalidInput(
                        "group triggers need distinct IDs".into(),
                    ));
                }
                if matches!(member.trigger, Schedule::Group { .. }) {
                    return Err(Error::InvalidInput("a group cannot contain a group".into()));
                }
                let spent =
                    matches!(member.trigger, Schedule::Once { .. }) && kept.contains(&member.id);
                if !spent {
                    validate_trigger(&member.trigger, zone, reference_ms)?;
                }
            }
            Ok(())
        }
        _ => {
            if matches!(trigger, Schedule::Every { interval_ms, .. } if *interval_ms < 60_000) {
                return Err(Error::InvalidInput(
                    "routine interval must be at least one minute".into(),
                ));
            }
            cron::next_after(trigger, zone, reference_ms)?.ok_or_else(|| {
                Error::InvalidInput("schedule never fires after its reference".into())
            })?;
            Ok(())
        }
    }
}

pub(crate) fn json<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&row.get::<_, String>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Routine> {
    Ok(Routine {
        routine_id: row.get(0)?,
        bot_id: row.get(1)?,
        conversation_id: row.get(2)?,
        name: row.get(3)?,
        body: json(row, 4)?,
        trigger: json(row, 5)?,
        time_zone: row.get(6)?,
        enabled: row.get(7)?,
        origin: if row.get::<_, String>(8)? == "model" {
            Origin::Model
        } else {
            Origin::User
        },
        untrusted: row.get(9)?,
        schedule_version: row.get(10)?,
        modified_at: row.get(11)?,
        last_served_at: row.get(12)?,
    })
}

#[cfg(test)]
pub(crate) mod routine_tests {
    use super::*;

    pub(crate) fn fixture() -> Routine {
        Routine {
            routine_id: "routine-1".into(),
            bot_id: "bot-1".into(),
            conversation_id: "routine-thread-1".into(),
            name: "Daily report".into(),
            body: RoutineBody::InlinePrompt {
                prompt: "Write a report".into(),
            },
            trigger: Schedule::Every {
                interval_ms: 60_000,
                phase_ms: None,
            },
            time_zone: "UTC".into(),
            enabled: true,
            origin: Origin::User,
            untrusted: false,
            schedule_version: 0,
            modified_at: 0,
            last_served_at: None,
        }
    }

    #[test]
    fn model_created_routine_is_paused() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.origin = Origin::Model;
        let mut stored = create(&conn, &input).unwrap();
        assert!(!stored.enabled);
        assert!(stored.untrusted);
        stored.enabled = true;
        stored = update(&conn, &stored, 1).unwrap();
        assert!(!stored.enabled);
        assert!(!get(&conn, &stored.routine_id).unwrap().unwrap().enabled);
        // Q5: only a confirmed conversation resumes through the normal toggle.
        set_enabled(&conn, &stored.routine_id, true, 2).unwrap();
        assert!(get(&conn, &stored.routine_id).unwrap().unwrap().enabled);
    }

    #[test]
    fn edit_bumps_schedule_version() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut stored = create(&conn, &fixture()).unwrap();
        assert_eq!(stored.schedule_version, 1);
        stored.name = "Renamed".into();
        stored = update(&conn, &stored, 1).unwrap();
        assert_eq!(stored.schedule_version, 1);
        stored.time_zone = "America/New_York".into();
        stored = update(&conn, &stored, 2).unwrap();
        assert_eq!(stored.schedule_version, 2);
        let stale = stored.clone();
        stored.trigger = Schedule::Once { at: 100 };
        stored = update(&conn, &stored, 3).unwrap();
        assert_eq!(stored.schedule_version, 3);
        assert!(update(&conn, &stale, 4).is_err());
        assert_eq!(get(&conn, &stored.routine_id).unwrap(), Some(stored));
    }

    #[test]
    fn webhook_routine_needs_no_future_slot() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.trigger = Schedule::Webhook;
        let mut stored = create(&conn, &input).unwrap();
        assert_eq!(stored.trigger, Schedule::Webhook);
        stored.name = "Renamed".into();
        stored = update(&conn, &stored, 1).unwrap();
        assert_eq!(stored.schedule_version, 1);
        assert_eq!(get(&conn, &stored.routine_id).unwrap(), Some(stored));
    }

    #[test]
    fn sub_minute_interval_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.trigger = cron::parse("@every 1ms").unwrap();
        assert!(create(&conn, &input).is_err());
        input.trigger = cron::parse("@every 59.999s").unwrap();
        assert!(create(&conn, &input).is_err());
        let mut stored = create(&conn, &fixture()).unwrap();
        stored.trigger = input.trigger;
        assert!(update(&conn, &stored, 1).is_err());
    }

    #[test]
    fn impossible_and_past_schedules_are_rejected_at_actual_edit_time() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.trigger = cron::parse("0 0 30 2 *").unwrap();
        assert!(create(&conn, &input).is_err());
        input.trigger = Schedule::Once { at: -1 };
        assert!(create(&conn, &input).is_err());
        let mut stored = create(&conn, &fixture()).unwrap();
        stored.trigger = Schedule::Once { at: 100 };
        assert!(update(&conn, &stored, 200).is_err());
    }

    #[test]
    fn metadata_edits_are_monotonic_and_reject_lost_updates() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.modified_at = 10;
        let original = create(&conn, &input).unwrap();
        let mut edit_a = original.clone();
        edit_a.name = "A".into();
        let mut stored = update(&conn, &edit_a, 10).unwrap();
        assert_eq!(stored.modified_at, 11);
        let mut edit_b = original;
        edit_b.body = RoutineBody::InlinePrompt { prompt: "B".into() };
        assert!(update(&conn, &edit_b, 100).is_err());
        stored.body = edit_b.body;
        stored = update(&conn, &stored, 5).unwrap();
        assert_eq!(stored.modified_at, 12);
        assert_eq!(stored.name, "A");
        assert_eq!(
            get(&conn, &stored.routine_id).unwrap(),
            Some(stored.clone())
        );
        // A concurrent writer/deletion or trigger that suppresses the write
        // must not produce a successful DTO for a change that did not happen.
        conn.execute_batch(
            "CREATE TRIGGER ignore_routine_update BEFORE UPDATE ON scheduler_routines
            BEGIN SELECT RAISE(IGNORE); END",
        )
        .unwrap();
        assert!(update(&conn, &stored, 13).is_err());
    }

    #[test]
    fn routine_migration_upgrades_pre_review_cursor_schema() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.modified_at = 123;
        create(&conn, &input).unwrap();
        conn.execute_batch("ALTER TABLE scheduler_routines DROP COLUMN active_since")
            .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let active_since: i64 = conn
            .query_row("SELECT active_since FROM scheduler_routines", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(active_since, 123);
    }

    #[test]
    fn routine_migration_rejects_unknown_schema() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "UPDATE component_versions SET version=2 WHERE component='scheduler.routine'",
            [],
        )
        .unwrap();
        assert!(migrate(&conn).is_err());
    }

    #[test]
    fn routine_crud_validates_and_preserves_provenance_and_history() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 42).unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let mut input = fixture();
        input.time_zone = "not-a-zone".into();
        assert!(create(&conn, &input).is_err());
        input = fixture();
        input.body = RoutineBody::SkillInvocation {
            skill_id: "skill-1".into(),
            version: 7,
        };
        input.origin = Origin::Model;
        let mut stored = create(&conn, &input).unwrap();
        stored.origin = Origin::User;
        stored.untrusted = false;
        stored.last_served_at = Some(i64::MAX);
        let stored = update(&conn, &stored, 1).unwrap();
        assert_eq!(stored.origin, Origin::Model);
        assert!(stored.untrusted);
        assert_eq!(stored.last_served_at, None);
        assert_eq!(list(&conn, "bot-1").unwrap(), vec![stored]);
        assert!(list(&conn, "bot-2").unwrap().is_empty());
        delete(&conn, "routine-1").unwrap();
        delete(&conn, "routine-1").unwrap();
        assert!(get(&conn, "routine-1").unwrap().is_none());
        assert!(list(&conn, "bot-1").unwrap().is_empty());
        assert!(create(&conn, &input).is_err());
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
    }
}
