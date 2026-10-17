//! Shared wire contracts. IDs are opaque strings (the domain IDs are not serde DTOs).
//! Times and durations use UTC Unix milliseconds unless a field says otherwise.
//! Secret values must never be put in these model/transcript-facing types.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    NotImplemented(&'static str),
    InvalidInput(String),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotImplemented(module) => write!(f, "{module} is not implemented yet"),
            Self::InvalidInput(message) => f.write_str(message),
            Self::Sqlite(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Fail closed while a capability's implementation slice is outstanding.
pub fn pending<T>(module: &'static str) -> Result<T> {
    Err(Error::NotImplemented(module))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    UserMessage { message_id: String, attempt: u32 },
    Occurrence { occurrence_id: String },
    Delegation { delegation_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Chat,
    Task,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatus {
    Working,
    NeedsInput,
    Done { summary: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEnd {
    Finished,
    Stopped,
    Superseded,
    BudgetExhausted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunBudgets {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub duration_ms: u64,
}
impl Default for RunBudgets {
    fn default() -> Self {
        Self {
            model_calls: 12,
            tool_calls: 24,
            duration_ms: 600_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub estimated_tokens: usize,
    pub characters_per_token: usize,
    pub compaction_percent: u8,
}
impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            estimated_tokens: 100_000,
            characters_per_token: 4,
            compaction_percent: 70,
        }
    }
}

/// Every approval is displayed in the principal bot's conversation, while its
/// resolution still wakes the originating run (including routine/child runs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRoute {
    pub requesting_bot_id: String,
    pub target_bot_id: String,
    pub target_conversation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub schema: Value,
    pub requires_approval: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuspendCause {
    HumanInput { id: String },
    Children { ids: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutcome {
    Done { output: Value },
    Failed { reason: String },
    Suspend { cause: SuspendCause },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeCause {
    Form,
    Approval,
    Secret,
    Child,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WakeStatus {
    Pending,
    /// Only persisted after the resumed run commits its first event.
    Delivered {
        run_id: String,
        first_event_seq: u64,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeReceipt {
    pub resolution_id: String,
    pub wake: WakeStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    Started,
    TextDelta,
    ToolCall,
    ToolResult,
    Suspended,
    Resumed,
    TurnEnded,
    TaskChanged,
    Interrupted,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub kind: RunEventKind,
    /// Sanitized payload only: no secret values or forgotten memory text.
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSource {
    AppContract,
    BotIdentity,
    BotInstructions,
    SkillIndex,
    Memory,
    Summary,
    History,
    CurrentMessage,
    ToolResult,
    ImageCaption,
    DelegatedContext,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBlock {
    pub source: ContextSource,
    /// Lower numbers are assembled first and retained before higher numbers.
    pub priority: u32,
    /// Character budget, not tokens.
    pub budget: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInput {
    pub attachment_id: String,
    /// A provider-ready URL/data URL, only on the image's sending turn.
    pub url: Option<String>,
    pub caption: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolItem {
    Call { call: ToolCall },
    Output { call_id: String, output: Value },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryItem {
    pub entry_id: String,
    pub role: Role,
    pub text: String,
    #[serde(default)]
    pub images: Vec<ImageInput>,
    #[serde(default)]
    pub tool_items: Vec<ToolItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRequest {
    pub run_id: String,
    pub trigger: Trigger,
    pub bot_id: String,
    pub conversation_id: String,
    pub current_message: HistoryItem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub summary_id: String,
    pub conversation_id: String,
    pub text: String,
    pub truncation_last_entry_inclusive: String,
    pub resume_from_entry_inclusive: String,
    pub previous_summary_id: Option<String>,
    pub includes_tool_results: bool,
    pub strategy: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedContext {
    pub texts: Vec<String>,
    pub entry_ids: Vec<String>,
    pub fact_ids: Vec<String>,
    pub skill_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model: String,
    pub instructions: String,
    /// Complete input on every call, including function calls and their outputs.
    pub input: Vec<HistoryItem>,
    pub tools: Vec<ToolSpec>,
    pub effort: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelEvent {
    TextDelta {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: Value,
    },
    Completed,
    Incomplete {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

/// Synchronous streaming boundary, usable behind a trait object and by scripted tests.
/// A callback error must stop generation (for example after cancellation).
pub trait ModelGateway: Send + Sync {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()>;
}

/// Run-thread transaction helper. Capability functions themselves never BEGIN/COMMIT.
/// Errors (including unwinding) roll back. Nested transactions are rejected by SQLite.
pub fn with_tx<T, E: From<rusqlite::Error>>(
    conn: &Connection,
    operation: impl FnOnce(&Connection) -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let transaction = conn.unchecked_transaction()?;
    let value = operation(&transaction)?;
    transaction.commit()?;
    Ok(value)
}

#[cfg(test)]
mod contracts_tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + fmt::Debug>(value: T) {
        let wire = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<T>(&wire).unwrap(), value);
    }

    #[test]
    fn contracts_trigger_round_trip() {
        for trigger in [
            Trigger::UserMessage {
                message_id: "message-1".into(),
                attempt: 2,
            },
            Trigger::Occurrence {
                occurrence_id: "occurrence-1".into(),
            },
            Trigger::Delegation {
                delegation_id: "delegation-1".into(),
            },
        ] {
            round_trip(trigger);
        }
        let wire = serde_json::to_value(Trigger::UserMessage {
            message_id: "m".into(),
            attempt: 3,
        })
        .unwrap();
        assert_eq!(
            wire,
            json!({"kind": "user_message", "message_id": "m", "attempt": 3})
        );
    }

    #[test]
    fn contracts_tool_outcome_round_trip() {
        for outcome in [
            ToolOutcome::Done {
                output: json!({"fact_id": "fact_1"}),
            },
            ToolOutcome::Failed {
                reason: "refused".into(),
            },
            ToolOutcome::Suspend {
                cause: SuspendCause::HumanInput {
                    id: "form-1".into(),
                },
            },
            ToolOutcome::Suspend {
                cause: SuspendCause::Children {
                    ids: vec!["child-1".into()],
                },
            },
        ] {
            round_trip(outcome);
        }
    }

    #[test]
    fn contracts_resume_receipt_round_trip() {
        for wake in [
            WakeStatus::Pending,
            WakeStatus::Delivered {
                run_id: "run-1".into(),
                first_event_seq: 4,
            },
            WakeStatus::Failed {
                reason: "cancelled".into(),
            },
        ] {
            round_trip(ResumeReceipt {
                resolution_id: "resolution-1".into(),
                wake,
            });
        }
    }

    #[test]
    fn contracts_approval_route_round_trip() {
        round_trip(ApprovalRoute {
            requesting_bot_id: "child-bot".into(),
            target_bot_id: "principal-bot".into(),
            target_conversation_id: "principal-conversation".into(),
        });
        assert!(serde_json::from_value::<ApprovalRoute>(json!({
            "requesting_bot_id": "child-bot"
        }))
        .is_err());
    }

    #[test]
    fn contracts_summary_round_trip() {
        for previous_summary_id in [None, Some("summary-0".into())] {
            round_trip(Summary {
                summary_id: "summary-1".into(),
                conversation_id: "conversation-1".into(),
                text: "Résumé avec outils et image 🖼".into(),
                truncation_last_entry_inclusive: "entry-7".into(),
                resume_from_entry_inclusive: "entry-8".into(),
                previous_summary_id,
                includes_tool_results: true,
                strategy: "rolling".into(),
            });
        }
    }

    #[test]
    fn contracts_model_request_preserves_tool_and_image_input() {
        round_trip(ModelRequest {
            model: "simulated".into(),
            instructions: "app contract".into(),
            input: vec![HistoryItem {
                entry_id: "entry-1".into(),
                role: Role::Assistant,
                text: String::new(),
                images: vec![ImageInput {
                    attachment_id: "image-1".into(),
                    url: Some("data:image/png;base64,AA==".into()),
                    caption: Some("image".into()),
                }],
                tool_items: vec![
                    ToolItem::Call {
                        call: ToolCall {
                            call_id: "call-1".into(),
                            name: "read_skill".into(),
                            arguments: json!({"name": "review"}),
                        },
                    },
                    ToolItem::Output {
                        call_id: "call-1".into(),
                        output: json!("skill body"),
                    },
                ],
            }],
            tools: vec![ToolSpec {
                name: "read_skill".into(),
                schema: json!({"type": "object"}),
                requires_approval: false,
            }],
            effort: None,
            service_tier: None,
        });
    }

    #[test]
    fn contracts_with_tx_commits_or_rolls_back() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE effects (id INTEGER PRIMARY KEY)")
            .unwrap();
        with_tx(&conn, |conn| {
            conn.execute("INSERT INTO effects VALUES (1)", [])
        })
        .unwrap();
        let failed: rusqlite::Result<()> = with_tx(&conn, |conn| {
            conn.execute("INSERT INTO effects VALUES (2)", [])?;
            Err(rusqlite::Error::InvalidQuery)
        });
        assert!(failed.is_err());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM effects", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let outer = conn.unchecked_transaction().unwrap();
        let nested: rusqlite::Result<()> = with_tx(&conn, |_| Ok(()));
        assert!(nested.is_err());
        assert!(!conn.is_autocommit());
        outer.rollback().unwrap();
    }
}
