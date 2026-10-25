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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};

empty_hooks!(
    state,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

const DAY_MS: u64 = 86_400_000;
const RETENTION_MS: u64 = 30 * DAY_MS;
const MAX_TURNS: i64 = 20_000;
const MAX_TOOLS: usize = 64;
const MAX_NAME_BYTES: usize = 64;
static LAST_PURGE: AtomicU64 = AtomicU64::new(0);

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

#[derive(Serialize, Deserialize)]
struct Tokens {
    input: u64,
    output: u64,
    cached: u64,
}

#[derive(Serialize, Deserialize)]
struct Tool {
    name: String,
    started_ms: Option<i64>,
    duration_ms: Option<u64>,
    result: String,
    code: ToolCode,
}

#[derive(Serialize, Deserialize)]
struct Turn {
    at_ms: i64,
    bot_id: String,
    source: String,
    turn: String,
    model: String,
    effort: Option<String>,
    status: String,
    turn_end: Option<String>,
    error_kind: Option<String>,
    duration_ms: u64,
    model_calls: u32,
    tool_calls: u32,
    tokens: Option<Tokens>,
    tools: Vec<Tool>,
}

pub fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wrapper_turn_log (
        run_id TEXT PRIMARY KEY, at_ms INTEGER NOT NULL, entry TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS wrapper_turn_log_at ON wrapper_turn_log(at_ms);",
    )?;
    purge_old(conn, crate::now())
}

pub fn tick(ctx: &Context<'_>) {
    if purge_due(LAST_PURGE.load(Ordering::Relaxed), ctx.now_ms) {
        LAST_PURGE.store(ctx.now_ms, Ordering::Relaxed);
        let _ = purge_old(ctx.connection, ctx.now_ms);
    }
}

fn purge_due(last: u64, now: u64) -> bool {
    now.saturating_sub(last) >= DAY_MS
}

fn purge_old(conn: &Connection, now: u64) -> Result<(), ApiError> {
    conn.execute(
        "DELETE FROM wrapper_turn_log WHERE at_ms < ?1",
        [now.saturating_sub(RETENTION_MS) as i64],
    )?;
    Ok(())
}

/// Un nom venu du modèle n'entre dans le journal que sous une forme sûre ; sinon `-`.
fn safe_name(name: &str, code: ToolCode) -> String {
    let plain = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    // Un appel jamais validé (inconnu ou réglé sans exécution) peut porter n'importe quel nom.
    if plain && !matches!(code, ToolCode::UnknownTool | ToolCode::NotExecuted) {
        name.to_owned()
    } else {
        "-".into()
    }
}

fn tools_of(conn: &Connection, run_id: &str) -> Result<Vec<Tool>, ApiError> {
    let events = store::events(conn, run_id, 0)?;
    let names: HashMap<String, String> = events
        .iter()
        .filter(|e| e.kind == RunEventKind::ToolCall)
        .filter_map(|e| serde_json::from_value::<ToolCall>(e.payload.clone()).ok())
        .map(|call| (call.call_id, call.name))
        .collect();
    Ok(events
        .iter()
        .filter(|e| e.kind == RunEventKind::ToolResult)
        .take(MAX_TOOLS)
        .map(|e| {
            let code = serde_json::from_value(e.payload["code"].clone()).unwrap_or_else(|_| {
                // Un résultat d'avant le journal n'a pas de code : on le déduit de son statut.
                if e.payload["outcome"]["status"] == "failed" {
                    ToolCode::Failed
                } else {
                    ToolCode::Ok
                }
            });
            let name = e.payload["call_id"]
                .as_str()
                .and_then(|id| names.get(id))
                .map_or("", String::as_str);
            Tool {
                name: safe_name(name, code),
                started_ms: e.payload["started_ms"].as_i64(),
                duration_ms: e.payload["elapsed_ms"].as_u64(),
                result: verdict(code).into(),
                code,
            }
        })
        .collect())
}

fn snake<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
}

fn build(conn: &Connection, run_id: &str, at_ms: i64) -> Result<Turn, ApiError> {
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
    Ok(Turn {
        at_ms,
        bot_id: run.bot_id,
        source,
        turn: crate::upstream::turn_digest(run_id),
        model,
        effort,
        status: snake(&run.status).unwrap_or_default(),
        turn_end: run.turn_end.and_then(|end| snake(&end)),
        error_kind,
        duration_ms: usage.elapsed_ms,
        model_calls: usage.model_calls,
        tool_calls: usage.tool_calls,
        tokens: store::token_usage(conn, run_id)?.map(|t| Tokens {
            input: t.input_tokens,
            output: t.output_tokens,
            cached: t.cached_input_tokens,
        }),
        tools: tools_of(conn, run_id)?,
    })
}

/// Écrit l'entrée d'un tour terminé. À appeler après le commit du tour, hors transaction :
/// l'appelant ignore l'erreur, un journal en panne ne change jamais l'issue d'un tour.
pub(crate) fn record(conn: &Connection, run_id: &str) -> Result<(), ApiError> {
    let now = crate::now();
    let turn = build(conn, run_id, now as i64)?;
    let entry = serde_json::to_string(&turn)
        .map_err(|_| ApiError::new(500, "Entrée du journal illisible."))?;
    conn.execute(
        "INSERT OR REPLACE INTO wrapper_turn_log(run_id,at_ms,entry) VALUES (?1,?2,?3)",
        rusqlite::params![run_id, turn.at_ms, entry],
    )?;
    purge_old(conn, now)?;
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

#[derive(Default)]
struct PerTool {
    calls: u64,
    durations: Vec<u64>,
}

fn aggregate(day: &str, turns: impl IntoIterator<Item = Turn>) -> Value {
    let (mut count, mut unmeasured) = (0u64, 0u64);
    let mut tokens = (0u64, 0u64, 0u64);
    let mut per_tool: BTreeMap<String, PerTool> = BTreeMap::new();
    let mut errors: BTreeMap<(String, String), u64> = BTreeMap::new();
    for turn in turns {
        count += 1;
        match turn.tokens {
            Some(t) => {
                tokens.0 += t.input;
                tokens.1 += t.output;
                tokens.2 += t.cached;
            }
            None => unmeasured += 1,
        }
        for tool in turn.tools {
            let entry = per_tool.entry(tool.name.clone()).or_default();
            entry.calls += 1;
            entry.durations.extend(tool.duration_ms);
            if tool.code != ToolCode::Ok {
                let code = snake(&tool.code).unwrap_or_default();
                *errors.entry((tool.name, code)).or_default() += 1;
            }
        }
    }
    let mut most_used: Vec<_> = per_tool.iter().map(|(n, t)| (n, t.calls)).collect();
    most_used.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let mut slowest: Vec<_> = per_tool
        .iter()
        .filter(|(_, t)| !t.durations.is_empty())
        .map(|(name, t)| {
            let mut sorted = t.durations.clone();
            sorted.sort_unstable();
            (
                name,
                sorted.len(),
                percentile(&sorted, 50),
                percentile(&sorted, 95),
            )
        })
        .collect();
    slowest.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(b.0)));
    let mut errors: Vec<_> = errors.into_iter().collect();
    errors.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    json!({
        "day": day,
        "turns": count,
        "tokens": {"input":tokens.0,"output":tokens.1,"cached":tokens.2,"unmeasured_turns":unmeasured},
        "most_used": most_used.iter().map(|(name, calls)| json!({"name":name,"calls":calls})).collect::<Vec<_>>(),
        "slowest": slowest.iter().map(|(name, calls, median, p95)| json!({"name":name,"calls":calls,"median_ms":median,"p95_ms":p95})).collect::<Vec<_>>(),
        "errors": errors.iter().map(|((name, code), count)| json!({"name":name,"code":code,"count":count})).collect::<Vec<_>>(),
    })
}

/// Résumé d'une journée en heure locale du système : SQLite convertit les bornes en UTC.
pub fn summary(conn: &Connection, day: &str) -> Result<Value, ApiError> {
    let invalid = || ApiError::new(400, "Le jour doit s’écrire AAAA-MM-JJ.");
    let valid: Option<bool> = conn.query_row("SELECT date(?1) = ?1", [day], |r| r.get(0))?;
    if valid != Some(true) {
        return Err(invalid());
    }
    let (start, end): (i64, i64) = conn.query_row(
        "SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER) * 1000,
                CAST(strftime('%s', ?1, '+1 day', 'utc') AS INTEGER) * 1000",
        [day],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut query =
        conn.prepare("SELECT entry FROM wrapper_turn_log WHERE at_ms >= ?1 AND at_ms < ?2")?;
    let turns = query
        .query_map([start, end], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|text| serde_json::from_str::<Turn>(&text).ok());
    Ok(aggregate(day, turns))
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
