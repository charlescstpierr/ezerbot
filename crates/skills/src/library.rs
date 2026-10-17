//! Versioned skill library signatures; storage is supplied by gb-skills-library.
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    Upload { extension: String, name: String },
    Manual,
    Model,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillContent {
    pub name: String,
    pub description: String,
    pub body: String,
    pub origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub skill_id: String,
    pub bot_id: String,
    pub content: SkillContent,
    pub version: u64,
    pub digest: String,
    pub deleted: bool,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn add(_conn: &Connection, _bot_id: &str, _content: &SkillContent) -> Result<Skill> {
    pending("skills.library")
}
pub fn update(
    _conn: &Connection,
    _skill_id: &str,
    _expected_version: u64,
    _content: &SkillContent,
) -> Result<Skill> {
    pending("skills.library")
}
pub fn delete(_conn: &Connection, _skill_id: &str, _expected_version: u64) -> Result<()> {
    pending("skills.library")
}
pub fn list(_conn: &Connection, _bot_id: &str) -> Result<Vec<Skill>> {
    pending("skills.library")
}
pub fn get_version(_conn: &Connection, _skill_id: &str, _version: u64) -> Result<Option<Skill>> {
    pending("skills.library")
}
pub fn history(_conn: &Connection, _skill_id: &str) -> Result<Vec<Skill>> {
    pending("skills.library")
}
