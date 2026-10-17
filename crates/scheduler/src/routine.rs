//! Routine storage signatures, implemented by gb-scheduler-occurrences.
//! Q5 steering: model activation follows confirmation in the conversation,
//! as reports/cron-detail.md CR19 describes; no formal approval is imposed here.
use crate::cron::Schedule;
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

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

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn create(_conn: &Connection, _routine: &Routine) -> Result<Routine> {
    pending("scheduler.routine")
}
pub fn update(_conn: &Connection, _routine: &Routine, _now_ms: i64) -> Result<Routine> {
    pending("scheduler.routine")
}
pub fn get(_conn: &Connection, _routine_id: &str) -> Result<Option<Routine>> {
    pending("scheduler.routine")
}
pub fn list(_conn: &Connection, _bot_id: &str) -> Result<Vec<Routine>> {
    pending("scheduler.routine")
}
pub fn set_enabled(_conn: &Connection, _routine_id: &str, _enabled: bool) -> Result<()> {
    pending("scheduler.routine")
}
pub fn delete(_conn: &Connection, _routine_id: &str) -> Result<()> {
    pending("scheduler.routine")
}
