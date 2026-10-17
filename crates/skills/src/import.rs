//! Text conversion signatures. Rules: server-orchestration-residual.md SR03/SR04.
//! `convert` is pure; `import_batch` will persist serially with per-file outcomes.
use crate::library::SkillContent;
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MAX_SKILL_CHARACTERS: usize = 256_000;
pub const MAX_SKILLS_PER_BOT: usize = 250;
pub const IMPORT_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const IMPORT_CLIENT_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ImportFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSkill {
    pub content: SkillContent,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportStatus {
    Written { skill_id: String },
    Skipped { reason: String },
    Duplicate { skill_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportItem {
    pub file_name: String,
    pub status: ImportStatus,
}

pub fn convert(_file: &ImportFile) -> Result<ImportedSkill> {
    pending("skills.import")
}
pub fn import_batch(
    _conn: &Connection,
    _bot_id: &str,
    _files: &[ImportFile],
) -> Result<Vec<ImportItem>> {
    pending("skills.import")
}
