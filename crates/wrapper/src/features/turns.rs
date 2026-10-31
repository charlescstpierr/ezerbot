//! Journal local des tours (#461) : une ligne par tour dans `wrapper_turn_log`, lue par un
//! résumé quotidien. Que des compteurs, des codes fixes et des identifiants opaques : jamais
//! un texte de message, un argument, une sortie ou une raison d'outil, un secret ni un jeton.
use super::Context;
use crate::{ApiError, ApiResponse, App, ACCOUNT};
use grokbot_engine::{
    contracts::{RunEventKind, ToolCall, ToolCode},
    store,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

empty_hooks!(tools, call_tool, context, entry);
empty_hooks!(state, on_run_event, link_attachments);

const DAY_MS: u64 = 86_400_000;
const MAX_TURNS: i64 = 20_000;
const MAX_TOOLS: usize = 64;
#[derive(Default)]
pub(crate) struct Purge {
    last: Mutex<HashMap<PathBuf, u64>>,
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_turn_log (
        run_id TEXT PRIMARY KEY, at_ms INTEGER NOT NULL, entry TEXT NOT NULL);
    CREATE INDEX IF NOT EXISTS wrapper_turn_log_at ON wrapper_turn_log(at_ms);",
    )?;
    purge_old(conn, crate::now())
}

pub fn tick(ctx: &Context<'_>) {
    let _ = tick_after_probe(
        ctx.connection,
        ctx.db_path,
        ctx.now_ms,
        &ctx.provider.turn_purge,
        |_| {},
    );
}

pub(super) fn tick_after_probe(
    conn: &Connection,
    path: &Path,
    now: u64,
    purge: &Purge,
    after_probe: impl FnOnce(bool),
) -> Result<(), ApiError> {
    let last = purge
        .last
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .copied()
        .unwrap_or(0);
    if purge_due(last, now) && purge_old_after_probe(conn, now, after_probe)? > 0 {
        purge
            .last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.to_path_buf(), now);
    }
    Ok(())
}

fn purge_due(last: u64, now: u64) -> bool {
    now.saturating_sub(last) >= DAY_MS
}

fn purge_old(conn: &Connection, now: u64) -> Result<(), ApiError> {
    purge_old_after_probe(conn, now, |_| {}).map(|_| ())
}

pub(super) const PURGE_WORK: &str =
    "SELECT EXISTS(SELECT 1 FROM wrapper_turn_log WHERE at_ms < ?1)";

pub(super) fn purge_old_after_probe(
    conn: &Connection,
    now: u64,
    after_probe: impl FnOnce(bool),
) -> Result<usize, ApiError> {
    let cutoff = now.saturating_sub(30 * DAY_MS) as i64;
    let available: bool = conn.query_row(PURGE_WORK, [cutoff], |r| r.get(0))?;
    after_probe(available);
    if !available {
        return Ok(0);
    }
    Ok(conn.execute("DELETE FROM wrapper_turn_log WHERE at_ms < ?1", [cutoff])?)
}

/// Un nom venu du modèle n'entre dans le journal que sous une forme sûre ; sinon `-`.
fn safe_name(name: &str, code: ToolCode) -> &str {
    let plain = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    // Un appel jamais validé (inconnu ou réglé sans exécution) peut porter n'importe quel nom.
    if plain && !matches!(code, ToolCode::UnknownTool | ToolCode::NotExecuted) {
        name
    } else {
        "-"
    }
}

/// Résultat affiché d'un appel ; le code fixe reste la clé de regroupement.
fn verdict(code: ToolCode) -> &'static str {
    match code {
        ToolCode::Ok => "ok",
        ToolCode::Failed | ToolCode::Uncertain => "erreur",
        ToolCode::InvalidArguments
        | ToolCode::UnknownTool
        | ToolCode::FollowUpPending
        | ToolCode::ApprovalPending => "refusé",
        ToolCode::ApprovalDenied => "approbation refusée",
        ToolCode::NotExecuted => "annulé",
    }
}

fn tools_of(conn: &Connection, run_id: &str) -> Result<Vec<Value>, ApiError> {
    let (mut names, mut tools) = (HashMap::new(), Vec::new());
    for e in store::events(conn, run_id, 0)? {
        let payload = &e.payload;
        if e.kind == RunEventKind::ToolCall {
            names.extend(
                serde_json::from_value::<ToolCall>(payload.clone()).map(|c| (c.call_id, c.name)),
            );
        } else if e.kind == RunEventKind::ToolResult && tools.len() < MAX_TOOLS {
            let coded = serde_json::from_value::<ToolCode>(payload["code"].clone()).ok();
            // Un résultat d'avant le journal n'a pas de code : on le déduit de son statut,
            // sans garder un nom dont rien ne prouve qu'il a été validé.
            let code = coded.unwrap_or(if payload["outcome"]["status"] == "failed" {
                ToolCode::Failed
            } else {
                ToolCode::Ok
            });
            let name = payload["call_id"].as_str().and_then(|id| names.get(id));
            tools.push(json!({
                "name": safe_name(name.filter(|_| coded.is_some()).map_or("", String::as_str), code),
                "started_ms": payload["started_ms"],
                "duration_ms": payload["elapsed_ms"],
                "result": verdict(code),
                "code": code,
            }));
        }
    }
    Ok(tools)
}

/// Écrit l'entrée d'un tour terminé. À appeler après le commit du tour, hors transaction :
/// l'appelant ignore l'erreur, un journal en panne ne change jamais l'issue d'un tour.
pub(crate) fn record(conn: &Connection, run_id: &str) -> Result<(), ApiError> {
    let at_ms = crate::now() as i64;
    let run = store::required(conn, run_id)?;
    let usage = store::budget_usage(conn, run_id, at_ms)?;
    let (source, model, effort, error_kind): (String, String, Option<String>, Option<String>) =
        conn.query_row(
            &format!(
                "SELECT {}, w.model, w.effort, w.error_kind FROM engine_runs r
                 JOIN wrapper_engine_runs w ON w.run_id=r.run_id
                 LEFT JOIN wrapper_conversations c ON c.account_id=?2 AND c.id=r.conversation_id
                 WHERE r.run_id=?1",
                super::usage::SOURCE_OF_RUN
            ),
            rusqlite::params![run_id, ACCOUNT],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let entry = json!({
        "at_ms": at_ms, "bot_id": run.bot_id, "source": source,
        "turn": crate::upstream::turn_digest(run_id), "model": model, "effort": effort,
        "status": run.status, "error_kind": error_kind, "duration_ms": usage.elapsed_ms,
        "model_calls": usage.model_calls, "tool_calls": usage.tool_calls,
        "tokens": store::token_usage(conn, run_id)?.map(|t| json!({"input": t.input_tokens, "output": t.output_tokens, "cached": t.cached_input_tokens})),
        "tools": tools_of(conn, run_id)?,
    });
    conn.execute(
        "INSERT OR IGNORE INTO wrapper_turn_log(run_id,at_ms,entry) VALUES (?1,?2,?3)",
        rusqlite::params![run_id, at_ms, entry.to_string()],
    )?;
    conn.execute(
        "DELETE FROM wrapper_turn_log WHERE run_id IN
         (SELECT run_id FROM wrapper_turn_log ORDER BY at_ms DESC, run_id LIMIT -1 OFFSET ?1)",
        [MAX_TURNS],
    )?;
    Ok(())
}

/// Rang le plus proche : `sorted[ceil(n * p / 100) - 1]`, `sorted` croissant et non vide.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    sorted[(sorted.len() * p).div_ceil(100).max(1) - 1]
}

/// Résumé d'une journée en heure locale du système : SQLite convertit les bornes en UTC.
pub fn summary(conn: &Connection, day: &str) -> Result<Value, ApiError> {
    let bounds: (Option<bool>, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT date(?1) = ?1, CAST(strftime('%s', ?1, 'utc') AS INTEGER) * 1000,
                CAST(strftime('%s', ?1, '+1 day', 'utc') AS INTEGER) * 1000",
        [day],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let (Some(true), Some(start), Some(end)) = bounds else {
        return Err(ApiError::new(400, "Le jour doit s’écrire AAAA-MM-JJ."));
    };
    let (turns, unmeasured, input, output, cached): (u64, u64, u64, u64, u64) = conn.query_row(
        "SELECT count(*), COALESCE(sum(json_extract(entry,'$.tokens') IS NULL),0),
                COALESCE(sum(json_extract(entry,'$.tokens.input')),0),
                COALESCE(sum(json_extract(entry,'$.tokens.output')),0),
                COALESCE(sum(json_extract(entry,'$.tokens.cached')),0)
         FROM wrapper_turn_log WHERE at_ms >= ?1 AND at_ms < ?2",
        [start, end],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let mut per_tool: BTreeMap<String, Vec<Option<u64>>> = BTreeMap::new();
    let mut errors: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut query = conn.prepare(
        "SELECT json_extract(t.value,'$.name'), json_extract(t.value,'$.code'),
                json_extract(t.value,'$.duration_ms')
         FROM wrapper_turn_log, json_each(entry,'$.tools') t WHERE at_ms >= ?1 AND at_ms < ?2",
    )?;
    for row in query.query_map([start, end], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
        let (name, code, duration): (String, String, Option<u64>) = row?;
        per_tool.entry(name.clone()).or_default().push(duration);
        if code != "ok" {
            *errors.entry((name, code)).or_default() += 1;
        }
    }
    // Les cartes sont triées par nom : un tri stable sur le compte garde le nom pour départager.
    let mut most_used: Vec<Value> = per_tool
        .iter()
        .map(|(name, calls)| json!({"name":name,"calls":calls.len()}))
        .collect();
    most_used.sort_by_key(|v| Reverse(v["calls"].as_u64()));
    let mut slowest: Vec<Value> = per_tool
        .iter()
        .filter_map(|(name, calls)| {
            let mut sorted: Vec<u64> = calls.iter().flatten().copied().collect();
            sorted.sort_unstable();
            (!sorted.is_empty()).then(|| json!({"name":name,"calls":sorted.len(),"median_ms":percentile(&sorted, 50),"p95_ms":percentile(&sorted, 95)}))
        })
        .collect();
    slowest.sort_by_key(|v| Reverse(v["p95_ms"].as_u64()));
    let mut errors: Vec<Value> = errors
        .into_iter()
        .map(|((name, code), count)| json!({"name":name,"code":code,"count":count}))
        .collect();
    errors.sort_by_key(|v| Reverse(v["count"].as_u64()));
    Ok(json!({
        "day": day,
        "turns": turns,
        "tokens": {"input":input,"output":output,"cached":cached,"unmeasured_turns":unmeasured},
        "most_used": most_used,
        "slowest": slowest,
        "errors": errors,
    }))
}

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    _: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    if (method, route) != ("GET", "/api/turns/summary") {
        return None;
    }
    Some(
        query
            .strip_prefix("day=")
            .ok_or_else(|| ApiError::new(400, "Indiquez seulement day=AAAA-MM-JJ."))
            .and_then(|day| summary(&app.connection, day))
            .map(ApiResponse::Json),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        let sorted = [5, 20, 30, 40, 50];
        assert_eq!((percentile(&sorted, 50), percentile(&sorted, 95)), (30, 50));
        assert_eq!(percentile(&[7], 95), 7);
        let hundred: Vec<u64> = (1..=100).collect();
        assert_eq!(
            (percentile(&hundred, 50), percentile(&hundred, 95)),
            (50, 95)
        );
    }

    #[test]
    fn the_daily_purge_waits_for_a_full_day() {
        assert!(purge_due(0, DAY_MS));
        assert!(!purge_due(DAY_MS, 2 * DAY_MS - 1));
        assert!(purge_due(DAY_MS, 2 * DAY_MS));
        assert!(!purge_due(0, 0));
    }

    #[test]
    fn a_model_chosen_name_is_kept_only_when_plain_and_known() {
        assert_eq!(
            safe_name("mcp_caisse.lire-1", ToolCode::Ok),
            "mcp_caisse.lire-1"
        );
        assert_eq!(safe_name("outil secret", ToolCode::Ok), "-");
        assert_eq!(safe_name("é", ToolCode::Ok), "-");
        assert_eq!(safe_name(&"a".repeat(65), ToolCode::Ok), "-");
        assert_eq!(safe_name("a", ToolCode::UnknownTool), "-");
        assert_eq!(safe_name("a", ToolCode::NotExecuted), "-");
        assert_eq!(safe_name("", ToolCode::Ok), "-");
    }
}
