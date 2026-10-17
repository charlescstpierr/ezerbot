//! Precaption projection/cache signatures (server-context-residual.md RC01).
//! Only five pure human messages plus the draft enter the caption context.
use grokbot_engine::contracts::{pending, Result, Role};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEBOUNCE_MS: u64 = 1_000;
pub const CAPTION_DEADLINE_MS: u64 = 150_000;
pub const SKIPPED_BACKOFF_MS: u64 = 600_000;
pub const HUMAN_CONTEXT_MESSAGES: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionMessage {
    pub entry_id: String,
    pub role: Role,
    pub text: String,
    pub channel: Option<String>,
    pub author: Option<Value>,
    pub from_agent: bool,
    pub branch: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrecaptionContext {
    pub draft: String,
    pub human_messages: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheKey {
    pub sha256: String,
    pub model: String,
    pub context_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CaptionStatus {
    Pending,
    Ready { text: String },
    Skipped { retry_after: i64 },
    Failed { reason: String },
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn project(_draft: &str, _messages: &[CaptionMessage]) -> Result<PrecaptionContext> {
    pending("attachments.precaption")
}
pub fn cache_key(_sha256: &str, _model: &str, _context: &PrecaptionContext) -> Result<CacheKey> {
    pending("attachments.precaption")
}
pub fn cached(_conn: &Connection, _key: &CacheKey) -> Result<Option<CaptionStatus>> {
    pending("attachments.precaption")
}
pub fn save(_conn: &Connection, _key: &CacheKey, _status: &CaptionStatus) -> Result<()> {
    pending("attachments.precaption")
}
