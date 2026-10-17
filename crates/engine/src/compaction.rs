//! Non-destructive summary boundary, implemented by gb-engine-compaction.
//! ConversationSummary fields derive from server-context-residual.md RC06.
use crate::contracts::{pending, ContextBudget, HistoryItem, ModelGateway, Result, Summary};
use rusqlite::Connection;

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn should_compact(_characters: usize, _budget: &ContextBudget) -> Result<bool> {
    pending("engine.compaction")
}
pub fn summarize(
    _gateway: &dyn ModelGateway,
    _model: &str,
    _conversation_id: &str,
    _history: &[HistoryItem],
    _previous: Option<&Summary>,
) -> Result<Summary> {
    pending("engine.compaction")
}
pub fn save(_conn: &Connection, _summary: &Summary) -> Result<()> {
    pending("engine.compaction")
}
pub fn latest(_conn: &Connection, _conversation_id: &str) -> Result<Option<Summary>> {
    pending("engine.compaction")
}
pub fn invalidate_covering(
    _conn: &Connection,
    _conversation_id: &str,
    _entry_id: &str,
) -> Result<usize> {
    pending("engine.compaction")
}
