//! Durable routines. The caller owns transactions, including migrations and edits.
//! `reports/cron-detail.md` CR01 permits an inline prompt without a skill. CR19
//! creates model routines paused/untrusted, then resumes after conversational
//! confirmation (Q5 steering); this store adds no separate approval gate.
use crate::cron::{self, Schedule};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, types::Type, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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
            deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
         );
         CREATE INDEX IF NOT EXISTS scheduler_routines_bot ON scheduler_routines(bot_id);
         INSERT INTO component_versions(component, version)
            VALUES ('scheduler.routine', 1) ON CONFLICT(component) DO NOTHING;",
    )?;
    Ok(())
}

/// Caller supplies stable IDs and creation time; version/progress/provenance
/// flags are normalized here, not trusted from a model's DTO.
pub fn create(conn: &Connection, routine: &Routine) -> Result<Routine> {
    validate(routine)?;
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
          enabled, origin, untrusted, schedule_version, modified_at, materialized_after)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?11)",
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
    Ok(routine)
}

/// A schedule edit starts strictly after the edit and the last served slot.
/// Metadata edits keep the cursor/interval anchor, and cannot forge progress or
/// change the owning bot, conversation or origin. Use the returned version on
/// subsequent edits. Compose this read/write in the caller's transaction.
pub fn update(conn: &Connection, routine: &Routine, now_ms: i64) -> Result<Routine> {
    validate(routine)?;
    let old = require(conn, &routine.routine_id)?;
    if routine.bot_id != old.bot_id || routine.conversation_id != old.conversation_id {
        return Err(Error::InvalidInput(
            "routine owner/conversation is immutable".into(),
        ));
    }
    if routine.schedule_version != old.schedule_version || now_ms < old.modified_at {
        return Err(Error::InvalidInput("stale routine edit".into()));
    }
    let changed = routine.trigger != old.trigger || routine.time_zone != old.time_zone;
    let version = old.schedule_version + u64::from(changed);
    if version > i64::MAX as u64 {
        return Err(Error::InvalidInput("schedule version exhausted".into()));
    }
    let mut updated = routine.clone();
    updated.origin = old.origin;
    updated.untrusted = old.untrusted;
    updated.last_served_at = old.last_served_at;
    updated.schedule_version = version;
    updated.modified_at = now_ms;
    conn.execute(
        "UPDATE scheduler_routines SET name=?2, body=?3, trigger=?4, time_zone=?5,
         enabled=?6, schedule_version=?7, modified_at=?8,
         materialized_after=CASE WHEN ?9 THEN MAX(?8, COALESCE(last_served_at, ?8))
                                ELSE materialized_after END
         WHERE routine_id=?1 AND deleted=0 AND schedule_version=?10",
        params![
            routine.routine_id,
            routine.name,
            serde_json::to_string(&routine.body)?,
            serde_json::to_string(&routine.trigger)?,
            routine.time_zone,
            routine.enabled,
            version,
            now_ms,
            changed,
            old.schedule_version,
        ],
    )?;
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
/// Pausing preserves history and the schedule anchor; resume uses catch-up policy.
pub fn set_enabled(conn: &Connection, routine_id: &str, enabled: bool) -> Result<()> {
    if conn.execute(
        "UPDATE scheduler_routines SET enabled=?2 WHERE routine_id=?1 AND deleted=0",
        params![routine_id, enabled],
    )? == 0
    {
        return Err(Error::InvalidInput("unknown routine".into()));
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

fn validate(routine: &Routine) -> Result<()> {
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
    cron::next_after(&routine.trigger, &routine.time_zone, routine.modified_at)?;
    Ok(())
}

fn json<T: DeserializeOwned>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
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
        let stored = create(&conn, &input).unwrap();
        assert!(!stored.enabled);
        assert!(stored.untrusted);
        // Q5: a confirmed conversation resumes through the normal toggle.
        set_enabled(&conn, &stored.routine_id, true).unwrap();
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
