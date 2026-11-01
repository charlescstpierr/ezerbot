use crate::contracts::{Result, ToolCall, ToolOutcome};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) const RECENT_ATTEMPTS: &str = "SELECT json_extract(payload,'$.effective_attempt')
    FROM engine_run_events WHERE run_id=?1 AND kind='tool_result'
    AND json_type(payload,'$.effective_attempt')='object' ORDER BY seq DESC LIMIT 2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EffectiveOutcome {
    Done,
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct EffectiveAttempt {
    fingerprint: String,
    outcome: EffectiveOutcome,
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut names: Vec<_> = fields.keys().collect();
            names.sort_unstable();
            Value::Object(
                names
                    .into_iter()
                    .map(|name| (name.clone(), canonical(&fields[name])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

pub(crate) fn fingerprint(call: &ToolCall) -> Result<String> {
    let bytes = serde_json::to_vec(&(call.name.as_str(), canonical(&call.arguments)))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn attempt(
    fingerprint: Option<&str>,
    outcome: &ToolOutcome,
) -> Option<EffectiveAttempt> {
    let fingerprint = fingerprint?;
    let outcome = match outcome {
        ToolOutcome::Done { .. } => EffectiveOutcome::Done,
        ToolOutcome::Failed { .. } => EffectiveOutcome::Failed,
        ToolOutcome::Suspend { .. } => return None,
    };
    Some(EffectiveAttempt {
        fingerprint: fingerprint.into(),
        outcome,
    })
}

pub(crate) fn blocks(conn: &Connection, run_id: &str, fingerprint: &str) -> Result<bool> {
    let mut statement = conn.prepare_cached(RECENT_ATTEMPTS)?;
    let rows = statement.query_map([run_id], |row| row.get::<_, String>(0))?;
    let recent: Vec<EffectiveAttempt> = rows
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect::<Result<_>>()?;
    let Some(last) = recent
        .first()
        .filter(|last| last.fingerprint == fingerprint)
    else {
        return Ok(false);
    };
    Ok(last.outcome == EffectiveOutcome::Failed
        || recent
            .get(1)
            .is_some_and(|prior| prior.fingerprint == fingerprint))
}

pub(crate) fn safe_tool_name(name: &str) -> &str {
    if !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        name
    } else {
        "-"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::{with_tx, RunBudgets, RunKind, Trigger};
    use crate::model::{self, Run, RunStatus};
    use crate::store;
    use rusqlite::{params, types::Value as SqlValue, StatementStatus};
    use serde_json::json;

    #[test]
    fn indexed_guard_reads_do_not_scan_ten_times_more_legacy_history() -> Result<()> {
        let mut measurements = Vec::new();
        for history in [1_000, 10_000] {
            let conn = Connection::open_in_memory()?;
            store::migrate(&conn)?;
            with_tx(&conn, |conn| -> Result<()> {
                model::start(
                    conn,
                    &Run {
                        run_id: "scale".into(),
                        bot_id: "bot".into(),
                        conversation_id: "conversation".into(),
                        trigger: Trigger::UserMessage {
                            message_id: "scale-message".into(),
                            attempt: 0,
                        },
                        kind: RunKind::Chat,
                        status: RunStatus::Queued,
                        task_status: None,
                        turn_end: None,
                        budgets: RunBudgets::default(),
                        started_at: 0,
                    },
                )?;
                let mut insert =
                    conn.prepare("INSERT INTO engine_run_events VALUES ('scale',?1,?2,?3)")?;
                for seq in 2..=3 {
                    insert.execute(params![seq,"tool_result",json!({"call_id":format!("actual-{seq}"),"effective_attempt":{"fingerprint":"a".repeat(64),"outcome":"done"}}).to_string()])?;
                }
                for seq in 4..=6 {
                    insert.execute(params![seq, "loop_guard", "{}"])?;
                }
                for seq in 7..history + 7 {
                    insert.execute(params![seq,"tool_result",json!({"call_id":format!("legacy-{seq}"),"outcome":{"status":"done","output":"old"}}).to_string()])?;
                }
                Ok(())
            })?;
            assert!(blocks(&conn, "scale", &"a".repeat(64))?);
            assert_eq!(store::loop_guard_count(&conn, "scale")?, 3);
            assert!(store::tool_result_exists(&conn, "scale", "actual-2")?);
            let mut steps = Vec::new();
            for (sql, parameters, expected_index) in [
                (
                    RECENT_ATTEMPTS,
                    vec![SqlValue::Text("scale".into())],
                    "engine_effective_attempt_tail",
                ),
                (
                    store::LOOP_GUARD_COUNT_SQL,
                    vec![SqlValue::Text("scale".into())],
                    "engine_loop_guard_run",
                ),
                (
                    store::TOOL_RESULT_EXISTS_SQL,
                    vec![
                        SqlValue::Text("scale".into()),
                        SqlValue::Text("actual-2".into()),
                    ],
                    "engine_tool_result_call",
                ),
            ] {
                let plan: Vec<String> = conn
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                    .query_map(rusqlite::params_from_iter(parameters.iter()), |row| {
                        row.get(3)
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                assert!(
                    plan.iter().any(|line| line.contains(expected_index)),
                    "{plan:?}"
                );
                let mut statement = conn.prepare(sql)?;
                {
                    let mut rows =
                        statement.query(rusqlite::params_from_iter(parameters.iter()))?;
                    while rows.next()?.is_some() {}
                }
                let vm_steps = statement.get_status(StatementStatus::VmStep);
                eprintln!("loop_guard_scale history={history} index={expected_index} vm_steps={vm_steps} plan={plan:?}");
                steps.push(vm_steps);
            }
            measurements.push(steps);
        }
        for (small, large) in measurements[0].iter().zip(&measurements[1]) {
            assert!(
                *large <= *small + 16,
                "old history changed indexed work: {small} -> {large}"
            );
        }
        Ok(())
    }
}
