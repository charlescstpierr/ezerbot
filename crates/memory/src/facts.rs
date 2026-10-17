//! Fact signatures; persistence/correction/forgetting belong to gb-memory-core.
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Private,
    Team,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserStated,
    ModelInferred,
    Imported,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub conversation_id: Option<String>,
    pub entry_id: Option<String>,
    pub source: Source,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FactStatus {
    Active,
    Superseded { by: String },
    Forgotten,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub fact_id: String,
    pub bot_id: String,
    pub scope: Scope,
    pub text: String,
    pub provenance: Provenance,
    pub learned_at: i64,
    pub status: FactStatus,
    pub sensitive: bool,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn remember(_conn: &Connection, _fact: &Fact) -> Result<Fact> {
    pending("memory.facts")
}
pub fn get(_conn: &Connection, _fact_id: &str) -> Result<Option<Fact>> {
    pending("memory.facts")
}
pub fn list_active(_conn: &Connection, _bot_id: &str) -> Result<Vec<Fact>> {
    pending("memory.facts")
}
pub fn correct(_conn: &Connection, _fact_id: &str, _new_text: &str) -> Result<Fact> {
    pending("memory.facts")
}
pub fn forget(_conn: &Connection, _fact_id: &str) -> Result<()> {
    pending("memory.facts")
}
