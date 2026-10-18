//! Natural-language policy review. REA PS05/PS06 recovers settings and cards,
//! not the remote evaluator. This local reconstruction is deliberately strict:
//! only an explicit, complete decision can grant; all other outcomes ask.
use crate::{approvals, requests, rules::AutoReviewConfig};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAnswer {
    pub decision: ReviewDecision,
    pub reason: String,
}
impl ReviewAnswer {
    pub fn parse(text: &str) -> Result<Self> {
        let answer: Self = serde_json::from_str(text)?;
        if answer.reason.trim().is_empty()
            || answer.reason.chars().count() > 240
            || answer.reason.chars().any(char::is_control)
        {
            return Err(Error::InvalidInput("invalid reviewer reason".into()));
        }
        Ok(answer)
    }
    pub fn ask(reason: &str) -> Self {
        Self {
            decision: ReviewDecision::Ask,
            reason: reason.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub version: String,
    pub rules: AutoReviewConfig,
    pub model: String,
    pub started_at: i64,
    pub pending: bool,
    pub answer: ReviewAnswer,
}

pub fn version(config: &AutoReviewConfig) -> Result<String> {
    approvals::args_digest("natural_language_rules_v1", &serde_json::to_value(config)?)
}
pub fn get(conn: &Connection, id: &str) -> Result<Option<ReviewRecord>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT review_json FROM forms_requests WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    text.map(|text| serde_json::from_str(&text).map_err(Into::into))
        .transpose()
}
/// A durable attempt is claimed before network I/O. A restart never retries a
/// potentially completed review: the still-pending human card remains usable.
pub fn claim(conn: &Connection, id: &str, record: &ReviewRecord) -> Result<bool> {
    requests::require_transaction(conn)?;
    Ok(conn.execute("UPDATE forms_requests SET review_json=?2 WHERE id=?1 AND status='pending' AND review_json IS NULL",
        params![id, serde_json::to_string(record)?])? == 1)
}
pub fn save(conn: &Connection, id: &str, record: &ReviewRecord) -> Result<()> {
    requests::require_transaction(conn)?;
    conn.execute(
        "UPDATE forms_requests SET review_json=?2 WHERE id=?1",
        params![id, serde_json::to_string(record)?],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_strict_complete_answers_are_accepted() {
        for decision in ["allow", "ask", "deny"] {
            assert!(ReviewAnswer::parse(&format!(
                r#"{{"decision":"{decision}","reason":"Règle explicite."}}"#
            ))
            .is_ok());
        }
        for text in [
            r#"{"decision":"allow"}"#,
            r#"{"decision":"allow","reason":""}"#,
            r#"{"decision":"maybe","reason":"Doute"}"#,
            r#"{"decision":"allow","reason":"Oui","confidence":0.5}"#,
            "```json\n{\"decision\":\"allow\",\"reason\":\"Oui\"}\n```",
            "{}",
        ] {
            assert!(ReviewAnswer::parse(text).is_err(), "{text}");
        }
    }
    #[test]
    fn text_version_changes_with_every_policy_change() {
        let mut rules = AutoReviewConfig::default();
        let first = version(&rules).unwrap();
        rules.allow.push("Autorise la lecture.".into());
        assert_ne!(version(&rules).unwrap(), first);
        let second = version(&rules).unwrap();
        rules.enabled = true;
        assert_ne!(version(&rules).unwrap(), second);
    }
}
