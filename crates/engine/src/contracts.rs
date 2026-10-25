//! Shared wire contracts. IDs are opaque strings (the domain IDs are not serde DTOs).
//! Times and durations use UTC Unix milliseconds unless a field says otherwise.
//! Secret values must never be put in these model/transcript-facing types.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    NotImplemented(&'static str),
    InvalidInput(String),
    Conflict(String),
    NotFound(String),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotImplemented(module) => write!(f, "{module} is not implemented yet"),
            Self::InvalidInput(message) | Self::Conflict(message) | Self::NotFound(message) => {
                f.write_str(message)
            }
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
    Done {
        summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        captures: Vec<CaptureRef>,
    },
}

impl TaskStatus {
    /// Rewrites the free text of a finished task (summary and capture captions);
    /// capture ids and labels stay. True when something changed.
    pub fn map_text(&mut self, rewrite: impl Fn(&str) -> String) -> bool {
        let Self::Done { summary, captures } = self else {
            return false;
        };
        let mut changed = false;
        for text in std::iter::once(summary).chain(captures.iter_mut().map(|c| &mut c.caption)) {
            let rewritten = rewrite(text);
            changed |= rewritten != *text;
            *text = rewritten;
        }
        changed
    }
}

pub const MAX_CAPTURES: usize = 4;
pub const MAX_CAPTION_CHARACTERS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureLabel {
    #[serde(rename = "avant")]
    Avant,
    #[serde(rename = "après")]
    Apres,
}
impl CaptureLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Avant => "avant",
            Self::Apres => "après",
        }
    }
}

/// An image the task produced in its own thread, joined to its result by reference.
/// `CaptureRef::parse_list` is the only way in from a model: everything past it
/// (nonempty caption of at most `MAX_CAPTION_CHARACTERS`, at most `MAX_CAPTURES`
/// per result) holds by construction. Ownership by the thread is the host's check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureRef {
    pub id: String,
    pub label: CaptureLabel,
    pub caption: String,
}
impl CaptureRef {
    pub fn parse_list(raw: Option<&Value>) -> std::result::Result<Vec<Self>, String> {
        let list = match raw {
            None | Some(Value::Null) => return Ok(vec![]),
            Some(Value::Array(list)) => list,
            Some(_) => return Err("captures doit être une liste.".into()),
        };
        if list.len() > MAX_CAPTURES {
            return Err(format!(
                "Au plus {MAX_CAPTURES} captures peuvent être jointes."
            ));
        }
        list.iter()
            .enumerate()
            .map(|(index, item)| {
                let number = index + 1;
                let mut capture: Self = serde_json::from_value(item.clone()).map_err(|_| {
                    format!("La capture n° {number} doit avoir un identifiant (id), une étiquette (label : « avant » ou « après ») et une légende (caption).")
                })?;
                capture.caption = capture.caption.trim().into();
                if capture.id.trim().is_empty() {
                    return Err(format!("La capture n° {number} n’a pas d’identifiant."));
                }
                let length = capture.caption.chars().count();
                if length == 0 || length > MAX_CAPTION_CHARACTERS {
                    return Err(format!("La légende de la capture n° {number} doit faire de 1 à {MAX_CAPTION_CHARACTERS} caractères."));
                }
                Ok(capture)
            })
            .collect()
    }
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

/// Consult the approval gate before executing a protected tool, after validating
/// object arguments. Ask reuses/creates an idempotent principal-conversation request;
/// Denied never executes. Check the existing (run_id, call_id, canonical digest)
/// decision first: pending -> Ask; denied/expired/cancelled -> Denied, not another
/// request. Authorized applies only to this tool/argument digest. Consume AllowOnce
/// inside the same caller transaction as the check and internal effect/result,
/// or the durable admission record for a non-replayable external effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalCheck {
    Authorized { by: AuthorizedBy },
    Denied { reason: String },
    Ask,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorizedBy {
    FolderGrant {
        path: String,
    },
    AllowOnce {
        request_id: String,
    },
    Rule {
        rule_id: String,
    },
    /// Reserved for the future reviewer (REA PS06): `allow:<n>` identifies the
    /// zero-based Allow instruction in the config snapshot used for this review.
    /// Standing permissions use Rule, not an invented auto-review identifier.
    AutoReview {
        rule_id: String,
    },
    /// Mode YOLO: the owner switched every approval off, globally or for this
    /// bot. `source` is `global` or `bot`; it records which switch applied.
    /// `session` lie un geste d'écran à la session vivante à l'admission.
    Yolo {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schema: Value,
    pub requires_approval: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    /// The gateway preserves invalid/truncated/non-object JSON as Value::String(raw).
    /// Such strings carry the original provider argument text for faithful replay.
    /// Before check_approval or request_approval, the loop rejects non-object
    /// arguments with a per-call Failed outcome ("invalid arguments"), retaining
    /// the call ID and continuing the turn without asking the user to approve it.
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
    /// A durable follow-up to the same task, never a new run.
    InputReceived,
    /// Highest event sequence included in an outgoing model request.
    InputConsumed,
    /// A completed response superseded by a follow-up, without ending the task.
    ResponseCompleted,
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
    /// Fichiers de contexte déposés dans la bibliothèque du bot.
    Library,
    /// Faits retenus par Charles pour tous les fils d'un sujet : leur propre budget, jamais pris sur la mémoire du bot.
    TopicMemory,
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
    Call {
        call: ToolCall,
    },
    Output {
        call_id: String,
        output: Value,
    },
    /// Opaque provider state, replayed with function calls when store:false.
    /// Never render encrypted reasoning as assistant text.
    Reasoning {
        item_id: String,
        encrypted_content: String,
    },
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
    /// Attachment ids from the origin thread. The child receives the bytes or
    /// the local path and MIME type; a file name is not an image.
    #[serde(default)]
    pub selected_images: Vec<String>,
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
    /// Non-secret, stable conversation affinity; auxiliary requests may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

/// Provider-reported counts, not context estimates. Cached input is a subset of input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
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
        /// Same invalid-JSON representation as ToolCall::arguments.
        arguments: Value,
    },
    /// Retain as ToolItem::Reasoning for the next full-input model call.
    Reasoning {
        item_id: String,
        encrypted_content: String,
    },
    /// Optional provider usage, emitted before Completed or Incomplete.
    Usage {
        usage: TokenUsage,
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

/// Read a component's schema version (zero when absent), independently of
/// PRAGMA user_version. All components share this DDL; the caller owns the transaction.
pub fn component_version(conn: &Connection, component: &str) -> Result<u32> {
    ensure_component_versions(conn, component)?;
    Ok(conn
        .query_row(
            "SELECT version FROM component_versions WHERE component = ?1",
            [component],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

/// Update only the named component. Migration effects and this version write
/// belong in the same caller-owned transaction.
pub fn set_component_version(conn: &Connection, component: &str, version: u32) -> Result<()> {
    ensure_component_versions(conn, component)?;
    conn.execute(
        "INSERT INTO component_versions(component, version) VALUES (?1, ?2)
         ON CONFLICT(component) DO UPDATE SET version = excluded.version",
        rusqlite::params![component, version],
    )?;
    Ok(())
}

fn ensure_component_versions(conn: &Connection, component: &str) -> Result<()> {
    if component.trim().is_empty() {
        return Err(Error::InvalidInput(
            "component name must not be empty".into(),
        ));
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS component_versions (
            component TEXT PRIMARY KEY, version INTEGER NOT NULL
        )",
    )?;
    Ok(())
}

/// Run-thread transaction helper. Capability functions themselves never BEGIN/COMMIT.
/// BEGIN IMMEDIATE takes the write lock up front so busy_timeout applies, instead
/// of failing a deferred read-to-write upgrade with SQLITE_BUSY_SNAPSHOT in WAL.
/// Holds the database write lock for the entire closure: no network I/O, blocking
/// waits or pure reads here (read outside with_tx); not for read-only connections.
/// Errors (including unwinding) roll back. Nested transactions are rejected by SQLite.
pub fn with_tx<T, E: From<rusqlite::Error>>(
    conn: &Connection,
    operation: impl FnOnce(&Connection) -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let transaction =
        rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
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
                description: "Read the full body of an available skill".into(),
                schema: json!({"type": "object"}),
                requires_approval: false,
            }],
            effort: None,
            service_tier: None,
            prompt_cache_key: Some("conversation-test".into()),
        });
    }

    #[test]
    fn contracts_approval_check_wire_format() {
        for by in [
            AuthorizedBy::AllowOnce {
                request_id: "approval-1".into(),
            },
            AuthorizedBy::Rule {
                rule_id: "rule-1".into(),
            },
            AuthorizedBy::AutoReview {
                rule_id: "review-1".into(),
            },
            AuthorizedBy::Yolo {
                source: "global".into(),
                session: None,
            },
            AuthorizedBy::Yolo {
                source: "bot".into(),
                session: Some(7),
            },
        ] {
            round_trip(ApprovalCheck::Authorized { by });
        }
        round_trip(ApprovalCheck::Denied {
            reason: "blocked".into(),
        });
        assert_eq!(
            serde_json::to_value(ApprovalCheck::Ask).unwrap(),
            json!({"status": "ask"})
        );
        assert_eq!(
            serde_json::to_value(ApprovalCheck::Authorized {
                by: AuthorizedBy::Rule {
                    rule_id: "rule-1".into()
                }
            })
            .unwrap(),
            json!({"status": "authorized", "by": {"kind": "rule", "rule_id": "rule-1"}})
        );
        assert_eq!(
            serde_json::to_value(ApprovalCheck::Authorized {
                by: AuthorizedBy::Yolo {
                    source: "bot".into(),
                    session: None
                }
            })
            .unwrap(),
            json!({"status": "authorized", "by": {"kind": "yolo", "source": "bot"}})
        );
        assert_eq!(
            serde_json::to_value(AuthorizedBy::Yolo {
                source: "global".into(),
                session: Some(7)
            })
            .unwrap(),
            json!({"kind": "yolo", "source": "global", "session": 7})
        );
    }

    #[test]
    fn contracts_ui_wire_keys_are_stable() {
        assert_eq!(
            serde_json::to_value(ToolOutcome::Suspend {
                cause: SuspendCause::HumanInput {
                    id: "approval-1".into()
                }
            })
            .unwrap(),
            json!({"status": "suspend", "cause": {"kind": "human_input", "id": "approval-1"}})
        );
        assert_eq!(
            serde_json::to_value(ResumeReceipt {
                resolution_id: "resolution-1".into(),
                wake: WakeStatus::Pending
            })
            .unwrap(),
            json!({"resolution_id": "resolution-1", "wake": {"status": "pending"}})
        );
        assert_eq!(
            serde_json::to_value(ResumeReceipt {
                resolution_id: "resolution-1".into(),
                wake: WakeStatus::Delivered {
                    run_id: "run-1".into(),
                    first_event_seq: 4
                }
            })
            .unwrap(),
            json!({"resolution_id": "resolution-1", "wake": {
                "status": "delivered", "run_id": "run-1", "first_event_seq": 4
            }})
        );
    }

    #[test]
    fn contracts_reasoning_and_invalid_arguments_survive_round_trip() {
        round_trip(ToolItem::Reasoning {
            item_id: "reasoning-1".into(),
            encrypted_content: "opaque".into(),
        });
        round_trip(ModelEvent::Reasoning {
            item_id: "reasoning-1".into(),
            encrypted_content: "opaque".into(),
        });
        round_trip(ToolCall {
            call_id: "broken-call".into(),
            name: "read_skill".into(),
            arguments: json!("{truncated"),
        });
        let spec: ToolSpec = serde_json::from_value(json!({
            "name": "legacy", "schema": {}, "requires_approval": false
        }))
        .unwrap();
        assert_eq!(spec.description, "");
    }

    #[test]
    fn contracts_component_versions_are_independent_and_transactional() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
        assert_eq!(component_version(&conn, "forms_rules").unwrap(), 0);
        with_tx(&conn, |conn| -> Result<()> {
            set_component_version(conn, "forms_rules", 1)?;
            set_component_version(conn, "skills", 2)
        })
        .unwrap();
        let failed = with_tx(&conn, |conn| -> Result<()> {
            set_component_version(conn, "forms_rules", 3)?;
            Err(Error::Conflict("rollback".into()))
        });
        assert!(matches!(failed, Err(Error::Conflict(_))));
        assert_eq!(component_version(&conn, "forms_rules").unwrap(), 1);
        assert_eq!(component_version(&conn, "skills").unwrap(), 2);
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            7
        );
        assert!(component_version(&conn, " ").is_err());
        assert_eq!(Error::NotFound("missing".into()).to_string(), "missing");
    }

    struct WalDatabase(grokbot_test_support::TestDir);

    impl WalDatabase {
        fn new() -> Self {
            let db = Self(grokbot_test_support::TestDir::new("grokbot-contracts"));
            let conn = db.open();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.execute_batch(
                "CREATE TABLE counter (value INTEGER NOT NULL); INSERT INTO counter VALUES (0)",
            )
            .unwrap();
            db
        }

        fn open(&self) -> Connection {
            let conn = Connection::open(self.0.join("test.sqlite")).unwrap();
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            conn
        }
    }

    #[test]
    fn contracts_with_tx_wal_two_connections_serialize_read_then_write() {
        use std::sync::mpsc;
        use std::time::Duration;
        let db = WalDatabase::new();
        let a = db.open();
        let b = db.open();
        let (writer_ready, ready) = mpsc::channel();
        let (read, reader_started) = mpsc::channel();
        let (committed, writer_committed) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let tx =
                rusqlite::Transaction::new_unchecked(&b, rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
            tx.execute("UPDATE counter SET value = 1", []).unwrap();
            writer_ready.send(()).unwrap();
            // DEFERRED enters the closure while this writer holds the lock and
            // signals its stale read. IMMEDIATE waits outside the closure, so
            // the bounded wait expires and this writer commits first.
            let _ = reader_started.recv_timeout(Duration::from_millis(250));
            tx.commit().unwrap();
            committed.send(()).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let result: rusqlite::Result<()> = with_tx(&a, |conn| {
            let value: i64 = conn.query_row("SELECT value FROM counter", [], |row| row.get(0))?;
            let _ = read.send(());
            writer_committed
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            conn.execute("UPDATE counter SET value = ?1", [value + 1])?;
            Ok(())
        });
        writer.join().unwrap();
        result.unwrap();
        assert_eq!(
            a.query_row("SELECT value FROM counter", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn contracts_with_tx_wal_eight_threads_read_then_write() {
        let db = WalDatabase::new();
        let start = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let conn = db.open();
                let start = start.clone();
                std::thread::spawn(move || -> rusqlite::Result<()> {
                    start.wait();
                    for _ in 0..50 {
                        with_tx(&conn, |conn| -> rusqlite::Result<()> {
                            let value: i64 =
                                conn.query_row("SELECT value FROM counter", [], |row| row.get(0))?;
                            std::thread::yield_now();
                            conn.execute("UPDATE counter SET value = ?1", [value + 1])?;
                            Ok(())
                        })?;
                    }
                    Ok(())
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(
            db.open()
                .query_row("SELECT value FROM counter", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            400
        );
    }

    #[test]
    fn contracts_with_tx_rolls_back_after_unwind() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE effects (id INTEGER PRIMARY KEY)")
            .unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: rusqlite::Result<()> = with_tx(&conn, |conn| {
                conn.execute("INSERT INTO effects VALUES (1)", [])?;
                panic!("rollback on unwind");
            });
        }));
        assert!(panic.is_err());
        assert!(conn.is_autocommit());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM effects", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
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
