//! Argument-bound, single-use approvals; no routine activation policy is imposed.
//!
//! `reports/goal-permissions-secrets.md` PS01–PS05 establish binding to an action,
//! separate retirement from cancellation, and save Always rules before resolution.
//! Single-use grants bind exact canonical argument digests. Persisted Global
//! permissions cover the same tool across calls; Subject rules retain the digest.
//! Decisions must originate in the request's principal-bot conversation; the
//! wrapper authenticates the user and supplies the trusted ApprovalRoute. Stored
//! permissions let [`auto_review`] resolve subsequent requests.
use crate::requests::{self, invalid, HumanInputRequest, InputKind, RequestStatus};
use grokbot_engine::contracts::{ApprovalRoute, Result, ResumeReceipt};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const EXPIRY_SUBMISSION_ID: &str = "expiry";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AlwaysAllow { scope: RuleScope },
    Deny,
    Never { scope: RuleScope },
}

/// PS01 policy scope: Call is one-use, Subject binds the exact digest (no local
/// subject extractor exists), Global covers the tool for this requesting/main bot
/// pair. Pattern binds one normalized shell command. Each resulting execution
/// grant still binds its own exact action digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Call,
    Subject,
    Global,
    Pattern,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRule {
    pub route: ApprovalRoute,
    pub tool: String,
    pub args_digest: String,
    pub scope: RuleScope,
    pub allow: bool,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS forms_approval_grants (
            request_id TEXT PRIMARY KEY, tool TEXT NOT NULL, args_digest TEXT NOT NULL,
            consumed_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS forms_approval_rules (
            requesting_bot_id TEXT NOT NULL, target_bot_id TEXT NOT NULL,
            target_conversation_id TEXT NOT NULL, tool TEXT NOT NULL, args_digest TEXT NOT NULL,
            scope TEXT NOT NULL, allow INTEGER NOT NULL CHECK(allow IN (0,1)),
            PRIMARY KEY(requesting_bot_id, target_bot_id, tool, args_digest, scope)
        );",
    )?;
    Ok(())
}

// Sort recursively rather than depending on serde_json's map feature flags.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<_> = object.keys().collect();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical(&object[key])))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}

/// SHA-256 of a canonical JSON tuple; tool name and argument boundaries are bound.
/// Object order is irrelevant; array order and all JSON value types are retained.
pub fn args_digest(tool: &str, arguments: &Value) -> Result<String> {
    if tool.trim().is_empty() {
        return Err(invalid("approval tool name is required"));
    }
    let bytes = serde_json::to_vec(&(tool, canonical(arguments)))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn approval(conn: &Connection, id: &str) -> Result<HumanInputRequest> {
    let request = requests::get(conn, id)?.ok_or_else(|| invalid("unknown approval request"))?;
    if !matches!(request.kind, InputKind::Approval { .. }) {
        return Err(invalid("request is not an approval"));
    }
    Ok(request)
}

/// Resolve overdue *pending decisions*, without expiring an already granted
/// action. The caller enqueues and attaches each Approval wake in this same
/// transaction, just as for Deny. The deterministic submission ID is `expiry`.
pub fn expire_pending(
    conn: &Connection,
    now_ms: i64,
) -> Result<Vec<(HumanInputRequest, ResumeReceipt)>> {
    requests::require_transaction(conn)?;
    let mut statement = conn.prepare(
        "SELECT id FROM forms_requests WHERE status='pending'
         AND json_extract(kind_json, '$.kind')='approval'
         AND json_extract(kind_json, '$.expires_at')<=?1 ORDER BY rowid",
    )?;
    let ids = statement
        .query_map([now_ms], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut expired = Vec::new();
    for id in ids {
        let request = approval(conn, &id)?;
        let receipt = requests::resolve(
            conn,
            &request,
            EXPIRY_SUBMISSION_ID,
            &Value::String("expired".into()),
            "expired",
        )?;
        let request = approval(conn, &id)?;
        expired.push((request, receipt));
    }
    Ok(expired)
}

/// The wrapper resolves and enqueues/attaches the wake in its transaction. Denial
/// also has a receipt and wakes the originating run with an explicit tool refusal.
/// Always persists before granting; failed persistence degrades to AllowOnce in
/// this transaction (PS01/PS05). Call-scoped Always is also AllowOnce (PS01).
/// Never persists a block when possible, but always denies the current request.
pub fn decide(
    conn: &Connection,
    id: &str,
    deciding_conversation_id: &str,
    submission_id: &str,
    decision: ApprovalDecision,
    now_ms: i64,
) -> Result<ResumeReceipt> {
    decide_with_pattern(
        conn,
        id,
        deciding_conversation_id,
        submission_id,
        decision,
        now_ms,
        None,
    )
}

/// `pattern_key` is the server-derived command text for [`RuleScope::Pattern`].
/// A missing or invalid key degrades to a one-shot decision, same as Call.
pub fn decide_with_pattern(
    conn: &Connection,
    id: &str,
    deciding_conversation_id: &str,
    submission_id: &str,
    decision: ApprovalDecision,
    now_ms: i64,
    pattern_key: Option<&str>,
) -> Result<ResumeReceipt> {
    let request = approval(conn, id)?;
    if deciding_conversation_id != request.route.target_conversation_id {
        return Err(invalid(
            "approval must be decided in the principal conversation",
        ));
    }
    if let Some(receipt) = requests::replay(conn, &request.entry_id, submission_id)? {
        return Ok(receipt);
    }
    requests::require_transaction(conn)?;
    if request.status != RequestStatus::Pending {
        return Err(invalid("approval is no longer pending"));
    }
    let InputKind::Approval {
        tool,
        args_digest,
        expires_at,
    } = &request.kind
    else {
        unreachable!()
    };
    if now_ms >= *expires_at {
        // Do not write and then return Err: the caller would roll expiry back.
        // expire_pending returns the durable resolution/wake work to its caller.
        return Err(invalid("approval expired"));
    }
    let effective = match decision {
        ApprovalDecision::AlwaysAllow { scope } => {
            if !can_persist(scope, pattern_key)
                || save_rule(conn, &request, scope, true, pattern_key).is_err()
            {
                requests::require_transaction(conn)?;
                ApprovalDecision::AllowOnce
            } else {
                decision
            }
        }
        ApprovalDecision::Never { scope } => {
            if !can_persist(scope, pattern_key)
                || save_rule(conn, &request, scope, false, pattern_key).is_err()
            {
                requests::require_transaction(conn)?;
                ApprovalDecision::Deny
            } else {
                decision
            }
        }
        _ => decision,
    };
    let denied = matches!(
        effective,
        ApprovalDecision::Deny | ApprovalDecision::Never { .. }
    );
    let receipt = requests::resolve(
        conn,
        &request,
        submission_id,
        &serde_json::to_value(effective)?,
        if denied { "denied" } else { "resolved" },
    )?;
    if !denied {
        conn.execute(
            "INSERT INTO forms_approval_grants (request_id, tool, args_digest) VALUES (?1,?2,?3)",
            params![id, tool, args_digest],
        )?;
    }
    Ok(receipt)
}

fn can_persist(scope: RuleScope, pattern_key: Option<&str>) -> bool {
    match scope {
        RuleScope::Call => false,
        RuleScope::Pattern => pattern_key.is_some_and(crate::policy::valid_pattern_key),
        RuleScope::Subject | RuleScope::Global => true,
    }
}

fn save_rule(
    conn: &Connection,
    request: &HumanInputRequest,
    scope: RuleScope,
    allow: bool,
    pattern_key: Option<&str>,
) -> Result<()> {
    let InputKind::Approval {
        tool, args_digest, ..
    } = &request.kind
    else {
        return Err(invalid("not an approval"));
    };
    let stored = match scope {
        RuleScope::Global => "",
        RuleScope::Pattern => pattern_key.unwrap_or(""),
        RuleScope::Call | RuleScope::Subject => args_digest.as_str(),
    };
    if scope == RuleScope::Pattern && !crate::policy::valid_pattern_key(stored) {
        return Err(invalid("pattern key is required"));
    }
    let route = &request.route;
    conn.execute(
        "INSERT INTO forms_approval_rules
         (requesting_bot_id,target_bot_id,target_conversation_id,tool,args_digest,scope,allow)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(requesting_bot_id,target_bot_id,tool,args_digest,scope)
         DO UPDATE SET allow=excluded.allow, target_conversation_id=excluded.target_conversation_id",
        params![
            route.requesting_bot_id,
            route.target_bot_id,
            route.target_conversation_id,
            tool,
            stored,
            serde_json::to_string(&scope)?,
            allow
        ],
    )?;
    Ok(())
}

/// Atomic admission, consumed once even across connections. Call in the same
/// transaction as an internal tool effect/result event. External effects require
/// their own durable dispatch/outcome protocol (outside this slice).
pub fn consume_allow_once(
    conn: &Connection,
    id: &str,
    run_id: &str,
    tool: &str,
    arguments: &Value,
    now_ms: i64,
) -> Result<()> {
    let request = approval(conn, id)?;
    if request.run_id != run_id {
        return Err(invalid("approval belongs to a different run"));
    }
    let digest = args_digest(tool, arguments)?;
    let InputKind::Approval {
        tool: approved_tool,
        args_digest: approved_digest,
        ..
    } = &request.kind
    else {
        unreachable!()
    };
    if approved_tool != tool || approved_digest != &digest {
        return Err(invalid("approval does not match tool and argument digest"));
    }
    let changed = conn.execute(
        "UPDATE forms_approval_grants SET consumed_at=?1 WHERE request_id=?2 AND tool=?3 AND args_digest=?4
         AND consumed_at IS NULL
         AND EXISTS (SELECT 1 FROM forms_requests WHERE id=?2 AND run_id=?5 AND status='resolved')",
        params![now_ms, id, tool, digest, run_id],
    )?;
    if changed != 1 {
        return Err(invalid("approval is not allowed or was already consumed"));
    }
    Ok(())
}

/// Persisted user permissions only; no heuristic/model reviewer is invented.
/// Rule identity excludes the display conversation so a new principal thread
/// does not silently discard permissions. Each grant still binds this run/call.
/// New requests without a matching rule remain pending for the user in the main
/// bot conversation. Even a rule-approved request receives a single-use grant.
pub fn auto_review(
    conn: &Connection,
    id: &str,
    submission_id: &str,
    now_ms: i64,
) -> Result<Option<ResumeReceipt>> {
    auto_review_matching(conn, id, submission_id, now_ms, None)
}

/// `pattern_key` matches a command rule in addition to subject and global rules.
pub fn auto_review_matching(
    conn: &Connection,
    id: &str,
    submission_id: &str,
    now_ms: i64,
    pattern_key: Option<&str>,
) -> Result<Option<ResumeReceipt>> {
    let request = approval(conn, id)?;
    if let Some(receipt) = requests::replay(conn, &request.entry_id, submission_id)? {
        return Ok(Some(receipt));
    }
    if request.status != RequestStatus::Pending {
        return Err(invalid("approval is no longer pending"));
    }
    let route = &request.route;
    let InputKind::Approval {
        tool, args_digest, ..
    } = &request.kind
    else {
        unreachable!()
    };
    let pattern = pattern_key.unwrap_or("");
    let allow: Option<bool> = conn
        .query_row(
            "SELECT allow FROM forms_approval_rules WHERE requesting_bot_id=?1 AND target_bot_id=?2
             AND tool=?3 AND (args_digest=?4 OR scope='\"global\"' OR (scope='\"pattern\"' AND args_digest=?5))
             ORDER BY allow ASC LIMIT 1",
            params![
                route.requesting_bot_id,
                route.target_bot_id,
                tool,
                args_digest,
                pattern
            ],
            |row| row.get(0),
        )
        .optional()?;
    let Some(allow) = allow else {
        return Ok(None);
    };
    decide(
        conn,
        id,
        &request.route.target_conversation_id,
        submission_id,
        if allow {
            ApprovalDecision::AllowOnce
        } else {
            ApprovalDecision::Deny
        },
        now_ms,
    )
    .map(Some)
}

pub fn list_rules(conn: &Connection) -> Result<Vec<ApprovalRule>> {
    let mut statement = conn.prepare(
        "SELECT requesting_bot_id, target_bot_id, target_conversation_id, tool, args_digest, scope, allow
         FROM forms_approval_rules ORDER BY requesting_bot_id, target_bot_id, target_conversation_id, tool, args_digest, scope",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            ApprovalRoute {
                requesting_bot_id: row.get(0)?,
                target_bot_id: row.get(1)?,
                target_conversation_id: row.get(2)?,
            },
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, bool>(6)?,
        ))
    })?;
    rows.map(|row| {
        let (route, tool, args_digest, scope, allow) = row?;
        Ok(ApprovalRule {
            route,
            tool,
            args_digest,
            scope: serde_json::from_str(&scope)?,
            allow,
        })
    })
    .collect()
}

/// Retirement affects future reviews, not effects/grants already admitted (PS03).
pub fn revoke_rule(conn: &Connection, rule: &ApprovalRule) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM forms_approval_rules WHERE requesting_bot_id=?1 AND target_bot_id=?2
         AND tool=?3 AND args_digest=?4 AND scope=?5",
        params![
            rule.route.requesting_bot_id,
            rule.route.target_bot_id,
            rule.tool,
            rule.args_digest,
            serde_json::to_string(&rule.scope)?
        ],
    )? != 0)
}

#[cfg(test)]
mod approvals_tests {
    use super::*;
    use crate::requests::requests_tests::{database, form};
    use grokbot_engine::contracts::with_tx;
    use serde_json::json;

    fn request(id: &str, entry: &str) -> HumanInputRequest {
        HumanInputRequest {
            id: id.into(),
            entry_id: entry.into(),
            run_id: "child-run".into(),
            route: ApprovalRoute {
                requesting_bot_id: "child-bot".into(),
                target_bot_id: "main-bot".into(),
                target_conversation_id: "main-conversation".into(),
            },
            kind: InputKind::Approval {
                tool: "write".into(),
                args_digest: args_digest("write", &json!({"path":"a","value":1})).unwrap(),
                expires_at: 100,
            },
            status: RequestStatus::Pending,
        }
    }

    // Most tests use this fixed trusted execution/display context. Tests of
    // rejected contexts call the public APIs explicitly via super::.
    fn decide(
        conn: &Connection,
        id: &str,
        submission_id: &str,
        decision: ApprovalDecision,
        now_ms: i64,
    ) -> Result<ResumeReceipt> {
        super::decide(
            conn,
            id,
            "main-conversation",
            submission_id,
            decision,
            now_ms,
        )
    }

    fn consume_allow_once(
        conn: &Connection,
        id: &str,
        tool: &str,
        arguments: &Value,
        now_ms: i64,
    ) -> Result<()> {
        super::consume_allow_once(conn, id, "child-run", tool, arguments, now_ms)
    }

    fn allow(conn: &Connection) -> ResumeReceipt {
        with_tx(conn, |conn| {
            decide(conn, "a", "s", ApprovalDecision::AllowOnce, 1)
        })
        .unwrap()
    }

    #[test]
    fn approval_bound_to_args_digest() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        allow(&conn);
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"b","value":1}), 2).is_err()
        );
        assert!(
            consume_allow_once(&conn, "a", "delete", &json!({"path":"a","value":1}), 2).is_err()
        );
        consume_allow_once(&conn, "a", "write", &json!({"value":1,"path":"a"}), 2).unwrap();
        assert_eq!(
            args_digest("tool", &json!({"b":{"d":2,"c":1},"a":[1,2]})).unwrap(),
            args_digest("tool", &json!({"a":[1,2],"b":{"c":1,"d":2}})).unwrap()
        );
        assert_ne!(
            args_digest("tool", &json!([1, 2])).unwrap(),
            args_digest("tool", &json!([2, 1])).unwrap()
        );
    }

    #[test]
    fn allow_once_consumed_once() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        let first = allow(&conn);
        consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).unwrap();
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).is_err()
        );
        assert_eq!(
            decide(&conn, "a", "s", ApprovalDecision::Deny, 3).unwrap(),
            first
        );
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 3).is_err()
        );
        assert!(with_tx(&conn, |conn| decide(
            conn,
            "a",
            "other",
            ApprovalDecision::AllowOnce,
            3
        ))
        .is_err());
    }

    #[test]
    fn expired_approval_refused() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        assert!(with_tx(&conn, |conn| decide(
            conn,
            "a",
            "s",
            ApprovalDecision::AllowOnce,
            100
        ))
        .is_err());
        assert!(expire_pending(&conn, 100).is_err());
        let expired = with_tx(&conn, |conn| expire_pending(conn, 100)).unwrap();
        assert_eq!(expired.len(), 1);
        let (request, receipt) = &expired[0];
        assert_eq!(
            request.status,
            RequestStatus::Expired {
                resolution_id: receipt.resolution_id.clone()
            }
        );
        assert_eq!(
            requests::resolved_values(&conn, &receipt.resolution_id).unwrap(),
            json!("expired")
        );
        assert_eq!(
            requests::receipt(&conn, "e", EXPIRY_SUBMISSION_ID).unwrap(),
            *receipt
        );
        with_tx(&conn, |conn| {
            requests::attach_wake(
                conn,
                &receipt.resolution_id,
                &grokbot_engine::store::Wake {
                    wake_id: "expiry-wake".into(),
                    run_id: request.run_id.clone(),
                    cause: grokbot_engine::contracts::WakeCause::Approval,
                    reference: receipt.resolution_id.clone(),
                    status: grokbot_engine::contracts::WakeStatus::Pending,
                },
            )
        })
        .unwrap();
        assert!(with_tx(&conn, |conn| expire_pending(conn, 200))
            .unwrap()
            .is_empty());
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 200).is_err()
        );
    }

    #[test]
    fn approved_action_survives_late_tick_and_execution_history_is_terminal() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        let granted = with_tx(&conn, |conn| {
            decide(conn, "a", "s", ApprovalDecision::AllowOnce, 99)
        })
        .unwrap();
        assert!(with_tx(&conn, |conn| expire_pending(conn, 101))
            .unwrap()
            .is_empty());
        with_tx(&conn, |conn| {
            consume_allow_once(conn, "a", "write", &json!({"path":"a","value":1}), 200)
        })
        .unwrap();
        assert!(with_tx(&conn, |conn| expire_pending(conn, 300))
            .unwrap()
            .is_empty());
        assert_eq!(requests::cancel_for_run(&conn, "child-run").unwrap(), 0);
        assert_eq!(
            requests::get(&conn, "a").unwrap().unwrap().status,
            RequestStatus::Resolved {
                resolution_id: granted.resolution_id
            }
        );
    }

    #[test]
    fn cancelled_unconsumed_approval_keeps_its_resolution() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        let granted = allow(&conn);
        assert_eq!(requests::cancel_for_run(&conn, "child-run").unwrap(), 1);
        assert_eq!(
            requests::get(&conn, "a").unwrap().unwrap().status,
            RequestStatus::Cancelled {
                resolution_id: Some(granted.resolution_id.clone())
            }
        );
        assert_eq!(requests::receipt(&conn, "e", "s").unwrap(), granted);
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).is_err()
        );
    }

    #[test]
    fn approvals_enforce_deciding_conversation_and_consuming_run() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        assert!(with_tx(&conn, |conn| super::decide(
            conn,
            "a",
            "child-conversation",
            "s",
            ApprovalDecision::AllowOnce,
            1
        ))
        .is_err());
        let granted = allow(&conn);
        assert!(super::decide(
            &conn,
            "a",
            "child-conversation",
            "s",
            ApprovalDecision::AllowOnce,
            2
        )
        .is_err());
        let args = json!({"path":"a","value":1});
        assert!(super::consume_allow_once(&conn, "a", "different-run", "write", &args, 2).is_err());
        consume_allow_once(&conn, "a", "write", &args, 2).unwrap();
        assert_eq!(requests::receipt(&conn, "e", "s").unwrap(), granted);
    }

    #[test]
    fn global_permission_covers_new_arguments_but_each_grant_remains_bound() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        with_tx(&conn, |conn| {
            decide(
                conn,
                "a",
                "s",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Global,
                },
                1,
            )
        })
        .unwrap();
        let mut next = request("b", "f");
        next.route.target_conversation_id = "new-principal-thread".into();
        if let InputKind::Approval {
            args_digest: digest,
            ..
        } = &mut next.kind
        {
            *digest = args_digest("write", &json!({"path":"b","value":2})).unwrap();
        }
        requests::create(&conn, &next).unwrap();
        with_tx(&conn, |conn| auto_review(conn, "b", "auto", 2))
            .unwrap()
            .unwrap();
        assert!(
            consume_allow_once(&conn, "b", "write", &json!({"path":"c","value":3}), 3).is_err()
        );
        consume_allow_once(&conn, "b", "write", &json!({"path":"b","value":2}), 3).unwrap();
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"b","value":2}), 3).is_err()
        );
        let rules = list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].args_digest, "");
        assert_eq!(rules[0].scope, RuleScope::Global);
        requests::create(&conn, &request("block", "block-entry")).unwrap();
        with_tx(&conn, |conn| {
            decide(
                conn,
                "block",
                "s",
                ApprovalDecision::Never {
                    scope: RuleScope::Global,
                },
                1,
            )
        })
        .unwrap();
        next.id = "c".into();
        next.entry_id = "g".into();
        requests::create(&conn, &next).unwrap();
        with_tx(&conn, |conn| auto_review(conn, "c", "auto", 2))
            .unwrap()
            .unwrap();
        assert_eq!(
            requests::get(&conn, "c").unwrap().unwrap().status,
            RequestStatus::Denied
        );
        assert!(revoke_rule(&conn, &list_rules(&conn).unwrap()[0]).unwrap());
    }

    #[test]
    fn approvals_deny_cancel_and_unresolved_never_grant() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        let args = json!({"path":"a","value":1});
        assert!(consume_allow_once(&conn, "a", "write", &args, 1).is_err());
        let denied = with_tx(&conn, |conn| {
            decide(conn, "a", "s", ApprovalDecision::Deny, 2)
        })
        .unwrap();
        assert!(consume_allow_once(&conn, "a", "write", &args, 3).is_err());
        assert_eq!(
            requests::get(&conn, "a").unwrap().unwrap().status,
            RequestStatus::Denied
        );
        assert_eq!(requests::receipt(&conn, "e", "s").unwrap(), denied);
        requests::create(&conn, &request("b", "f")).unwrap();
        with_tx(&conn, |conn| {
            decide(conn, "b", "s", ApprovalDecision::AllowOnce, 1)
        })
        .unwrap();
        requests::cancel_for_run(&conn, "child-run").unwrap();
        assert!(consume_allow_once(&conn, "b", "write", &args, 3).is_err());
    }

    #[test]
    fn approvals_cannot_bypass_typed_api_or_principal_route() {
        let conn = database();
        let mut approval = request("a", "e");
        approval.route.target_conversation_id.clear();
        assert!(requests::create(&conn, &approval).is_err());
        let approval = request("a", "e");
        assert_eq!(requests::create(&conn, &approval).unwrap(), approval);
        assert!(requests::submit(&conn, "e", "s", &json!({"allow":true})).is_err());
        requests::create(&conn, &form()).unwrap();
        assert!(decide(&conn, "request-1", "s", ApprovalDecision::AllowOnce, 1).is_err());
    }

    #[test]
    fn approvals_always_rule_is_persisted_scoped_digest_bound_and_revocable() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        assert!(auto_review(&conn, "a", "auto", 1).unwrap().is_none());
        with_tx(&conn, |conn| {
            decide(
                conn,
                "a",
                "user",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Subject,
                },
                1,
            )
        })
        .unwrap();
        let rules = list_rules(&conn).unwrap();
        assert_eq!(rules.len(), 1);
        requests::create(&conn, &request("b", "f")).unwrap();
        let result = with_tx(&conn, |conn| auto_review(conn, "b", "auto", 2))
            .unwrap()
            .unwrap();
        assert_eq!(requests::receipt(&conn, "f", "auto").unwrap(), result);
        consume_allow_once(&conn, "b", "write", &json!({"path":"a","value":1}), 3).unwrap();
        assert!(
            consume_allow_once(&conn, "b", "write", &json!({"path":"a","value":1}), 3).is_err()
        );
        let mut changed = request("c", "g");
        if let InputKind::Approval {
            args_digest: digest,
            ..
        } = &mut changed.kind
        {
            *digest = args_digest("write", &json!({"path":"b","value":1})).unwrap();
        }
        requests::create(&conn, &changed).unwrap();
        assert!(auto_review(&conn, "c", "auto", 1).unwrap().is_none());
        let mut other_bot = request("d", "h");
        other_bot.route.requesting_bot_id = "other-bot".into();
        requests::create(&conn, &other_bot).unwrap();
        assert!(auto_review(&conn, "d", "auto", 1).unwrap().is_none());
        assert!(revoke_rule(&conn, &rules[0]).unwrap());
        requests::create(&conn, &request("next", "next-entry")).unwrap();
        assert!(auto_review(&conn, "next", "auto", 1).unwrap().is_none());
    }

    #[test]
    fn approvals_grant_and_rule_roll_back_with_wake_failure() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        let failed: Result<()> = with_tx(&conn, |conn| {
            decide(
                conn,
                "a",
                "s",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Subject,
                },
                1,
            )?;
            Err(invalid("wake failed"))
        });
        assert!(failed.is_err());
        assert!(list_rules(&conn).unwrap().is_empty());
        assert_eq!(
            requests::get(&conn, "a").unwrap().unwrap().status,
            RequestStatus::Pending
        );
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).is_err()
        );
    }

    #[test]
    fn approvals_failed_rule_save_degrades_in_same_transaction() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        conn.execute_batch("CREATE TRIGGER fail_rule BEFORE INSERT ON forms_approval_rules BEGIN SELECT RAISE(ABORT, 'rule storage failed'); END;").unwrap();
        let receipt = with_tx(&conn, |conn| {
            decide(
                conn,
                "a",
                "s",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Global,
                },
                1,
            )
        })
        .unwrap();
        assert!(list_rules(&conn).unwrap().is_empty());
        assert_eq!(
            requests::resolved_values(&conn, &receipt.resolution_id).unwrap(),
            json!("allow_once")
        );
        requests::create(&conn, &request("b", "f")).unwrap();
        let denied = with_tx(&conn, |conn| {
            decide(
                conn,
                "b",
                "s",
                ApprovalDecision::Never {
                    scope: RuleScope::Global,
                },
                1,
            )
        })
        .unwrap();
        assert_eq!(
            requests::resolved_values(&conn, &denied.resolution_id).unwrap(),
            json!("deny")
        );
        assert!(list_rules(&conn).unwrap().is_empty());
        consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).unwrap();
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 2).is_err()
        );
    }

    #[test]
    fn approvals_call_scope_does_not_persist_and_never_blocks_future_requests() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        with_tx(&conn, |conn| {
            decide(
                conn,
                "a",
                "s",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Call,
                },
                1,
            )
        })
        .unwrap();
        assert!(list_rules(&conn).unwrap().is_empty());
        requests::create(&conn, &request("b", "f")).unwrap();
        with_tx(&conn, |conn| {
            decide(
                conn,
                "b",
                "s",
                ApprovalDecision::AlwaysAllow {
                    scope: RuleScope::Global,
                },
                1,
            )
        })
        .unwrap();
        requests::create(&conn, &request("c", "g")).unwrap();
        with_tx(&conn, |conn| {
            decide(
                conn,
                "c",
                "s",
                ApprovalDecision::Never {
                    scope: RuleScope::Subject,
                },
                1,
            )
        })
        .unwrap();
        requests::create(&conn, &request("next", "next-entry")).unwrap();
        with_tx(&conn, |conn| auto_review(conn, "next", "auto", 2))
            .unwrap()
            .unwrap();
        assert_eq!(
            requests::get(&conn, "next").unwrap().unwrap().status,
            RequestStatus::Denied
        );
        assert!(
            consume_allow_once(&conn, "next", "write", &json!({"path":"a","value":1}), 3).is_err()
        );
        for scope in [RuleScope::Call, RuleScope::Subject, RuleScope::Global] {
            let decision = ApprovalDecision::Never { scope };
            assert_eq!(
                serde_json::from_value::<ApprovalDecision>(serde_json::to_value(decision).unwrap())
                    .unwrap(),
                decision
            );
        }
    }

    #[test]
    fn approvals_expiry_is_explicit_and_migrations_preserve_other_components() {
        let conn = database();
        conn.execute_batch("PRAGMA user_version=42").unwrap();
        requests::create(&conn, &request("a", "e")).unwrap();
        requests::create(&conn, &form()).unwrap();
        assert!(with_tx(&conn, |conn| expire_pending(conn, 99))
            .unwrap()
            .is_empty());
        let expired = with_tx(&conn, |conn| expire_pending(conn, 100)).unwrap();
        assert_eq!(expired.len(), 1);
        assert!(with_tx(&conn, |conn| expire_pending(conn, 101))
            .unwrap()
            .is_empty());
        let expected = RequestStatus::Expired {
            resolution_id: expired[0].1.resolution_id.clone(),
        };
        assert_eq!(requests::get(&conn, "a").unwrap().unwrap().status, expected);
        assert_eq!(
            requests::get(&conn, "request-1").unwrap().unwrap().status,
            RequestStatus::Pending
        );
        crate::migrate(&conn).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert_eq!(requests::get(&conn, "a").unwrap().unwrap().status, expected);
    }

    #[test]
    fn approvals_single_use_is_atomic_across_connections() {
        use std::sync::{Arc, Barrier};
        struct TempDb(std::path::PathBuf);
        impl Drop for TempDb {
            fn drop(&mut self) {
                for suffix in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
                }
            }
        }
        let mut random = [0_u8; 8];
        getrandom::getrandom(&mut random).unwrap();
        let file = TempDb(std::env::temp_dir().join(format!(
            "forms-approval-{}-{:x}.sqlite",
            std::process::id(),
            u64::from_ne_bytes(random)
        )));
        let conn = Connection::open(&file.0).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL").unwrap();
        crate::migrate(&conn).unwrap();
        requests::create(&conn, &request("a", "e")).unwrap();
        allow(&conn);
        let barrier = Arc::new(Barrier::new(2));
        let attempts: Vec<_> = (0..2)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let path = file.0.clone();
                std::thread::spawn(move || -> Result<()> {
                    let conn = Connection::open(path)?;
                    conn.busy_timeout(std::time::Duration::from_secs(5))?;
                    barrier.wait();
                    let tx = rusqlite::Transaction::new_unchecked(
                        &conn,
                        rusqlite::TransactionBehavior::Immediate,
                    )?;
                    consume_allow_once(&tx, "a", "write", &json!({"path":"a","value":1}), 2)?;
                    tx.commit()?;
                    Ok(())
                })
            })
            .collect();
        let results: Vec<_> = attempts
            .into_iter()
            .map(|attempt| attempt.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let error = results.into_iter().find_map(|result| result.err()).unwrap();
        assert!(
            matches!(error, grokbot_engine::contracts::Error::InvalidInput(ref reason) if reason.contains("already consumed")),
            "{error}"
        );
        assert!(
            consume_allow_once(&conn, "a", "write", &json!({"path":"a","value":1}), 3).is_err()
        );
    }

    #[test]
    fn approvals_consumption_rolls_back_with_tool_effect() {
        let conn = database();
        requests::create(&conn, &request("a", "e")).unwrap();
        allow(&conn);
        let args = json!({"path":"a","value":1});
        let failed: Result<()> = with_tx(&conn, |conn| {
            consume_allow_once(conn, "a", "write", &args, 2)?;
            Err(invalid("tool effect failed"))
        });
        assert!(failed.is_err());
        consume_allow_once(&conn, "a", "write", &args, 3).unwrap();
    }
}
