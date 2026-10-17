//! Append-only events and durable wake queue signatures, owned by gb-engine-model.
//! Effects and their result events must be written in the caller's same transaction.
use crate::contracts::{pending, Result, RunEvent, RunEventKind, WakeCause, WakeStatus};
use crate::model::Run;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wake {
    pub wake_id: String,
    pub run_id: String,
    pub cause: WakeCause,
    pub reference: String,
    pub status: WakeStatus,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn get(_conn: &Connection, _run_id: &str) -> Result<Option<Run>> {
    pending("engine.store")
}
pub fn save(_conn: &Connection, _run: &Run) -> Result<()> {
    pending("engine.store")
}
pub fn append_event(
    _conn: &Connection,
    _run_id: &str,
    _kind: RunEventKind,
    _payload: &Value,
) -> Result<RunEvent> {
    pending("engine.store")
}
pub fn events(_conn: &Connection, _run_id: &str, _after_seq: u64) -> Result<Vec<RunEvent>> {
    pending("engine.store")
}
pub fn enqueue_wake(
    _conn: &Connection,
    _run_id: &str,
    _cause: WakeCause,
    _reference: &str,
) -> Result<Wake> {
    pending("engine.store")
}
pub fn pending_wakes(_conn: &Connection) -> Result<Vec<Wake>> {
    pending("engine.store")
}
pub fn wake_status(_conn: &Connection, _wake_id: &str) -> Result<WakeStatus> {
    pending("engine.store")
}
/// Call only in the transaction that commits the first post-wake run event.
pub fn mark_wake_delivered(
    _conn: &Connection,
    _wake_id: &str,
    _first_event_seq: u64,
) -> Result<()> {
    pending("engine.store")
}
pub fn interrupt_running(_conn: &Connection) -> Result<usize> {
    pending("engine.store")
}
pub fn redact_payloads(_conn: &Connection, _bot_id: &str, _text: &str) -> Result<usize> {
    pending("engine.store")
}
