//! Argument-bound, single-use approval signatures, implemented by gb-forms-core.
//! These generic approvals do not impose a routine activation policy.
use grokbot_engine::contracts::{pending, Result, ResumeReceipt};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    Deny,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn args_digest(_tool: &str, _arguments: &Value) -> Result<String> {
    pending("forms.approvals")
}
pub fn decide(
    _conn: &Connection,
    _id: &str,
    _submission_id: &str,
    _decision: ApprovalDecision,
    _now_ms: i64,
) -> Result<ResumeReceipt> {
    pending("forms.approvals")
}
pub fn consume_allow_once(
    _conn: &Connection,
    _id: &str,
    _tool: &str,
    _arguments: &Value,
    _now_ms: i64,
) -> Result<()> {
    pending("forms.approvals")
}
