//! Durable permission grants and auto-review settings.
//!
//! REA `goal-permissions-secrets.md` PS01/PS05: save Always before resolving;
//! a failed save must fall back to AllowOnce in the approval owner. PS04 orders
//! never < ask < always. This reconstruction gives any matching Block precedence.
//! PS01 Global grants cover the exact tool across arguments. Subject/Call bind
//! the digest because no subject extractor exists. Standing rules do not replace
//! the per-request digest-bound, single-use grant required at dispatch. The host
//! authenticates principal-conversation consent and owns bot/workspace isolation.
//! PS06 does not recover the remote natural-language reviewer. Instructions below
//! are configuration for that reviewer, never substring/glob authorization rules.
//! All mutations join the caller's transaction; removal does not cancel inflight
//! effects (PS03). Neither rule text nor identifiers may contain secret values.

use grokbot_engine::contracts::{component_version, set_component_version, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

pub const MAX_AUTO_REVIEW_ENTRIES: usize = 20;
pub const MAX_AUTO_REVIEW_ENTRY_CHARS: usize = 1000;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Call,
    Subject,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub rule_id: String,
    pub tool: String,
    pub scope: RuleScope,
    /// Required for Call/Subject; Global requires None and covers this exact
    /// tool across arguments (PS01). A resulting execution grant must still bind
    /// the current request's digest, not the action that created the rule.
    pub args_digest: Option<String>,
    pub effect: Effect,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

pub fn migrate(conn: &Connection) -> Result<()> {
    let version = component_version(conn, "forms_rules")?;
    if version > SCHEMA_VERSION {
        return Err(invalid("unsupported forms rules schema version"));
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forms_permission_rules (
            rule_id TEXT PRIMARY KEY NOT NULL,
            tool TEXT NOT NULL,
            scope TEXT NOT NULL CHECK(scope IN ('call', 'subject', 'global')),
            args_digest TEXT,
            effect TEXT NOT NULL CHECK(effect IN ('allow', 'block')),
            created_at INTEGER NOT NULL,
            CHECK(length(trim(rule_id)) > 0 AND length(trim(tool)) > 0),
            CHECK(args_digest IS NULL OR length(trim(args_digest)) > 0),
            CHECK((scope = 'global' AND args_digest IS NULL)
               OR (scope != 'global' AND args_digest IS NOT NULL))
        );
        CREATE INDEX IF NOT EXISTS forms_permission_rules_tool ON forms_permission_rules(tool);
        CREATE TABLE IF NOT EXISTS forms_auto_review_config (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            config TEXT NOT NULL
        );",
    )?;
    set_component_version(conn, "forms_rules", SCHEMA_VERSION)
}

/// Save an explicit user-approved rule before resolving Always. Failure must not
/// result in a standing grant. The approval owner handles the AllowOnce fallback.
pub fn save_rule(conn: &Connection, rule: &PermissionRule) -> Result<()> {
    validate_rule(rule)?;
    let scope = match rule.scope {
        RuleScope::Call => "call",
        RuleScope::Subject => "subject",
        RuleScope::Global => "global",
    };
    let effect = match rule.effect {
        Effect::Allow => "allow",
        Effect::Block => "block",
    };
    conn.execute(
        "INSERT INTO forms_permission_rules(rule_id, tool, scope, args_digest, effect, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(rule_id) DO UPDATE SET tool = excluded.tool, scope = excluded.scope,
             args_digest = excluded.args_digest, effect = excluded.effect, created_at = excluded.created_at",
        params![rule.rule_id, rule.tool, scope, rule.args_digest, effect, rule.created_at],
    )?;
    Ok(())
}

pub fn list_rules(conn: &Connection) -> Result<Vec<PermissionRule>> {
    let mut statement = conn.prepare(
        "SELECT rule_id, tool, scope, args_digest, effect, created_at
         FROM forms_permission_rules ORDER BY created_at, rule_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    rows.map(|row| {
        let (rule_id, tool, scope, args_digest, effect, created_at) = row?;
        let rule = PermissionRule {
            rule_id,
            tool,
            args_digest,
            created_at,
            scope: match scope.as_str() {
                "call" => RuleScope::Call,
                "subject" => RuleScope::Subject,
                "global" => RuleScope::Global,
                _ => return Err(invalid("invalid stored rule scope")),
            },
            effect: match effect.as_str() {
                "allow" => Effect::Allow,
                "block" => Effect::Block,
                _ => return Err(invalid("invalid stored rule effect")),
            },
        };
        validate_rule(&rule)?;
        Ok(rule)
    })
    .collect()
}

/// Idempotent revocation. The bool reports whether a row was removed.
pub fn remove_rule(conn: &Connection, rule_id: &str) -> Result<bool> {
    nonblank(rule_id, "rule ID must not be empty")?;
    Ok(conn.execute(
        "DELETE FROM forms_permission_rules WHERE rule_id = ?1",
        [rule_id],
    )? != 0)
}

/// None means Ask, never implicit authorization. This only evaluates durable,
/// explicit grants; natural-language auto-review requires a separate reviewer.
pub fn evaluate(conn: &Connection, tool: &str, args_digest: &str) -> Result<Option<Effect>> {
    Ok(evaluate_rule(conn, tool, args_digest)?.map(|rule| rule.effect))
}

/// Same decision with provenance for AuthorizedBy::Rule. When several rules
/// match, Block wins, then the oldest rule/ID for deterministic auditing.
pub fn evaluate_rule(
    conn: &Connection,
    tool: &str,
    args_digest: &str,
) -> Result<Option<PermissionRule>> {
    nonblank(tool, "tool must not be empty")?;
    nonblank(args_digest, "argument digest must not be empty")?;
    let mut allowed = None;
    for rule in list_rules(conn)? {
        if rule.tool != tool
            || (rule.scope != RuleScope::Global && rule.args_digest.as_deref() != Some(args_digest))
        {
            continue;
        }
        match rule.effect {
            Effect::Block => return Ok(Some(rule)),
            Effect::Allow if allowed.is_none() => allowed = Some(rule),
            Effect::Allow => {}
        }
    }
    Ok(allowed)
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

fn nonblank(value: &str, message: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(invalid(message))
    } else {
        Ok(())
    }
}

fn validate_rule(rule: &PermissionRule) -> Result<()> {
    nonblank(&rule.rule_id, "rule ID must not be empty")?;
    nonblank(&rule.tool, "tool must not be empty")?;
    match (rule.scope, &rule.args_digest) {
        (RuleScope::Global, None) => {}
        (RuleScope::Global, Some(_)) => {
            return Err(invalid("global rules must omit the argument digest"));
        }
        (_, Some(digest)) => nonblank(digest, "argument digest must not be empty")?,
        (_, None) => return Err(invalid("call and subject rules require an argument digest")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_engine::contracts::with_tx;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        with_tx(&conn, super::super::migrate).unwrap();
        conn
    }

    fn rule(id: &str, effect: Effect, scope: RuleScope, digest: Option<&str>) -> PermissionRule {
        PermissionRule {
            rule_id: id.into(),
            tool: "delete_skill".into(),
            scope,
            args_digest: digest.map(str::to_owned),
            effect,
            created_at: 123,
        }
    }

    #[test]
    fn rules_call_and_subject_allow_are_bound_to_tool_and_digest() {
        for scope in [RuleScope::Call, RuleScope::Subject] {
            let conn = connection();
            let grant = rule("allow", Effect::Allow, scope, Some("digest-a"));
            save_rule(&conn, &grant).unwrap();
            assert_eq!(
                evaluate(&conn, "delete_skill", "digest-a").unwrap(),
                Some(Effect::Allow)
            );
            assert_eq!(
                evaluate_rule(&conn, "delete_skill", "digest-a").unwrap(),
                Some(grant)
            );
            assert_eq!(evaluate(&conn, "delete_skill", "digest-b").unwrap(), None);
            assert_eq!(evaluate(&conn, "other_tool", "digest-a").unwrap(), None);
            assert!(save_rule(&conn, &rule("unbound", Effect::Allow, scope, None)).is_err());
        }
    }

    #[test]
    fn rules_global_allow_covers_new_arguments_but_not_other_tools() {
        let conn = connection();
        let grant = rule("global", Effect::Allow, RuleScope::Global, None);
        save_rule(&conn, &grant).unwrap();
        for digest in ["digest-a", "digest-b"] {
            assert_eq!(
                evaluate(&conn, "delete_skill", digest).unwrap(),
                Some(Effect::Allow)
            );
            assert_eq!(
                evaluate_rule(&conn, "delete_skill", digest).unwrap(),
                Some(grant.clone())
            );
        }
        assert_eq!(evaluate(&conn, "other_tool", "digest-a").unwrap(), None);
        assert!(save_rule(
            &conn,
            &rule(
                "ambiguous",
                Effect::Allow,
                RuleScope::Global,
                Some("digest-a")
            )
        )
        .is_err());
        assert!(save_rule(
            &conn,
            &rule("unbound-subject", Effect::Block, RuleScope::Subject, None)
        )
        .is_err());
        save_rule(
            &conn,
            &rule("block", Effect::Block, RuleScope::Subject, Some("digest-b")),
        )
        .unwrap();
        assert_eq!(
            evaluate(&conn, "delete_skill", "digest-b").unwrap(),
            Some(Effect::Block)
        );
        assert_eq!(
            evaluate(&conn, "delete_skill", "digest-a").unwrap(),
            Some(Effect::Allow)
        );
    }

    #[test]
    fn rules_block_wins_regardless_of_insertion_order_or_scope() {
        for block_first in [true, false] {
            let conn = connection();
            let allow = rule("allow", Effect::Allow, RuleScope::Subject, Some("digest-a"));
            let block = rule("block", Effect::Block, RuleScope::Global, None);
            for rule in if block_first {
                [&block, &allow]
            } else {
                [&allow, &block]
            } {
                save_rule(&conn, rule).unwrap();
            }
            assert_eq!(
                evaluate(&conn, "delete_skill", "digest-a").unwrap(),
                Some(Effect::Block)
            );
            assert_eq!(
                evaluate(&conn, "delete_skill", "digest-b").unwrap(),
                Some(Effect::Block)
            );
            assert_eq!(evaluate(&conn, "other_tool", "digest-a").unwrap(), None);
            assert!(remove_rule(&conn, "block").unwrap());
            assert!(!remove_rule(&conn, "block").unwrap());
            assert_eq!(
                evaluate(&conn, "delete_skill", "digest-a").unwrap(),
                Some(Effect::Allow)
            );
        }
    }

    #[test]
    fn rules_call_block_is_digest_bound_and_bad_inputs_fail_closed() {
        let conn = connection();
        assert_eq!(evaluate(&conn, "delete_skill", "digest-a").unwrap(), None);
        save_rule(
            &conn,
            &rule("block", Effect::Block, RuleScope::Call, Some("digest-a")),
        )
        .unwrap();
        assert_eq!(
            evaluate(&conn, "delete_skill", "digest-a").unwrap(),
            Some(Effect::Block)
        );
        assert_eq!(evaluate(&conn, "delete_skill", "digest-b").unwrap(), None);
        assert!(save_rule(&conn, &rule("bad", Effect::Block, RuleScope::Call, None)).is_err());
        assert!(save_rule(
            &conn,
            &rule(" ", Effect::Allow, RuleScope::Call, Some("digest-a"))
        )
        .is_err());
        assert!(save_rule(
            &conn,
            &rule("bad", Effect::Allow, RuleScope::Call, Some(" "))
        )
        .is_err());
        assert!(evaluate(&conn, "delete_skill", "").is_err());
        assert!(evaluate(&conn, " ", "digest-a").is_err());
    }

    #[test]
    fn rules_save_replace_and_remove_share_caller_rollback() {
        let conn = connection();
        let grant = rule("rule-1", Effect::Allow, RuleScope::Call, Some("digest-a"));
        save_rule(&conn, &grant).unwrap();
        let failed: Result<()> = with_tx(&conn, |conn| {
            let mut updated = grant.clone();
            updated.effect = Effect::Block;
            save_rule(conn, &updated)?;
            save_rule(
                conn,
                &rule(
                    "rule-2",
                    Effect::Allow,
                    RuleScope::Subject,
                    Some("digest-b"),
                ),
            )?;
            remove_rule(conn, "rule-1")?;
            Err(invalid("rollback"))
        });
        assert!(failed.is_err());
        assert_eq!(list_rules(&conn).unwrap(), vec![grant.clone()]);
        let mut updated = grant;
        updated.args_digest = Some("digest-b".into());
        save_rule(&conn, &updated).unwrap();
        assert_eq!(list_rules(&conn).unwrap().len(), 1);
        assert_eq!(evaluate(&conn, "delete_skill", "digest-a").unwrap(), None);
        assert_eq!(
            evaluate(&conn, "delete_skill", "digest-b").unwrap(),
            Some(Effect::Allow)
        );
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
        assert_eq!(evaluate(&conn, "delete_skill", "digest-a").unwrap(), None);
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
            [
                serde_json::json!({"enabled": true, "allow": ["x".repeat(1001)], "block": []})
                    .to_string(),
            ],
        )
        .unwrap();
        assert!(load_auto_review_config(&conn).is_err());
    }

    #[test]
    fn rules_identifiers_are_literal_and_listing_order_is_stable() {
        let conn = connection();
        let mut a = rule("a", Effect::Allow, RuleScope::Call, Some("digest-a"));
        a.tool = "tool'; DROP TABLE forms_permission_rules; --".into();
        let mut b = a.clone();
        b.rule_id = "b".into();
        save_rule(&conn, &b).unwrap();
        save_rule(&conn, &a).unwrap();
        assert_eq!(list_rules(&conn).unwrap(), vec![a.clone(), b]);
        assert_eq!(
            evaluate(&conn, &a.tool, "digest-a").unwrap(),
            Some(Effect::Allow)
        );
        assert_eq!(evaluate(&conn, "tool", "digest-a").unwrap(), None);
        assert!(!remove_rule(&conn, "a' OR 1=1 --").unwrap());
        assert_eq!(list_rules(&conn).unwrap().len(), 2);
    }

    #[test]
    fn rules_migration_is_versioned_without_user_version_or_downgrade() {
        let conn = connection();
        conn.pragma_update(None, "user_version", 17).unwrap();
        set_component_version(&conn, "other", 9).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(component_version(&conn, "forms_rules").unwrap(), 1);
        assert_eq!(component_version(&conn, "other").unwrap(), 9);
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            17
        );
        set_component_version(&conn, "forms_rules", 2).unwrap();
        assert!(migrate(&conn).is_err());
        assert_eq!(component_version(&conn, "forms_rules").unwrap(), 2);
    }

    #[test]
    fn rules_and_auto_review_config_survive_reopen() {
        let mut bytes = [0; 8];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = std::env::temp_dir().join(format!(
            "grokbot-rules-{}-{:x}.sqlite",
            std::process::id(),
            u64::from_ne_bytes(bytes)
        ));
        let grant = rule(
            "rule-1",
            Effect::Allow,
            RuleScope::Subject,
            Some("digest-a"),
        );
        let config = AutoReviewConfig {
            enabled: true,
            allow: vec!["allow reviewed actions".into()],
            block: vec![],
        };
        {
            let conn = Connection::open(&path).unwrap();
            with_tx(&conn, |conn| -> Result<()> {
                migrate(conn)?;
                save_rule(conn, &grant)?;
                save_auto_review_config(conn, &config)
            })
            .unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            migrate(&conn).unwrap();
            assert_eq!(list_rules(&conn).unwrap(), vec![grant]);
            assert_eq!(load_auto_review_config(&conn).unwrap(), config);
            assert_eq!(
                evaluate(&conn, "delete_skill", "digest-a").unwrap(),
                Some(Effect::Allow)
            );
            remove_rule(&conn, "rule-1").unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            assert_eq!(evaluate(&conn, "delete_skill", "digest-a").unwrap(), None);
        }
        std::fs::remove_file(path).unwrap();
    }
}
