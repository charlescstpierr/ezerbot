//! Local-team promotion signatures (goal-context-memory-learning.md CM09–CM12).
//! Team means every bot in the single local workspace; sensitive facts stay private.
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const MAX_PROMOTION_BATCH: usize = 100;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionReport {
    pub created: Vec<String>,
    pub already_in_team: Vec<String>,
    pub kept_private_count: usize,
}
pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn promote(_conn: &Connection, _bot_id: &str, _fact_ids: &[String]) -> Result<PromotionReport> {
    pending("memory.promotion")
}
