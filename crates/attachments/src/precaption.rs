//! Precaption projection follows `reports/server-context-residual.md` RC01:
//! the unchanged draft and last five pure human texts in chronological order,
//! with no additional length cutoff. Debounce and cancellation are host duties;
//! expired caption leases can be reclaimed here after the 150-second deadline.
//! RC01's SKIPPED cooldown is global, independent of image, model and draft.
//! Persisting that cooldown across restarts is a conservative reconstruction
//! choice; Grok's in-memory controller resets it when the app restarts.
//! Cache reuse is limited to conversations that requested the exact context key.
//! Captions remain bot-neutral: the host must not add bot identity or private
//! instructions to the vision prompt. Grok's cache itself was not recovered.
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
    // Yye checks presence, not truthiness: an explicit false/null is a marker.
    // Omit None on serialization so a pure message stays pure on a round trip.
    #[serde(
        default,
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub author: Option<Value>,
    #[serde(
        default,
        alias = "fromAgent",
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub from_agent: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    // Legacy adapter metadata, not a purity marker in Grok's Yye predicate.
    pub branch: Option<Value>,
    #[serde(default = "message_kind")]
    pub kind: String,
    #[serde(
        default,
        alias = "channelSender",
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel_sender: Option<Value>,
    #[serde(
        default,
        alias = "fromUser",
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub from_user: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub suppressed: Option<Value>,
    // Unlike the presence markers, RC01 excludes only the literal true here.
    #[serde(
        default,
        deserialize_with = "present_marker",
        skip_serializing_if = "Option::is_none"
    )]
    pub branched: Option<Value>,
}
fn present_marker<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
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
    Pending {
        #[serde(default)]
        started_at: i64,
    },
    Ready {
        text: String,
    },
    Skipped {
        retry_after: i64,
    },
    Failed {
        reason: String,
    },
}

/// A completion must carry this lease, not just the cache key. An opaque token
/// also prevents an old completion from matching a retry in the same millisecond.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionClaim {
    pub started_at: i64,
    pub token: String,
}

impl CaptionStatus {
    /// UTC Unix milliseconds, matching the host clock. Saturating arithmetic
    /// prevents an overflow from turning a cooldown into an immediate retry.
    pub fn skipped(now_ms: i64) -> Self {
        Self::Skipped {
            retry_after: now_ms.saturating_add(SKIPPED_BACKOFF_MS as i64),
        }
    }

    /// Key-local eligibility only. The host must also consult `backoff_until`,
    /// or call `begin`, which applies the global cooldown atomically.
    pub fn retry_allowed(&self, now_ms: i64) -> bool {
        match self {
            Self::Skipped { retry_after } => now_ms >= *retry_after,
            Self::Pending { started_at } => {
                now_ms >= started_at.saturating_add(CAPTION_DEADLINE_MS as i64)
            }
            Self::Failed { .. } => true,
            Self::Ready { .. } => false,
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
            updated_at INTEGER NOT NULL DEFAULT 0,
            claim_token TEXT,
            PRIMARY KEY(sha256, model, context_digest)
        );
        CREATE TABLE IF NOT EXISTS attachments_precaption_backoff (
            scope TEXT PRIMARY KEY,
            retry_after INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS attachments_caption_uses (
            conversation_id TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            model TEXT NOT NULL,
            context_digest TEXT NOT NULL,
            used_at INTEGER NOT NULL,
            PRIMARY KEY(conversation_id, sha256, model, context_digest)
        );",
    )?;
    let columns = conn
        .prepare("PRAGMA table_info(attachments_caption_cache)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, declaration) in [
        ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
        ("claim_token", "TEXT"),
    ] {
        if !columns.iter().any(|column| column == name) {
            conn.execute(
                &format!("ALTER TABLE attachments_caption_cache ADD COLUMN {name} {declaration}"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS attachments_caption_updated ON attachments_caption_cache(updated_at);
        DROP TRIGGER IF EXISTS attachments_caption_skipped_insert;
        DROP TRIGGER IF EXISTS attachments_caption_skipped_update;
        CREATE TRIGGER attachments_caption_skipped_insert
        AFTER INSERT ON attachments_caption_cache
        WHEN json_extract(NEW.status, '$.status') = 'skipped'
        BEGIN
            INSERT INTO attachments_precaption_backoff(scope, retry_after)
            VALUES ('global', json_extract(NEW.status, '$.retry_after'))
            ON CONFLICT(scope) DO UPDATE SET retry_after = MAX(retry_after, excluded.retry_after);
        END;
        CREATE TRIGGER attachments_caption_skipped_update
        AFTER UPDATE OF status ON attachments_caption_cache
        WHEN json_extract(NEW.status, '$.status') = 'skipped'
        BEGIN
            INSERT INTO attachments_precaption_backoff(scope, retry_after)
            VALUES ('global', json_extract(NEW.status, '$.retry_after'))
            ON CONFLICT(scope) DO UPDATE SET retry_after = MAX(retry_after, excluded.retry_after);
        END;
        INSERT INTO attachments_precaption_backoff(scope, retry_after)
        SELECT 'global', MAX(json_extract(status, '$.retry_after')) FROM attachments_caption_cache
        WHERE json_extract(status, '$.status') = 'skipped' HAVING COUNT(*) > 0
        ON CONFLICT(scope) DO UPDATE SET retry_after = MAX(retry_after, excluded.retry_after);",
    )?;
    // Trigger BEGIN/END above are SQLite trigger syntax, not transaction control.
    // They keep the result and its global cooldown atomic in the caller's write.
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
                && m.from_agent.is_none()
                && m.suppressed.is_none()
                && m.branched.as_ref() != Some(&Value::Bool(true))
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

/// Forced cache write for restoration/import and tests. Background workers must
/// use `begin` + `finish` so a late completion cannot overwrite a newer lease.
pub fn save(conn: &Connection, key: &CacheKey, status: &CaptionStatus) -> Result<()> {
    validate_key(key)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .as_millis();
    let now_ms = i64::try_from(now_ms).map_err(|_| Error::InvalidInput("clock overflow".into()))?;
    conn.execute(
        "INSERT INTO attachments_caption_cache(sha256, model, context_digest, status, updated_at)
         VALUES (?1, ?2, ?3, ?4, MAX(?5, COALESCE((SELECT MAX(updated_at) + 1 FROM attachments_caption_cache), ?5)))
         ON CONFLICT(sha256, model, context_digest)
         DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at, claim_token = NULL",
        params![key.sha256, key.model, key.context_digest, serde_json::to_string(status)?, now_ms],
    )?;
    Ok(())
}

/// Controller-wide RC01 cooldown, also available to the host when a new key has
/// no cached row but `begin` is refused. It is not cleared by changing the draft.
pub fn backoff_until(conn: &Connection) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT retry_after FROM attachments_precaption_backoff WHERE scope = 'global'",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

/// Record this conversation's use of the exact context key on every request,
/// including cache hits and blocked claims. Then atomically claim a job. Pending
/// leases expire at 150 seconds, including after an app crash. The global cooldown
/// check is part of the claim write; blocked new keys do not create cache rows.
/// The caller owns any transaction spanning the usage record and claim.
pub fn begin(
    conn: &Connection,
    conversation_id: &str,
    key: &CacheKey,
    now_ms: i64,
) -> Result<Option<CaptionClaim>> {
    if conversation_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "conversation ID is required for captioning".into(),
        ));
    }
    validate_key(key)?;
    conn.execute(
        "INSERT INTO attachments_caption_uses(conversation_id, sha256, model, context_digest, used_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(conversation_id, sha256, model, context_digest)
         DO UPDATE SET used_at = MAX(used_at, excluded.used_at)",
        params![conversation_id, key.sha256, key.model, key.context_digest, now_ms],
    )?;
    let mut nonce = [0; 16];
    getrandom::getrandom(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
    let claim = CaptionClaim {
        started_at: now_ms,
        token: digest(&nonce),
    };
    let token: Option<String> = conn.query_row(
        "INSERT INTO attachments_caption_cache(sha256, model, context_digest, status, updated_at, claim_token)
         SELECT ?1, ?2, ?3, ?4,
             MAX(?5, COALESCE((SELECT MAX(updated_at) + 1 FROM attachments_caption_cache), ?5)), ?6
         WHERE NOT EXISTS(SELECT 1 FROM attachments_precaption_backoff WHERE scope = 'global' AND retry_after > ?5)
         ON CONFLICT(sha256, model, context_digest)
         DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at, claim_token = excluded.claim_token
         WHERE json_extract(attachments_caption_cache.status, '$.status') = 'failed'
         OR (json_extract(attachments_caption_cache.status, '$.status') = 'skipped'
             AND json_extract(attachments_caption_cache.status, '$.retry_after') <= ?5)
         OR (json_extract(attachments_caption_cache.status, '$.status') = 'pending'
             AND COALESCE(json_extract(attachments_caption_cache.status, '$.started_at'), 0) <= ?7)
         RETURNING claim_token",
        params![key.sha256, key.model, key.context_digest,
            serde_json::to_string(&CaptionStatus::Pending { started_at: now_ms })?,
            now_ms, claim.token, now_ms.saturating_sub(CAPTION_DEADLINE_MS as i64)],
        |row| row.get(0),
    ).optional()?;
    Ok(token.map(|_| claim))
}

/// Accept a terminal result only for the still-current lease. A stale Skipped
/// result neither replaces the new job nor changes the global cooldown. Updated
/// times are monotone across the cache to order completions in the same ms.
pub fn finish(
    conn: &Connection,
    key: &CacheKey,
    claim: &CaptionClaim,
    status: &CaptionStatus,
    now_ms: i64,
) -> Result<bool> {
    validate_key(key)?;
    if matches!(status, CaptionStatus::Pending { .. }) {
        return Err(Error::InvalidInput(
            "a caption completion must be terminal".into(),
        ));
    }
    Ok(conn.execute(
        "UPDATE attachments_caption_cache SET status = ?4, claim_token = NULL,
             updated_at = MAX(?7, COALESCE((SELECT MAX(updated_at) + 1 FROM attachments_caption_cache), ?7))
         WHERE sha256 = ?1 AND model = ?2 AND context_digest = ?3 AND claim_token = ?5
             AND json_extract(status, '$.status') = 'pending'
             AND json_extract(status, '$.started_at') = ?6",
        params![key.sha256, key.model, key.context_digest, serde_json::to_string(status)?, claim.token, claim.started_at, now_ms],
    )? == 1)
}

/// Resolve a caption after the draft/history have changed, but only among the
/// exact context keys requested by this conversation. Another conversation's
/// private draft/messages must never change this conversation's caption.
pub fn latest_ready(
    conn: &Connection,
    conversation_id: &str,
    sha256: &str,
    model: &str,
) -> Result<Option<String>> {
    if conversation_id.trim().is_empty() || !valid_digest(sha256) || model.trim().is_empty() {
        return Err(Error::InvalidInput(
            "invalid conversation/image/model for caption lookup".into(),
        ));
    }
    Ok(conn
        .query_row(
            "SELECT json_extract(c.status, '$.text') FROM attachments_caption_uses u
         JOIN attachments_caption_cache c USING(sha256, model, context_digest)
         WHERE u.conversation_id = ?1 AND u.sha256 = ?2 AND u.model = ?3
             AND json_extract(c.status, '$.status') = 'ready'
         ORDER BY c.updated_at DESC LIMIT 1",
            params![conversation_id, sha256, model],
            |row| row.get(0),
        )
        .optional()?)
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
    fn ready(text: &str) -> CaptionStatus {
        CaptionStatus::Ready { text: text.into() }
    }
    // Existing per-key cases share one explicit conversation; isolation cases
    // below call the public APIs directly with distinct conversation IDs.
    fn begin(conn: &Connection, key: &CacheKey, now_ms: i64) -> Result<Option<CaptionClaim>> {
        super::begin(conn, "conversation", key, now_ms)
    }
    fn latest_ready(conn: &Connection, sha256: &str, model: &str) -> Result<Option<String>> {
        super::latest_ready(conn, "conversation", sha256, model)
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
        assert!(project("", &[]).unwrap().human_messages.is_empty());
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
        // A positive round-trip witness detects accidental exclusion of every
        // message by serde defaults rather than by the individual markers.
        messages.push(serde_json::from_value(serde_json::to_value(human(1)).unwrap()).unwrap());
        for marker in [
            json!({"author": {"type": "user", "id": "current-user"}}),
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
            let mut value = serde_json::to_value(human(2)).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .extend(marker.as_object().unwrap().clone());
            messages.push(serde_json::from_value(value).unwrap());
        }
        assert_eq!(
            project("draft", &messages).unwrap().human_messages,
            vec!["human 0", "human 1"]
        );
    }

    #[test]
    fn presence_markers_exclude_false_and_null_but_branched_only_excludes_true() {
        for field in [
            "channel",
            "author",
            "fromAgent",
            "from_agent",
            "channelSender",
            "fromUser",
            "suppressed",
        ] {
            for marker in [
                Value::Null,
                json!(false),
                json!("source"),
                json!({"id": "source"}),
            ] {
                let mut value = serde_json::to_value(human(0)).unwrap();
                value[field] = marker;
                let message: CaptionMessage = serde_json::from_value(value).unwrap();
                let round_trip =
                    serde_json::from_value(serde_json::to_value(&message).unwrap()).unwrap();
                assert!(
                    project("", &[message, round_trip])
                        .unwrap()
                        .human_messages
                        .is_empty(),
                    "{field}"
                );
            }
        }
        for (branched, admitted) in [
            (json!(true), false),
            (json!(false), true),
            (Value::Null, true),
        ] {
            let mut value = serde_json::to_value(human(0)).unwrap();
            value["branched"] = branched;
            let message = serde_json::from_value(value).unwrap();
            assert_eq!(
                !project("", &[message]).unwrap().human_messages.is_empty(),
                admitted
            );
        }
    }

    #[test]
    fn branch_metadata_is_not_a_yye_purity_marker() {
        for branch in [json!("main"), json!(false), Value::Null] {
            let mut value = serde_json::to_value(human(0)).unwrap();
            value["branch"] = branch;
            let message = serde_json::from_value(value).unwrap();
            assert_eq!(
                project("", &[message]).unwrap().human_messages,
                vec!["human 0"]
            );
        }
    }

    #[test]
    fn latest_ready_is_scoped_to_conversation() {
        let conn = connection();
        let image = digest(b"same image");
        let b = cache_key(
            &image,
            "vision",
            &project("what is this?", &[human(0)]).unwrap(),
        )
        .unwrap();
        let claim_b = super::begin(&conn, "conversation-b", &b, 100)
            .unwrap()
            .unwrap();
        assert!(finish(&conn, &b, &claim_b, &ready("B caption"), 100).unwrap());
        let a = cache_key(
            &image,
            "vision",
            &project("A private question", &[human(1)]).unwrap(),
        )
        .unwrap();
        let claim_a = super::begin(&conn, "conversation-a", &a, 200)
            .unwrap()
            .unwrap();
        assert!(finish(
            &conn,
            &a,
            &claim_a,
            &ready("caption from A's secret context"),
            200
        )
        .unwrap());
        assert_eq!(
            super::latest_ready(&conn, "conversation-b", &image, "vision").unwrap(),
            Some("B caption".into())
        );
        assert_eq!(
            super::latest_ready(&conn, "conversation-a", &image, "vision").unwrap(),
            Some("caption from A's secret context".into())
        );
        assert_eq!(
            super::latest_ready(&conn, "conversation-c", &image, "vision").unwrap(),
            None
        );
        assert!(super::begin(&conn, "", &a, 300).is_err());
        assert!(super::latest_ready(&conn, "", &image, "vision").is_err());
    }

    #[test]
    fn caption_usage_is_recorded_on_pending_ready_and_global_backoff_refusals() {
        let conn = connection();
        let key = key();
        let claim = super::begin(&conn, "first", &key, 100).unwrap().unwrap();
        assert!(super::begin(&conn, "pending-reader", &key, 101)
            .unwrap()
            .is_none());
        assert!(finish(
            &conn,
            &key,
            &claim,
            &ready("shared exact-context caption"),
            102
        )
        .unwrap());
        assert!(super::begin(&conn, "ready-reader", &key, 103)
            .unwrap()
            .is_none());
        for conversation in ["pending-reader", "ready-reader"] {
            assert_eq!(
                super::latest_ready(&conn, conversation, &key.sha256, &key.model).unwrap(),
                Some("shared exact-context caption".into())
            );
        }
        let blocked = cache_key(
            &digest(b"other image"),
            &key.model,
            &project("draft", &[]).unwrap(),
        )
        .unwrap();
        save(&conn, &key, &CaptionStatus::skipped(200)).unwrap();
        assert!(super::begin(&conn, "blocked-reader", &blocked, 300)
            .unwrap()
            .is_none());
        assert!(cached(&conn, &blocked).unwrap().is_none());
        assert!(super::begin(&conn, "blocked-reader", &blocked, 250)
            .unwrap()
            .is_none());
        let used_at: i64 = conn.query_row(
            "SELECT used_at FROM attachments_caption_uses WHERE conversation_id = 'blocked-reader'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(used_at, 300);
        let after_backoff = super::begin(&conn, "later-reader", &blocked, 600_200)
            .unwrap()
            .unwrap();
        assert!(finish(
            &conn,
            &blocked,
            &after_backoff,
            &ready("requested exact context"),
            600_201
        )
        .unwrap());
        assert_eq!(
            super::latest_ready(&conn, "blocked-reader", &blocked.sha256, &blocked.model).unwrap(),
            Some("requested exact context".into())
        );
    }

    #[test]
    fn pending_reclaimed_after_deadline() {
        let conn = connection();
        let key = key();
        let first = begin(&conn, &key, 0).unwrap().unwrap();
        assert_eq!(first.started_at, 0);
        let pending = cached(&conn, &key).unwrap().unwrap();
        assert!(!pending.retry_allowed(149_999));
        assert!(begin(&conn, &key, 149_999).unwrap().is_none());
        assert!(pending.retry_allowed(150_000));
        let second = begin(&conn, &key, 150_000).unwrap().unwrap();
        assert_eq!(second.started_at, 150_000);
        assert_ne!(first.token, second.token);
        assert!(!finish(
            &conn,
            &key,
            &first,
            &CaptionStatus::skipped(150_000),
            150_000
        )
        .unwrap());
        assert_eq!(backoff_until(&conn).unwrap(), None);
        assert_eq!(
            cached(&conn, &key).unwrap(),
            Some(CaptionStatus::Pending {
                started_at: 150_000
            })
        );
    }

    #[test]
    fn failed_image_does_not_pause_another_image_or_its_own_retry() {
        let conn = connection();
        let first = key();
        let claim = begin(&conn, &first, 1_000).unwrap().unwrap();
        assert!(finish(
            &conn,
            &first,
            &claim,
            &CaptionStatus::Failed {
                reason: "temporary failure".into(),
            },
            1_001,
        )
        .unwrap());
        assert_eq!(backoff_until(&conn).unwrap(), None);
        let other = cache_key(
            &digest(b"different image"),
            &first.model,
            &project("draft", &[]).unwrap(),
        )
        .unwrap();
        assert!(begin(&conn, &other, 1_002).unwrap().is_some());
        assert!(begin(&conn, &first, 1_002).unwrap().is_some());
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
        assert!(begin(&conn, &key, 600_999).unwrap().is_none());
        let new_keys = [
            cache_key(
                &digest(b"another image"),
                &key.model,
                &project("draft", &[]).unwrap(),
            )
            .unwrap(),
            cache_key(&key.sha256, &key.model, &project("new draft", &[]).unwrap()).unwrap(),
            cache_key(&key.sha256, "new model", &project("draft", &[]).unwrap()).unwrap(),
        ];
        for other in &new_keys {
            assert!(begin(&conn, other, 600_999).unwrap().is_none());
            assert!(cached(&conn, other).unwrap().is_none());
        }
        // A shorter backoff from a different result must never shorten the global one.
        save(&conn, &new_keys[0], &CaptionStatus::skipped(0)).unwrap();
        assert_eq!(backoff_until(&conn).unwrap(), Some(601_000));
        for other in &new_keys {
            assert!(begin(&conn, other, 600_999).unwrap().is_none());
            assert!(begin(&conn, other, 601_000).unwrap().is_some());
        }
        assert!(begin(&conn, &key, 601_000).unwrap().is_some());
        assert_eq!(
            CaptionStatus::skipped(i64::MAX),
            CaptionStatus::Skipped {
                retry_after: i64::MAX
            }
        );
    }

    #[test]
    fn late_completions_cannot_replace_a_new_lease_even_in_same_millisecond() {
        let conn = connection();
        let key = key();
        let first = begin(&conn, &key, 0).unwrap().unwrap();
        let failed = CaptionStatus::Failed {
            reason: "deadline".into(),
        };
        assert!(finish(&conn, &key, &first, &failed, 0).unwrap());
        let second = begin(&conn, &key, 0).unwrap().unwrap();
        assert_eq!(first.started_at, second.started_at);
        assert_ne!(first.token, second.token);
        assert!(!finish(&conn, &key, &first, &failed, 1).unwrap());
        assert!(begin(&conn, &key, 1).unwrap().is_none());
        assert!(finish(&conn, &key, &second, &ready("current caption"), 2).unwrap());
        assert!(!finish(&conn, &key, &first, &CaptionStatus::skipped(3), 3).unwrap());
        assert_eq!(backoff_until(&conn).unwrap(), None);
        assert_eq!(cached(&conn, &key).unwrap(), Some(ready("current caption")));
        assert!(finish(
            &conn,
            &key,
            &second,
            &CaptionStatus::Pending { started_at: 4 },
            4
        )
        .is_err());
    }

    #[test]
    fn latest_ready_survives_changed_drafts_and_orders_same_ms_completions() {
        let conn = connection();
        let first = key();
        let second = cache_key(
            &first.sha256,
            &first.model,
            &project("new draft", &[]).unwrap(),
        )
        .unwrap();
        assert_eq!(
            latest_ready(&conn, &first.sha256, &first.model).unwrap(),
            None
        );
        for (key, text) in [(&first, "first"), (&second, "second")] {
            let claim = begin(&conn, key, 100).unwrap().unwrap();
            assert!(finish(&conn, key, &claim, &ready(text), 100).unwrap());
        }
        assert_eq!(
            latest_ready(&conn, &first.sha256, &first.model).unwrap(),
            Some("second".into())
        );
        let other_model =
            cache_key(&first.sha256, "other", &project("draft", &[]).unwrap()).unwrap();
        let other_image = cache_key(
            &digest(b"another image"),
            &first.model,
            &project("draft", &[]).unwrap(),
        )
        .unwrap();
        for key in [other_model, other_image] {
            save(&conn, &key, &ready("unrelated")).unwrap();
        }
        save(
            &conn,
            &first,
            &CaptionStatus::Failed {
                reason: "failed".into(),
            },
        )
        .unwrap();
        assert_eq!(
            latest_ready(&conn, &first.sha256, &first.model).unwrap(),
            Some("second".into())
        );
        save(&conn, &first, &ready("third")).unwrap();
        assert_eq!(
            latest_ready(&conn, &first.sha256, &first.model).unwrap(),
            Some("third".into())
        );
    }

    #[test]
    fn cache_keyed_by_sha_model_and_context() {
        let conn = connection();
        let context = project("draft", &[human(0)]).unwrap();
        let first = cache_key(&digest(b"image"), "vision", &context).unwrap();
        save(&conn, &first, &ready("A caption")).unwrap();
        assert_eq!(cached(&conn, &first).unwrap(), Some(ready("A caption")));
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
        let claim = begin(&conn, &key, 0).unwrap().unwrap();
        assert!(begin(&conn, &key, 1).unwrap().is_none());
        let pending = CaptionStatus::Pending { started_at: 0 };
        let tx = conn.unchecked_transaction().unwrap();
        assert!(finish(&tx, &key, &claim, &CaptionStatus::skipped(10), 10).unwrap());
        assert_eq!(backoff_until(&tx).unwrap(), Some(600_010));
        tx.rollback().unwrap();
        assert_eq!(cached(&conn, &key).unwrap(), Some(pending));
        assert_eq!(backoff_until(&conn).unwrap(), None);
        for status in [
            ready("caption\n\"Unicode: é\""),
            CaptionStatus::Failed {
                reason: "deadline exceeded".into(),
            },
        ] {
            save(&conn, &key, &status).unwrap();
            migrate(&conn).unwrap();
            assert_eq!(cached(&conn, &key).unwrap(), Some(status.clone()));
            assert_eq!(
                begin(&conn, &key, 2).unwrap().is_some(),
                status.retry_allowed(2)
            );
        }
    }

    #[test]
    fn old_cache_migrates_without_losing_results_or_leaving_legacy_pending_stuck() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE attachments_caption_cache (
            sha256 TEXT, model TEXT, context_digest TEXT, status TEXT,
            PRIMARY KEY(sha256, model, context_digest));",
        )
        .unwrap();
        let key = key();
        conn.execute(
            "INSERT INTO attachments_caption_cache VALUES (?1, ?2, ?3, ?4)",
            params![
                key.sha256,
                key.model,
                key.context_digest,
                r#"{"status":"pending"}"#
            ],
        )
        .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            cached(&conn, &key).unwrap(),
            Some(CaptionStatus::Pending { started_at: 0 })
        );
        assert!(begin(&conn, &key, 150_000).unwrap().is_some());
        save(&conn, &key, &CaptionStatus::skipped(150_000)).unwrap();
        conn.execute("DELETE FROM attachments_precaption_backoff", [])
            .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(backoff_until(&conn).unwrap(), Some(750_000));
    }
}
