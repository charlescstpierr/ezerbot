//! Parent/child graph and selected-context signatures (delegation-lifecycle-deep.md).
use grokbot_engine::contracts::{pending, ContextBlock, Result, SelectedContext};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DelegationStatus {
    Queued,
    Running,
    Done { result: String },
    Failed { reason: String },
    Refused { reason: String },
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    pub id: String,
    pub parent_run_id: String,
    pub root_run_id: String,
    pub parent_bot: String,
    pub child_bot: String,
    pub depth: u32,
    pub child_run_id: Option<String>,
    pub conversation_id: Option<String>,
    pub selected_context: SelectedContext,
    pub status: DelegationStatus,
}

/// Return None for unreadable resources; never hand the child a parent's full history.
pub trait ContextResolver {
    fn entry(&self, parent_bot: &str, entry_id: &str) -> Result<Option<String>>;
    fn fact(&self, parent_bot: &str, fact_id: &str) -> Result<Option<String>>;
    fn skill(&self, parent_bot: &str, skill_id: &str) -> Result<Option<String>>;
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn create(_conn: &Connection, _delegation: &Delegation) -> Result<Delegation> {
    pending("delegation.graph")
}
pub fn get(_conn: &Connection, _id: &str) -> Result<Option<Delegation>> {
    pending("delegation.graph")
}
pub fn descendants(_conn: &Connection, _parent_run_id: &str) -> Result<Vec<Delegation>> {
    pending("delegation.graph")
}
pub fn ancestor_bots(_conn: &Connection, _parent_run_id: &str) -> Result<Vec<String>> {
    pending("delegation.graph")
}
pub fn set_status(_conn: &Connection, _id: &str, _status: &DelegationStatus) -> Result<()> {
    pending("delegation.graph")
}
pub fn cancel_descendants(_conn: &Connection, _parent_run_id: &str) -> Result<Vec<Delegation>> {
    pending("delegation.graph")
}
pub fn resolve_selected_context(
    _resolver: &dyn ContextResolver,
    _parent_bot: &str,
    _selected: &SelectedContext,
    _budget_characters: usize,
) -> Result<Vec<ContextBlock>> {
    pending("delegation.graph")
}
