//! Persisted run lifecycle, independent of task status and turn completion.
//! REA: `reports/goal-execution-autonomy.md` §14 distinguishes these three axes;
//! transition rules, trigger deduplication and budgets are local design choices.
//! Mutations take part in the caller's transaction (use `contracts::with_tx`).
use crate::contracts::{
    Error, Result, RunBudgets, RunEventKind, RunKind, TaskStatus, Trigger, TurnEnd, WakeCause,
};
use crate::store;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;

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

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finished | Self::Failed | Self::Cancelled)
    }

    pub fn is_suspended(self) -> bool {
        matches!(
            self,
            Self::AwaitingApproval | Self::AwaitingHumanInput | Self::AwaitingChildren
        )
    }

    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        if self.is_terminal() {
            return false;
        }
        self == next
            || matches!(next, Self::Cancelled | Self::Failed)
            || match self {
                Self::Queued => next == Self::Running,
                Self::Running => {
                    next.is_suspended() || matches!(next, Self::Interrupted | Self::Finished)
                }
                Self::Interrupted => next == Self::Running,
                status if status.is_suspended() => next == Self::Running,
                _ => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub bot_id: String,
    pub conversation_id: String,
    pub trigger: Trigger,
    pub kind: RunKind,
    pub status: RunStatus,
    /// Always None for a Chat run; a turn ending does not complete a Task.
    pub task_status: Option<TaskStatus>,
    /// Superseded is represented here, with lifecycle Cancelled; it is not a
    /// task status and does not make a Task Done.
    pub turn_end: Option<TurnEnd>,
    pub budgets: RunBudgets,
    pub started_at: i64,
}

impl Run {
    pub(crate) fn validate(&self) -> Result<()> {
        let (_, reference, _) = store::trigger_key(&self.trigger);
        if [&self.run_id, &self.bot_id, &self.conversation_id, reference]
            .iter()
            .any(|id| id.is_empty())
        {
            return Err(invalid("run identifiers must not be empty"));
        }
        if (self.kind == RunKind::Chat) != self.task_status.is_none() {
            return Err(invalid("only Task runs carry a task status"));
        }
        if matches!(
            self.trigger,
            Trigger::Occurrence { .. } | Trigger::Delegation { .. }
        ) && self.kind != RunKind::Task
        {
            return Err(invalid("occurrence and delegation runs must be Tasks"));
        }
        if matches!(self.trigger, Trigger::DelegationReturn { .. }) && self.kind != RunKind::Chat {
            return Err(invalid("delegation returns must be Chats"));
        }
        let valid_end = match self.status {
            RunStatus::Cancelled => {
                matches!(self.turn_end, Some(TurnEnd::Stopped | TurnEnd::Superseded))
            }
            RunStatus::Finished => matches!(
                self.turn_end,
                Some(TurnEnd::Finished | TurnEnd::BudgetExhausted)
            ),
            RunStatus::Failed => self.turn_end.is_none(),
            _ => self.turn_end.is_none(),
        };
        if !valid_end {
            return Err(invalid("turn end does not match the run lifecycle"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub elapsed_ms: u64,
}

impl BudgetUsage {
    /// Check before starting another call. Reaching any bound exhausts the run;
    /// the loop records BudgetExhausted without changing Working into Done.
    pub fn exhausted(&self, budgets: &RunBudgets) -> bool {
        self.model_calls >= budgets.model_calls
            || self.tool_calls >= budgets.tool_calls
            || self.elapsed_ms >= budgets.duration_ms
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput(message.into())
}

/// Create once per trigger, returning the existing run on retry. Regeneration
/// supersedes the latest attempt, even if its previous turn already finished.
/// Concurrent DEFERRED transactions may return SQLITE_BUSY (including a stale
/// WAL snapshot) or a uniqueness error. The caller must roll back and retry the
/// WHOLE transaction; retrying a statement within the same snapshot is unsafe.
pub fn start(conn: &Connection, run: &Run) -> Result<Run> {
    store::require_transaction(conn)?;
    run.validate()?;
    let (kind, reference, attempt) = store::trigger_key(&run.trigger);
    if let Some(existing) = store::by_trigger(conn, kind, reference, attempt)? {
        if existing.bot_id != run.bot_id || existing.conversation_id != run.conversation_id {
            return Err(invalid("trigger belongs to another bot or conversation"));
        }
        return Ok(existing);
    }
    if !matches!(run.status, RunStatus::Queued | RunStatus::Running)
        || run.turn_end.is_some()
        || (run.kind == RunKind::Task && run.task_status != Some(TaskStatus::Working))
    {
        return Err(invalid(
            "new runs must be queued or running, with no completed work",
        ));
    }
    // Check the proposed ID before superseding anything.
    if store::get(conn, &run.run_id)?.is_some() {
        return Err(invalid("run_id already belongs to another trigger"));
    }
    if let Some(previous) = store::latest_attempt(conn, kind, reference)? {
        let (_, _, last_attempt) = store::trigger_key(&previous.trigger);
        if previous.bot_id != run.bot_id
            || previous.conversation_id != run.conversation_id
            || last_attempt.checked_add(1) != Some(attempt)
        {
            return Err(invalid(
                "regeneration must use the next attempt of the same conversation",
            ));
        }
        cancel(conn, &previous.run_id, TurnEnd::Superseded)?;
    }
    store::insert(conn, run)?;
    store::append_event(
        conn,
        &run.run_id,
        RunEventKind::Started,
        &json!({"trigger": run.trigger}),
    )?;
    Ok(run.clone())
}

/// Stopped and Superseded are turn ends, not task completion. Repeated
/// cancellation is a no-op; a late tool result cannot revive this run.
pub fn cancel(conn: &Connection, run_id: &str, end: TurnEnd) -> Result<Run> {
    store::require_transaction(conn)?;
    if !matches!(end, TurnEnd::Stopped | TurnEnd::Superseded) {
        return Err(invalid("cancellation requires Stopped or Superseded"));
    }
    let mut run = store::required(conn, run_id)?;
    if run.status.is_terminal() && (end != TurnEnd::Superseded || run.turn_end == Some(end)) {
        return Ok(run);
    }
    run.status = RunStatus::Cancelled;
    run.turn_end = Some(end);
    if run.task_status == Some(TaskStatus::NeedsInput) {
        run.task_status = Some(TaskStatus::Working);
    }
    store::save(conn, &run)?;
    store::append_event(
        conn,
        run_id,
        RunEventKind::Cancelled,
        &json!({"turn_end": end}),
    )?;
    Ok(run)
}

fn resume_wakes(conn: &Connection, run: &Run) -> Result<Option<Vec<store::Wake>>> {
    if run.status == RunStatus::Interrupted {
        return Ok(Some(Vec::new()));
    }
    if !run.status.is_suspended() {
        return Ok(None);
    }
    let references = store::awaiting_references(conn, &run.run_id)?;
    let wakes: Vec<_> = store::pending_wakes(conn)?
        .into_iter()
        .filter(|wake| {
            wake.run_id == run.run_id
                && references.contains(&wake.reference)
                && match run.status {
                    RunStatus::AwaitingApproval => wake.cause == WakeCause::Approval,
                    RunStatus::AwaitingChildren => wake.cause == WakeCause::Child,
                    RunStatus::AwaitingHumanInput => {
                        matches!(wake.cause, WakeCause::Form | WakeCause::Secret)
                    }
                    _ => false,
                }
        })
        .collect();
    let ready = !references.is_empty()
        && references
            .iter()
            .all(|reference| wakes.iter().any(|wake| &wake.reference == reference));
    Ok(ready.then_some(wakes))
}

/// Use this instead of an any-pending-wake check when driving a suspended run.
/// False means keep waiting, not a failed task. Resume revalidates in its write
/// transaction; Interrupted is ready but does not consume pending resolutions.
pub fn ready_to_resume(conn: &Connection, run_id: &str) -> Result<bool> {
    Ok(resume_wakes(conn, &store::required(conn, run_id)?)?.is_some())
}

/// Resume the same run, retaining its committed event prefix and budget usage.
/// A suspended run needs a durable wake; interruption recovery does not. The
/// Resumed event and all corresponding delivery receipts commit atomically.
pub fn resume(conn: &Connection, run_id: &str) -> Result<Run> {
    resume_at(conn, run_id, store::now_ms())
}

/// Deterministic-clock equivalent of resume. Interruption recovery consumes no
/// wakes: the run was Running, so queued resolutions still belong to a later
/// suspension. Children resume together only when every expected child is ready.
pub fn resume_at(conn: &Connection, run_id: &str, now_ms: i64) -> Result<Run> {
    store::require_transaction(conn)?;
    let mut run = store::required(conn, run_id)?;
    if run.status == RunStatus::Running {
        return Ok(run);
    }
    if run.status != RunStatus::Interrupted && !run.status.is_suspended() {
        return Err(invalid("only interrupted or suspended runs can resume"));
    }
    let wakes = resume_wakes(conn, &run)?
        .ok_or_else(|| invalid("suspended run is still waiting for its referenced resolutions"))?;
    run.status = RunStatus::Running;
    if run.task_status == Some(TaskStatus::NeedsInput) {
        run.task_status = Some(TaskStatus::Working);
    }
    store::save_at(conn, &run, now_ms)?;
    let event = store::append_event(
        conn,
        run_id,
        RunEventKind::Resumed,
        &json!({"wake_ids": wakes.iter().map(|wake| &wake.wake_id).collect::<Vec<_>>()}),
    )?;
    for wake in wakes {
        store::mark_wake_delivered(conn, &wake.wake_id, event.seq)?;
    }
    Ok(run)
}

/// Promote a queued run without charging its queue residence to the budget.
pub fn set_running_at(conn: &Connection, run_id: &str, now_ms: i64) -> Result<Run> {
    let mut run = store::required(conn, run_id)?;
    if run.status != RunStatus::Queued {
        return Err(invalid("only a queued run can start running"));
    }
    run.status = RunStatus::Running;
    store::save_at(conn, &run, now_ms)?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::with_tx;

    fn sample(id: &str, attempt: u32) -> Run {
        Run {
            run_id: id.into(),
            bot_id: "bot".into(),
            conversation_id: "conversation".into(),
            trigger: Trigger::UserMessage {
                message_id: "message".into(),
                attempt,
            },
            kind: RunKind::Chat,
            status: RunStatus::Queued,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 1000,
        }
    }

    #[test]
    fn model_same_trigger_attempt_starts_one_run() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        store::migrate(&conn)?;
        with_tx(&conn, |conn| {
            let run = start(conn, &sample("run-1", 0))?;
            assert_eq!(start(conn, &sample("unused-id", 0))?, run);
            assert_eq!(store::events(conn, "run-1", 0)?.len(), 1);
            assert!(store::get(conn, "unused-id")?.is_none());
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn delegation_return_is_one_chat_per_origin_without_task_completion() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        store::migrate(&conn)?;
        with_tx(&conn, |conn| {
            let mut proposed = sample("return", 0);
            proposed.trigger = Trigger::DelegationReturn {
                parent_run_id: "origin".into(),
            };
            let saved = start(conn, &proposed)?;
            proposed.run_id = "retry".into();
            assert_eq!(start(conn, &proposed)?, saved);
            assert_eq!(saved.kind, RunKind::Chat);
            assert_eq!(saved.task_status, None);
            assert_eq!(store::events(conn, &saved.run_id, 0)?.len(), 1);
            proposed.kind = RunKind::Task;
            proposed.task_status = Some(TaskStatus::Working);
            assert!(start(conn, &proposed).is_err());
            assert!(store::get(conn, "retry")?.is_none());
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn model_regenerate_supersedes_previous_attempt() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        store::migrate(&conn)?;
        with_tx(&conn, |conn| {
            start(conn, &sample("run-1", 0))?;
            let next = start(conn, &sample("run-2", 1))?;
            let old = store::required(conn, "run-1")?;
            assert_eq!(old.status, RunStatus::Cancelled);
            assert_eq!(old.turn_end, Some(TurnEnd::Superseded));
            assert_eq!(next.turn_end, None);
            assert_eq!(start(conn, &sample("retry", 0))?, old);
            assert!(start(conn, &sample("gap", 3)).is_err());
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn model_queued_run_can_be_cancelled() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        store::migrate(&conn)?;
        with_tx(&conn, |conn| {
            start(conn, &sample("run", 0))?;
            let cancelled = cancel(conn, "run", TurnEnd::Stopped)?;
            assert_eq!(cancelled.status, RunStatus::Cancelled);
            assert_eq!(cancel(conn, "run", TurnEnd::Stopped)?, cancelled);
            assert_eq!(store::events(conn, "run", 0)?.len(), 2);
            assert!(resume(conn, "run").is_err());
            Ok::<_, Error>(())
        })
    }

    #[test]
    fn model_budget_axes_are_independent() {
        let limits = RunBudgets::default();
        assert_eq!(
            (limits.model_calls, limits.tool_calls, limits.duration_ms),
            (12, 24, 600_000)
        );
        assert!(!BudgetUsage::default().exhausted(&limits));
        for usage in [
            BudgetUsage {
                model_calls: 12,
                ..BudgetUsage::default()
            },
            BudgetUsage {
                tool_calls: 24,
                ..BudgetUsage::default()
            },
            BudgetUsage {
                elapsed_ms: 600_000,
                ..BudgetUsage::default()
            },
        ] {
            assert!(usage.exhausted(&limits));
        }
        let mut run = sample("task", 0);
        run.kind = RunKind::Task;
        run.task_status = Some(TaskStatus::Working);
        run.status = RunStatus::Finished;
        run.turn_end = Some(TurnEnd::BudgetExhausted);
        assert!(run.validate().is_ok());
        assert_eq!(run.task_status, Some(TaskStatus::Working));
    }
}
