//! Run DTOs and transition signatures; implemented by gb-engine-model.
use crate::contracts::{pending, Result, RunBudgets, RunKind, TaskStatus, Trigger, TurnEnd};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Persisted lifecycle; separate from task status and the end of a model turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    AwaitingApproval,
    AwaitingHumanInput,
    AwaitingChildren,
    Interrupted,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub trigger: Trigger,
    pub kind: RunKind,
    pub status: RunStatus,
    /// Always None for a Chat run.
    pub task_status: Option<TaskStatus>,
    pub turn_end: Option<TurnEnd>,
    pub budgets: RunBudgets,
    pub started_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub elapsed_ms: u64,
}

pub fn start(_conn: &Connection, _run: &Run) -> Result<Run> {
    pending("engine.model")
}
pub fn cancel(_conn: &Connection, _run_id: &str, _end: TurnEnd) -> Result<Run> {
    pending("engine.model")
}
pub fn resume(_conn: &Connection, _run_id: &str) -> Result<Run> {
    pending("engine.model")
}
