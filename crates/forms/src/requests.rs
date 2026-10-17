//! Typed human-input requests, implemented by gb-forms-core.
//! Resume receipts address server-orchestration-residual.md SR01/SR02.
use grokbot_engine::contracts::{pending, ApprovalRoute, Result, ResumeReceipt, SecretRef};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Number,
    Choice { options: Vec<String> },
    Boolean,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub field_type: FieldType,
    pub required: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputKind {
    Form {
        fields: Vec<Field>,
    },
    Approval {
        route: ApprovalRoute,
        tool: String,
        args_digest: String,
        expires_at: i64,
    },
    Secret {
        name: String,
        purpose: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    Resolved { resolution_id: String },
    Denied,
    Expired,
    Cancelled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanInputRequest {
    pub id: String,
    pub run_id: String,
    pub entry_id: String,
    pub kind: InputKind,
    pub status: RequestStatus,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn create(_conn: &Connection, _request: &HumanInputRequest) -> Result<HumanInputRequest> {
    pending("forms.requests")
}
pub fn get(_conn: &Connection, _id: &str) -> Result<Option<HumanInputRequest>> {
    pending("forms.requests")
}
pub fn list_for_run(_conn: &Connection, _run_id: &str) -> Result<Vec<HumanInputRequest>> {
    pending("forms.requests")
}
/// Resolution and enqueueing the wake share the caller's transaction.
pub fn submit(
    _conn: &Connection,
    _entry_id: &str,
    _submission_id: &str,
    _values: &Value,
) -> Result<ResumeReceipt> {
    pending("forms.requests")
}
pub fn submit_secret_ref(
    _conn: &Connection,
    _entry_id: &str,
    _submission_id: &str,
    _secret: &SecretRef,
) -> Result<ResumeReceipt> {
    pending("forms.requests")
}
pub fn receipt(_conn: &Connection, _entry_id: &str, _submission_id: &str) -> Result<ResumeReceipt> {
    pending("forms.requests")
}
pub fn cancel_for_run(_conn: &Connection, _run_id: &str) -> Result<usize> {
    pending("forms.requests")
}
