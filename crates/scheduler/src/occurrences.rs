//! Durable occurrence and manual-nonce signatures, implemented by gb-scheduler-occurrences.
//! Q4: one catch-up run in the late window; older slots are explicitly Missed.
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
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

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn materialize_due(
    _conn: &Connection,
    _now_ms: i64,
    _policy: &CatchUpPolicy,
) -> Result<Vec<Occurrence>> {
    pending("scheduler.occurrences")
}
pub fn run_now(
    _conn: &Connection,
    _routine_id: &str,
    _nonce: &str,
    _now_ms: i64,
) -> Result<Occurrence> {
    pending("scheduler.occurrences")
}
pub fn list(_conn: &Connection, _routine_id: &str) -> Result<Vec<Occurrence>> {
    pending("scheduler.occurrences")
}
pub fn attach_run(_conn: &Connection, _occurrence_id: &str, _run_id: &str) -> Result<()> {
    pending("scheduler.occurrences")
}
