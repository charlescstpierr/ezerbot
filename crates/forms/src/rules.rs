//! Standing-permission lookup and persisted auto-review settings.
//!
//! `approvals` owns the sole permission registry, `forms_approval_rules`, including
//! save-before-resolve, Always fallback, listing and revocation (REA PS01/PS05).
//! This module only reads that registry, using its bot-pair isolation and canonical
//! argument digest. Global covers the tool; Subject binds the digest; Call never
//! persists. A matching Block wins (PS04). Revocation does not cancel grants or
//! effects already admitted (PS03).
//!
//! PS06 does not recover the natural-language reviewer. AutoReviewConfig is
//! interpreted by a separate model call, never by substring/glob matching.
//! Neither rule text nor identifiers may contain secret values. The caller owns
//! transactions; standing lookup, grant consumption and tool effects must share
//! the dispatch transaction, so revocation cannot race admission.

use crate::approvals::{self, RuleScope};
use grokbot_engine::contracts::{
    component_version, set_component_version, ApprovalRoute, Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAX_AUTO_REVIEW_ENTRIES: usize = 20;
pub const MAX_AUTO_REVIEW_ENTRY_CHARS: usize = 1000;
const SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoReviewConfig {
    pub enabled: bool,
    pub allow: Vec<String>,
    pub block: Vec<String>,
}

impl AutoReviewConfig {
    /// REA schema limits apply independently to Allow[] and Block[]. Length is
    /// measured in Unicode characters, not UTF-8 bytes.
    pub fn validate(&self) -> Result<()> {
        for entries in [&self.allow, &self.block] {
            if entries.len() > MAX_AUTO_REVIEW_ENTRIES {
                return Err(invalid("auto-review accepts at most 20 entries per list"));
            }
            for entry in entries {
                if entry.trim().is_empty() || entry.chars().count() > MAX_AUTO_REVIEW_ENTRY_CHARS {
                    return Err(invalid("auto-review entries require 1 to 1000 characters"));
                }
            }
        }
        Ok(())
    }
}

/// Settings and the request's reviewer audit column are owned here.
/// forms::migrate initializes requests and the standing-rule registry first.
pub fn migrate(conn: &Connection) -> Result<()> {
    let version = component_version(conn, "forms_rules")?;
    if version > SCHEMA_VERSION {
        return Err(invalid("unsupported forms rules schema version"));
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forms_auto_review_config (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            config TEXT NOT NULL
        );",
    )?;
    let has_review: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('forms_requests') WHERE name='review_json')",
        [],
        |r| r.get(0),
    )?;
    if !has_review {
        conn.execute_batch("ALTER TABLE forms_requests ADD COLUMN review_json TEXT")?;
    }
    set_component_version(conn, "forms_rules", SCHEMA_VERSION)
}

/// Read the same standing rules as approvals::auto_review. None means Ask;
/// Some((false, id)) means Denied. This neither resolves a request nor consumes
/// its single-use grant. check_approval must first honor existing per-call
/// decisions, so a prior Deny/Expired/Cancelled cannot become a new request.
///
/// Digest calculation is shared with approvals, never supplied by the caller.
/// Only requesting/target bot IDs scope permission: a new display conversation
/// preserves it, but another bot, child or routine cannot borrow it. route must
/// come from trusted host state, not model-supplied tool arguments.
///
/// rule_id is "permission:<sha256>" of the JSON tuple of the composite registry
/// key (requesting bot, target bot, tool, stored digest, scope). It excludes the
/// display conversation, current Global call digest and mutable Allow/Block flag.
/// Ties are deterministic; a Block precedes every Allow. Lookup is read-only and
/// uses the registry key's indexed bot/tool prefix, not a full-table scan.
pub fn evaluate(
    conn: &Connection,
    route: &ApprovalRoute,
    tool: &str,
    arguments: &Value,
) -> Result<Option<(bool, String)>> {
    if route.requesting_bot_id.trim().is_empty()
        || route.target_bot_id.trim().is_empty()
        || route.target_conversation_id.trim().is_empty()
    {
        return Err(invalid("approval route identifiers must not be empty"));
    }
    if !arguments.is_object() {
        return Err(invalid("invalid arguments"));
    }
    let digest = approvals::args_digest(tool, arguments)?;
    let matched: Option<(bool, String, String)> = conn
        .query_row(
            "SELECT allow, args_digest, scope FROM forms_approval_rules
             WHERE requesting_bot_id=?1 AND target_bot_id=?2 AND tool=?3
               AND (args_digest=?4 OR scope='\"global\"')
             ORDER BY allow ASC, scope, args_digest LIMIT 1",
            params![route.requesting_bot_id, route.target_bot_id, tool, digest],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((allow, stored_digest, scope)) = matched else {
        return Ok(None);
    };
    let scope: RuleScope = serde_json::from_str(&scope)?;
    if scope == RuleScope::Call {
        return Err(invalid("call permission cannot be a standing rule"));
    }
    let key = serde_json::to_vec(&(
        &route.requesting_bot_id,
        &route.target_bot_id,
        tool,
        stored_digest,
        scope,
    ))?;
    let rule_id = format!("permission:{:x}", Sha256::digest(key));
    Ok(Some((allow, rule_id)))
}

pub fn save_auto_review_config(conn: &Connection, config: &AutoReviewConfig) -> Result<()> {
    config.validate()?;
    conn.execute(
        "INSERT INTO forms_auto_review_config(singleton, config) VALUES (1, ?1)
         ON CONFLICT(singleton) DO UPDATE SET config = excluded.config",
        [serde_json::to_string(config)?],
    )?;
    Ok(())
}

/// Disabled by default. Invalid stored settings fail closed rather than silently
/// dropping Block entries or enabling a partially decoded configuration.
pub fn load_auto_review_config(conn: &Connection) -> Result<AutoReviewConfig> {
    let config: Option<String> = conn
        .query_row(
            "SELECT config FROM forms_auto_review_config WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let config: AutoReviewConfig = match config {
        Some(config) => serde_json::from_str(&config)?,
        None => AutoReviewConfig::default(),
    };
    config.validate()?;
    Ok(config)
}

fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::ApprovalDecision;
    use crate::requests::{self, HumanInputRequest, InputKind, RequestStatus};
    use grokbot_engine::contracts::with_tx;
    use serde_json::json;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        with_tx(&conn, super::super::migrate).unwrap();
        conn
    }

    fn route() -> ApprovalRoute {
        ApprovalRoute {
            requesting_bot_id: "child-bot".into(),
            target_bot_id: "main-bot".into(),
            target_conversation_id: "main-conversation".into(),
        }
    }

    fn decide(
        conn: &Connection,
        id: &str,
        route: &ApprovalRoute,
        tool: &str,
        arguments: &Value,
        decision: ApprovalDecision,
    ) {
        with_tx(conn, |conn| -> Result<()> {
            requests::create(
                conn,
                &HumanInputRequest {
                    id: id.into(),
                    run_id: format!("run-{id}"),
                    entry_id: format!("entry-{id}"),
                    route: route.clone(),
                    kind: InputKind::Approval {
                        tool: tool.into(),
                        args_digest: approvals::args_digest(tool, arguments)?,
                        expires_at: 100,
                    },
                    status: RequestStatus::Pending,
                },
            )?;
            approvals::decide(
                conn,
                id,
                &route.target_conversation_id,
                &format!("submit-{id}"),
                decision,
                1,
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn rules_use_core_always_and_never_registry_for_global_arguments() {
        let conn = connection();
        let route = route();
        let a = json!({"path": "a", "value": 1});
        let b = json!({"path": "b", "value": 2});
        assert_eq!(evaluate(&conn, &route, "write", &a).unwrap(), None);
        decide(
            &conn,
            "allow",
            &route,
            "write",
            &a,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Global,
            },
        );
        let first = evaluate(&conn, &route, "write", &a).unwrap().unwrap();
        assert!(first.0);
        assert_eq!(
            evaluate(&conn, &route, "write", &b).unwrap(),
            Some(first.clone())
        );
        assert_eq!(approvals::list_rules(&conn).unwrap().len(), 1);
        decide(
            &conn,
            "never",
            &route,
            "write",
            &b,
            ApprovalDecision::Never {
                scope: RuleScope::Global,
            },
        );
        let denied = evaluate(&conn, &route, "write", &a).unwrap().unwrap();
        assert!(!denied.0);
        assert_eq!(denied.1, first.1); // Same composite key, changed effect.
        assert_eq!(approvals::list_rules(&conn).unwrap().len(), 1);
        assert_eq!(evaluate(&conn, &route, "write", &b).unwrap(), Some(denied));
        assert!(!conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name='forms_permission_rules')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap());
    }

    #[test]
    fn rules_are_bot_pair_bound_not_display_conversation_bound() {
        let conn = connection();
        let route = route();
        let args = json!({"path": "a"});
        decide(
            &conn,
            "allow",
            &route,
            "write",
            &args,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Global,
            },
        );
        let expected = evaluate(&conn, &route, "write", &args).unwrap();
        assert!(expected.as_ref().unwrap().0);
        let mut other = route.clone();
        other.requesting_bot_id = "untrusted-routine-or-other-child".into();
        assert_eq!(evaluate(&conn, &other, "write", &args).unwrap(), None);
        other = route.clone();
        other.target_bot_id = "other-principal".into();
        assert_eq!(evaluate(&conn, &other, "write", &args).unwrap(), None);
        other = route.clone();
        other.target_conversation_id = "new-main-thread".into();
        assert_eq!(evaluate(&conn, &other, "write", &args).unwrap(), expected);
        assert_eq!(evaluate(&conn, &route, "other-tool", &args).unwrap(), None);
    }

    #[test]
    fn rules_subject_lookup_uses_core_canonical_argument_digest() {
        let conn = connection();
        let route = route();
        let args = json!({"z": {"c": 1, "b": 2}, "a": [1, 2]});
        decide(
            &conn,
            "subject",
            &route,
            "write",
            &args,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Subject,
            },
        );
        let first = evaluate(&conn, &route, "write", &args).unwrap();
        assert!(first.as_ref().unwrap().0);
        assert_eq!(
            evaluate(
                &conn,
                &route,
                "write",
                &json!({"a": [1, 2], "z": {"b": 2, "c": 1}})
            )
            .unwrap(),
            first
        );
        assert_eq!(
            evaluate(
                &conn,
                &route,
                "write",
                &json!({"a": [2, 1], "z": {"b": 2, "c": 1}})
            )
            .unwrap(),
            None
        );
        let other_args = json!({"different": true});
        decide(
            &conn,
            "subject-2",
            &route,
            "write",
            &other_args,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Subject,
            },
        );
        assert_ne!(
            evaluate(&conn, &route, "write", &other_args)
                .unwrap()
                .unwrap()
                .1,
            first.unwrap().1
        );
    }

    #[test]
    fn rules_block_wins_across_scopes_and_insertion_orders() {
        for global_block in [true, false] {
            for block_first in [true, false] {
                let conn = connection();
                let route = route();
                let args = json!({"path": "a"});
                let allow = ApprovalDecision::AlwaysAllow {
                    scope: if global_block {
                        RuleScope::Subject
                    } else {
                        RuleScope::Global
                    },
                };
                let block = ApprovalDecision::Never {
                    scope: if global_block {
                        RuleScope::Global
                    } else {
                        RuleScope::Subject
                    },
                };
                for (id, decision) in if block_first {
                    [("block", block), ("allow", allow)]
                } else {
                    [("allow", allow), ("block", block)]
                } {
                    decide(&conn, id, &route, "write", &args, decision);
                }
                assert!(!evaluate(&conn, &route, "write", &args).unwrap().unwrap().0);
                assert_eq!(
                    evaluate(&conn, &route, "write", &json!({"path": "b"}))
                        .unwrap()
                        .unwrap()
                        .0,
                    !global_block
                );
            }
        }
    }

    #[test]
    fn rules_call_always_and_allow_once_never_become_standing_permissions() {
        for decision in [
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Call,
            },
            ApprovalDecision::AllowOnce,
        ] {
            let conn = connection();
            let route = route();
            let args = json!({"path": "a"});
            decide(&conn, "once", &route, "write", &args, decision);
            assert!(approvals::list_rules(&conn).unwrap().is_empty());
            assert_eq!(evaluate(&conn, &route, "write", &args).unwrap(), None);
            with_tx(&conn, |conn| {
                approvals::consume_allow_once(conn, "once", "run-once", "write", &args, 2)
            })
            .unwrap();
            assert!(with_tx(&conn, |conn| approvals::consume_allow_once(
                conn, "once", "run-once", "write", &args, 3
            ))
            .is_err());
        }
    }

    #[test]
    fn rules_revocation_uses_core_registry_and_lookup_is_read_only() {
        let conn = connection();
        let route = route();
        let args = json!({"path": "a"});
        decide(
            &conn,
            "allow",
            &route,
            "write",
            &args,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Subject,
            },
        );
        // An unrelated malformed rule must not poison every tool's lookup.
        conn.execute(
            "INSERT INTO forms_approval_rules VALUES (?1, ?2, ?3, 'unrelated-tool', '', 'invalid-json-scope', 1)",
            params![route.requesting_bot_id, route.target_bot_id, route.target_conversation_id],
        ).unwrap();
        conn.pragma_update(None, "query_only", true).unwrap();
        assert!(evaluate(&conn, &route, "write", &args).unwrap().unwrap().0);
        conn.pragma_update(None, "query_only", false).unwrap();
        conn.execute(
            "DELETE FROM forms_approval_rules WHERE tool='unrelated-tool'",
            [],
        )
        .unwrap();
        let rule = approvals::list_rules(&conn).unwrap().pop().unwrap();
        with_tx(&conn, |conn| approvals::revoke_rule(conn, &rule)).unwrap();
        assert_eq!(evaluate(&conn, &route, "write", &args).unwrap(), None);
        // Revocation changes future reviews, not an already admitted grant.
        with_tx(&conn, |conn| {
            approvals::consume_allow_once(conn, "allow", "run-allow", "write", &args, 2)
        })
        .unwrap();
    }

    #[test]
    fn rules_lookup_rejects_invalid_arguments_and_treats_identifiers_literally() {
        let conn = connection();
        let mut route = route();
        route.requesting_bot_id = "child' OR 1=1 --".into();
        let tool = "tool'; DROP TABLE forms_approval_rules; --";
        let args = json!({"path": "a"});
        decide(
            &conn,
            "allow",
            &route,
            tool,
            &args,
            ApprovalDecision::AlwaysAllow {
                scope: RuleScope::Global,
            },
        );
        assert!(evaluate(&conn, &route, tool, &args).unwrap().unwrap().0);
        assert_eq!(evaluate(&conn, &self::route(), tool, &args).unwrap(), None);
        assert_eq!(evaluate(&conn, &route, "tool", &args).unwrap(), None);
        assert!(evaluate(&conn, &route, tool, &json!("{truncated")).is_err());
        assert!(evaluate(&conn, &route, tool, &json!([])).is_err());
        assert!(evaluate(&conn, &route, " ", &args).is_err());
        route.target_bot_id.clear();
        assert!(evaluate(&conn, &route, tool, &args).is_err());
    }

    #[test]
    fn rules_auto_review_limits_unicode_and_disabled_default() {
        let conn = connection();
        assert_eq!(
            load_auto_review_config(&conn).unwrap(),
            AutoReviewConfig::default()
        );
        let valid = AutoReviewConfig {
            enabled: true,
            allow: vec!["é".repeat(1000); 20],
            block: vec!["禁".repeat(1000); 20],
        };
        save_auto_review_config(&conn, &valid).unwrap();
        assert_eq!(load_auto_review_config(&conn).unwrap(), valid);
        for bad in [
            AutoReviewConfig {
                allow: vec!["x".into(); 21],
                ..valid.clone()
            },
            AutoReviewConfig {
                block: vec!["x".into(); 21],
                ..valid.clone()
            },
            AutoReviewConfig {
                allow: vec!["é".repeat(1001)],
                ..valid.clone()
            },
            AutoReviewConfig {
                block: vec!["禁".repeat(1001)],
                ..valid.clone()
            },
            AutoReviewConfig {
                allow: vec![" \n".into()],
                ..valid.clone()
            },
        ] {
            assert!(save_auto_review_config(&conn, &bad).is_err());
            assert_eq!(load_auto_review_config(&conn).unwrap(), valid);
        }
        // Natural-language instructions cannot accidentally act as tool grants.
        assert_eq!(
            evaluate(&conn, &route(), "write", &json!({})).unwrap(),
            None
        );
    }

    #[test]
    fn rules_auto_review_rollback_and_invalid_stored_config_fail_closed() {
        let conn = connection();
        let failed: Result<()> = with_tx(&conn, |conn| {
            save_auto_review_config(
                conn,
                &AutoReviewConfig {
                    enabled: true,
                    allow: vec!["reviewed actions".into()],
                    block: vec![],
                },
            )?;
            Err(invalid("rollback"))
        });
        assert!(failed.is_err());
        assert_eq!(
            load_auto_review_config(&conn).unwrap(),
            AutoReviewConfig::default()
        );
        conn.execute(
            "INSERT INTO forms_auto_review_config VALUES (1, ?1)",
            [json!({"enabled": true, "allow": ["x".repeat(1001)], "block": []}).to_string()],
        )
        .unwrap();
        assert!(load_auto_review_config(&conn).is_err());
    }

    #[test]
    fn rules_migration_is_versioned_without_user_version_or_downgrade() {
        let conn = connection();
        conn.pragma_update(None, "user_version", 17).unwrap();
        set_component_version(&conn, "other", 9).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            component_version(&conn, "forms_rules").unwrap(),
            SCHEMA_VERSION
        );
        assert_eq!(component_version(&conn, "other").unwrap(), 9);
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            17
        );
        set_component_version(&conn, "forms_rules", SCHEMA_VERSION + 1).unwrap();
        assert!(migrate(&conn).is_err());
        assert_eq!(
            component_version(&conn, "forms_rules").unwrap(),
            SCHEMA_VERSION + 1
        );
    }

    #[test]
    fn rules_core_permission_and_auto_review_config_survive_reopen() {
        let mut bytes = [0; 8];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = std::env::temp_dir().join(format!(
            "grokbot-rules-{}-{:x}.sqlite",
            std::process::id(),
            u64::from_ne_bytes(bytes)
        ));
        let route = route();
        let args = json!({"path": "a"});
        let config = AutoReviewConfig {
            enabled: true,
            allow: vec!["allow reviewed actions".into()],
            block: vec![],
        };
        let rule_id;
        {
            let conn = Connection::open(&path).unwrap();
            with_tx(&conn, super::super::migrate).unwrap();
            decide(
                &conn,
                "allow",
                &route,
                "write",
                &args,
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Subject,
                },
            );
            with_tx(&conn, |conn| save_auto_review_config(conn, &config)).unwrap();
            rule_id = evaluate(&conn, &route, "write", &args).unwrap().unwrap().1;
        }
        {
            let conn = Connection::open(&path).unwrap();
            with_tx(&conn, super::super::migrate).unwrap();
            assert_eq!(load_auto_review_config(&conn).unwrap(), config);
            assert_eq!(
                evaluate(&conn, &route, "write", &args).unwrap(),
                Some((true, rule_id))
            );
            let rule = approvals::list_rules(&conn).unwrap().pop().unwrap();
            with_tx(&conn, |conn| approvals::revoke_rule(conn, &rule)).unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(evaluate(&conn, &route, "write", &args).unwrap(), None);
        }
        std::fs::remove_file(path).unwrap();
    }
}
