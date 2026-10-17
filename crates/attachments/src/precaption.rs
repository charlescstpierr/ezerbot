//! Precaption projection follows `reports/server-context-residual.md` RC01:
//! the unchanged draft and last five pure human texts in chronological order,
//! with no additional length cutoff. Debounce/deadline are enforced by the host.
//! The persistent cache keyed by image/model/context is a reconstruction choice;
//! Grok's actual caption cache remains server-side and was not recovered.
use crate::staging::{digest, valid_digest};
use grokbot_engine::contracts::{Error, Result, Role};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEBOUNCE_MS: u64 = 1_000;
pub const CAPTION_DEADLINE_MS: u64 = 150_000;
pub const SKIPPED_BACKOFF_MS: u64 = 600_000;
pub const HUMAN_CONTEXT_MESSAGES: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionMessage {
    pub entry_id: String,
    pub role: Role,
    pub text: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub author: Option<Value>,
    #[serde(default, alias = "fromAgent")]
    pub from_agent: bool,
    #[serde(default)]
    pub branch: Option<String>,
    // Preserve all RC01 purity markers rather than silently treating an
    // interbot/structured entry as a user message after dropping its metadata.
    #[serde(default = "message_kind")]
    pub kind: String,
    #[serde(default, alias = "channelSender")]
    pub channel_sender: Option<Value>,
    #[serde(default, alias = "fromUser")]
    pub from_user: Option<Value>,
    #[serde(default)]
    pub suppressed: bool,
    #[serde(default)]
    pub branched: bool,
}
fn message_kind() -> String {
    "message".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrecaptionContext {
    pub draft: String,
    pub human_messages: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheKey {
    pub sha256: String,
    pub model: String,
    pub context_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CaptionStatus {
    Pending,
    Ready { text: String },
    Skipped { retry_after: i64 },
    Failed { reason: String },
}
impl CaptionStatus {
    /// UTC Unix milliseconds, matching the host clock. Saturating arithmetic
    /// prevents an overflow from turning a cooldown into an immediate retry.
    pub fn skipped(now_ms: i64) -> Self {
        Self::Skipped {
            retry_after: now_ms.saturating_add(SKIPPED_BACKOFF_MS as i64),
        }
    }

    pub fn retry_allowed(&self, now_ms: i64) -> bool {
        match self {
            Self::Skipped { retry_after } => now_ms >= *retry_after,
            Self::Failed { .. } => true,
            Self::Pending | Self::Ready { .. } => false,
        }
    }
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments_caption_cache (
            sha256 TEXT NOT NULL,
            model TEXT NOT NULL,
            context_digest TEXT NOT NULL,
            status TEXT NOT NULL,
            PRIMARY KEY(sha256, model, context_digest)
        );",
    )?;
    Ok(())
}

/// `messages` must be in transcript order (oldest first). Do not summarize,
/// trim, or otherwise rewrite the admitted texts or the draft (RC01).
pub fn project(draft: &str, messages: &[CaptionMessage]) -> Result<PrecaptionContext> {
    let mut human_messages: Vec<_> = messages
        .iter()
        .rev()
        .filter(|m| {
            m.kind == "message"
                && m.role == Role::User
                && !m.text.trim().is_empty()
                && m.channel.is_none()
                && m.channel_sender.is_none()
                && m.author.is_none()
                && m.from_user.is_none()
                && !m.from_agent
                && !m.suppressed
                && m.branch.is_none()
                && !m.branched
        })
        .take(HUMAN_CONTEXT_MESSAGES)
        .map(|m| m.text.clone())
        .collect();
    human_messages.reverse();
    Ok(PrecaptionContext {
        draft: draft.into(),
        human_messages,
    })
}

pub fn cache_key(sha256: &str, model: &str, context: &PrecaptionContext) -> Result<CacheKey> {
    let key = CacheKey {
        sha256: sha256.into(),
        model: model.into(),
        // A typed JSON object provides unambiguous boundaries, including empty
        // messages, newlines, and Unicode; concatenation would permit collisions.
        context_digest: digest(&serde_json::to_vec(context)?),
    };
    validate_key(&key)?;
    Ok(key)
}

fn validate_key(key: &CacheKey) -> Result<()> {
    if !valid_digest(&key.sha256)
        || !valid_digest(&key.context_digest)
        || key.model.trim().is_empty()
    {
        return Err(Error::InvalidInput("invalid precaption cache key".into()));
    }
    Ok(())
}

pub fn cached(conn: &Connection, key: &CacheKey) -> Result<Option<CaptionStatus>> {
    validate_key(key)?;
    let json: Option<String> = conn
        .query_row(
            "SELECT status FROM attachments_caption_cache
             WHERE sha256 = ?1 AND model = ?2 AND context_digest = ?3",
            params![key.sha256, key.model, key.context_digest],
            |row| row.get(0),
        )
        .optional()?;
    Ok(json.map(|s| serde_json::from_str(&s)).transpose()?)
}

pub fn save(conn: &Connection, key: &CacheKey, status: &CaptionStatus) -> Result<()> {
    validate_key(key)?;
    conn.execute(
        "INSERT INTO attachments_caption_cache(sha256, model, context_digest, status)
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(sha256, model, context_digest)
         DO UPDATE SET status = excluded.status",
        params![
            key.sha256,
            key.model,
            key.context_digest,
            serde_json::to_string(status)?
        ],
    )?;
    Ok(())
}

/// Atomically claim a caption job before starting background work. A repeated
/// request cannot bypass Pending/Ready or the ten-minute SKIPPED cooldown by
/// overwriting the row. Failed jobs can be retried. The host saves a final status
/// (including Failed on deadline/cancellation); Pending does not expire here.
/// No transaction is opened or committed by this function.
pub fn begin(conn: &Connection, key: &CacheKey, now_ms: i64) -> Result<bool> {
    validate_key(key)?;
    Ok(conn.execute(
        "INSERT INTO attachments_caption_cache(sha256, model, context_digest, status)
         VALUES (?1, ?2, ?3, ?4) ON CONFLICT(sha256, model, context_digest)
         DO UPDATE SET status = excluded.status
         WHERE json_extract(attachments_caption_cache.status, '$.status') = 'failed'
         OR (json_extract(attachments_caption_cache.status, '$.status') = 'skipped'
             AND json_extract(attachments_caption_cache.status, '$.retry_after') <= ?5)",
        params![
            key.sha256,
            key.model,
            key.context_digest,
            serde_json::to_string(&CaptionStatus::Pending)?,
            now_ms
        ],
    )? == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn human(index: usize) -> CaptionMessage {
        serde_json::from_value(json!({
            "entry_id": format!("entry-{index}"), "role": "user", "text": format!("human {index}")
        }))
        .unwrap()
    }
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }
    fn key() -> CacheKey {
        cache_key(
            &digest(b"image"),
            "vision-model",
            &project("draft", &[]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn projection_keeps_last_five_pure_human_messages() {
        let mut messages = Vec::new();
        for i in 0..7 {
            messages.push(human(i));
            let mut bot = human(i);
            bot.role = Role::Assistant;
            bot.text = "bot text must not leak".into();
            messages.push(bot);
        }
        let projection = project("  unchanged draft\n", &messages).unwrap();
        assert_eq!(projection.draft, "  unchanged draft\n");
        assert_eq!(
            projection.human_messages,
            (2..7).map(|i| format!("human {i}")).collect::<Vec<_>>()
        );
        assert_eq!(
            project("", &[]).unwrap().human_messages,
            Vec::<String>::new()
        );
        messages.push(human(8));
        messages.last_mut().unwrap().text = " x ".repeat(100_000);
        assert_eq!(
            project("", &messages)
                .unwrap()
                .human_messages
                .last()
                .unwrap(),
            &" x ".repeat(100_000)
        );
    }

    #[test]
    fn projection_excludes_structured_author_and_branch() {
        let mut messages = vec![human(0)];
        for marker in [
            json!({"author": {"type": "user", "id": "current-user"}}),
            json!({"branch": "thread"}),
            json!({"branched": true}),
            json!({"channel": "slack"}),
            json!({"channelSender": {"id": "sender"}}),
            json!({"fromUser": {"id": "user"}}),
            json!({"fromAgent": true}),
            json!({"suppressed": true}),
            json!({"kind": "event"}),
            json!({"role": "system"}),
            json!({"role": "tool"}),
            json!({"text": " \n\t"}),
        ] {
            let mut value = serde_json::to_value(human(1)).unwrap();
            // Remove default snake_case keys when testing the native aliases.
            value.as_object_mut().unwrap().remove("channel_sender");
            value.as_object_mut().unwrap().remove("from_user");
            value.as_object_mut().unwrap().remove("from_agent");
            value
                .as_object_mut()
                .unwrap()
                .extend(marker.as_object().unwrap().clone());
            messages.push(serde_json::from_value(value).unwrap());
        }
        assert_eq!(
            project("draft", &messages).unwrap().human_messages,
            vec!["human 0"]
        );
    }

    #[test]
    fn skipped_backs_off_ten_minutes() {
        let conn = connection();
        let key = key();
        let skipped = CaptionStatus::skipped(1_000);
        assert_eq!(
            skipped,
            CaptionStatus::Skipped {
                retry_after: 601_000
            }
        );
        save(&conn, &key, &skipped).unwrap();
        assert!(!cached(&conn, &key).unwrap().unwrap().retry_allowed(600_999));
        assert!(!begin(&conn, &key, 600_999).unwrap());
        assert_eq!(cached(&conn, &key).unwrap(), Some(skipped));
        assert!(cached(&conn, &key).unwrap().unwrap().retry_allowed(601_000));
        assert!(begin(&conn, &key, 601_000).unwrap());
        assert_eq!(cached(&conn, &key).unwrap(), Some(CaptionStatus::Pending));
        assert_eq!(
            CaptionStatus::skipped(i64::MAX),
            CaptionStatus::Skipped {
                retry_after: i64::MAX
            }
        );
    }

    #[test]
    fn cache_keyed_by_sha_model_and_context() {
        let conn = connection();
        let context = project("draft", &[human(0)]).unwrap();
        let first = cache_key(&digest(b"image"), "vision", &context).unwrap();
        let ready = CaptionStatus::Ready {
            text: "A caption".into(),
        };
        save(&conn, &first, &ready).unwrap();
        assert_eq!(cached(&conn, &first).unwrap(), Some(ready));
        for key in [
            cache_key(&digest(b"other image"), "vision", &context).unwrap(),
            cache_key(&digest(b"image"), "other model", &context).unwrap(),
            cache_key(
                &digest(b"image"),
                "vision",
                &project("other draft", &[human(0)]).unwrap(),
            )
            .unwrap(),
            cache_key(
                &digest(b"image"),
                "vision",
                &project("draft", &[human(1)]).unwrap(),
            )
            .unwrap(),
        ] {
            assert!(cached(&conn, &key).unwrap().is_none());
        }
        assert_eq!(
            cache_key(&digest(b"image"), "vision", &context).unwrap(),
            first
        );
        let a = PrecaptionContext {
            draft: "a".into(),
            human_messages: vec!["bc".into()],
        };
        let b = PrecaptionContext {
            draft: "ab".into(),
            human_messages: vec!["c".into()],
        };
        assert_ne!(
            cache_key(&digest(b"image"), "vision", &a).unwrap(),
            cache_key(&digest(b"image"), "vision", &b).unwrap()
        );
        assert!(cache_key("../bad", "vision", &context).is_err());
        assert!(cache_key(&digest(b"image"), " ", &context).is_err());
    }

    #[test]
    fn cache_claims_are_idempotent_and_results_round_trip() {
        let conn = connection();
        let key = key();
        assert!(begin(&conn, &key, 0).unwrap());
        assert!(!begin(&conn, &key, 1).unwrap());
        for status in [
            CaptionStatus::Ready {
                text: "caption\n\"Unicode: é\"".into(),
            },
            CaptionStatus::Failed {
                reason: "deadline exceeded".into(),
            },
        ] {
            save(&conn, &key, &status).unwrap();
            migrate(&conn).unwrap();
            assert_eq!(cached(&conn, &key).unwrap(), Some(status.clone()));
            assert_eq!(begin(&conn, &key, 2).unwrap(), status.retry_allowed(2));
        }
        let tx = conn.unchecked_transaction().unwrap();
        save(&tx, &key, &CaptionStatus::skipped(10)).unwrap();
        tx.rollback().unwrap();
        assert_eq!(cached(&conn, &key).unwrap(), Some(CaptionStatus::Pending));
    }
}
