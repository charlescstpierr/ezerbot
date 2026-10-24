//! What a routine costs in tokens, never in money: what its passages measurably used,
//! what its schedule promises over 7 days and, when calibration holds, the share of the
//! weekly Codex quota that represents.
//!
//! Returned as `routine.cost` (unknown is `null` or an explicit state, never a zero; there
//! is no price or dollar field):
//! `{state, average_tokens, average_sample, tokens_7d,
//!   forecast:{state, reason, runs_next_7d, tokens_per_day, tokens_next_7d},
//!   quota_share:{state, reason, percent}}`
//! - `state`: `measured`, or `no_measure_yet` (no finished, measured passage).
//! - `forecast.state`: `estimated`, `impossible` (`reason`: `event_trigger`,
//!   `schedule_unusable`) or `no_measure_yet`.
//! - `quota_share.state`: `estimated`, or `unknown` (`reason`: `forecast_unavailable`,
//!   `quota_unavailable`, `calibration_weak`, `account_unconfirmed`).
use crate::{ApiError, Provider};
use grokbot_scheduler::{occurrences, routine::Routine};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::OnceCell;

const WEEK_S: u64 = 7 * 86_400;
const WEEK_MS: i64 = WEEK_S as i64 * 1000;
/// Latest finished, measured passages the average is taken over.
const AVERAGED_PASSAGES: u32 = 10;
/// The quota gauge is a rounded percentage: below these, tokens per percent is noise.
const MIN_USED_PERCENT: f64 = 5.0;
const MIN_LOCAL_TOKENS: u64 = 50_000;

/// The signed-in Codex account and since when this app has seen it; the finished passages whose run
/// sent usage, kept as they happen so that the latest of them are found without reading the ones that
/// sent none; indexes that let a routine's runs of the week and the local tokens of a window be
/// found without reading history. A passage enters the list once it is both finished and measured,
/// whichever of the two comes last.
pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS wrapper_routine_cost_account (
    id INTEGER PRIMARY KEY CHECK(id=1), identity TEXT NOT NULL, since INTEGER NOT NULL
); CREATE TABLE IF NOT EXISTS wrapper_measured_passages (
    routine_id TEXT NOT NULL, scheduled_at_utc INTEGER NOT NULL, occurrence_id TEXT NOT NULL,
    PRIMARY KEY(routine_id, scheduled_at_utc DESC, occurrence_id DESC)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS wrapper_measured_passages_done_insert
    AFTER INSERT ON scheduler_occurrences WHEN NEW.status='done'
BEGIN INSERT OR IGNORE INTO wrapper_measured_passages
    SELECT NEW.routine_id, NEW.scheduled_at_utc, NEW.occurrence_id
    WHERE EXISTS (SELECT 1 FROM engine_response_usage WHERE run_id=NEW.run_id); END;
CREATE TRIGGER IF NOT EXISTS wrapper_measured_passages_done_update
    AFTER UPDATE OF status ON scheduler_occurrences WHEN NEW.status='done' AND OLD.status!='done'
BEGIN INSERT OR IGNORE INTO wrapper_measured_passages
    SELECT NEW.routine_id, NEW.scheduled_at_utc, NEW.occurrence_id
    WHERE EXISTS (SELECT 1 FROM engine_response_usage WHERE run_id=NEW.run_id); END;
CREATE TRIGGER IF NOT EXISTS wrapper_measured_passages_usage
    AFTER INSERT ON engine_response_usage
BEGIN INSERT OR IGNORE INTO wrapper_measured_passages
    SELECT routine_id, scheduled_at_utc, occurrence_id FROM scheduler_occurrences
    WHERE run_id=NEW.run_id AND status='done'; END;
CREATE INDEX IF NOT EXISTS wrapper_engine_runs_started
    ON engine_runs(json_extract(record,'$.started_at'));
CREATE INDEX IF NOT EXISTS wrapper_engine_runs_thread_started
    ON engine_runs(conversation_id, json_extract(record,'$.started_at'));";

/// Creates the tables, triggers and indexes of [`SCHEMA`]; the list of measured passages is filled
/// from the history once, in the same transaction that creates it.
pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='wrapper_measured_passages')",
        [],
        |r| r.get(0),
    )?;
    conn.execute_batch("SAVEPOINT routine_cost_schema")?;
    let created = conn.execute_batch(SCHEMA).and_then(|()| {
        if exists {
            return Ok(());
        }
        conn.execute_batch(
            "INSERT OR IGNORE INTO wrapper_measured_passages
             SELECT routine_id, scheduled_at_utc, occurrence_id FROM scheduler_occurrences
             WHERE status='done' AND run_id IN (SELECT run_id FROM engine_response_usage)",
        )
    });
    conn.execute_batch(if created.is_ok() {
        "RELEASE routine_cost_schema"
    } else {
        "ROLLBACK TO routine_cost_schema; RELEASE routine_cost_schema"
    })?;
    Ok(created?)
}

/// A passage is its run plus every run below it in the delegation graph, each counted once.
/// `?1` routine, `?2` start of the week (in ms), `?3` passages averaged, `?4` the routine's thread.
/// The averaged passages are the latest of the routine's measured list; the week's are those whose
/// run started in it, whatever slot they were owed for. Both reads go through indexes, so old
/// history is not read.
const PASSAGES: &str = "WITH RECURSIVE recent(id) AS (
        SELECT occurrence_id FROM wrapper_measured_passages WHERE routine_id=?1
        ORDER BY scheduled_at_utc DESC, occurrence_id DESC LIMIT ?3),
     tree(occurrence_id, root, run_id) AS (
        SELECT o.occurrence_id, o.run_id, o.run_id FROM engine_runs r
        JOIN scheduler_occurrences o ON o.run_id=r.run_id
        WHERE r.conversation_id=?4 AND json_extract(r.record,'$.started_at')>=?2
          AND o.routine_id=?1
        UNION
        SELECT occurrence_id, run_id, run_id FROM scheduler_occurrences
        WHERE occurrence_id IN recent
        UNION
        SELECT t.occurrence_id, t.root, d.child_run_id FROM tree t
        JOIN delegation_graph d ON d.parent_run_id=t.run_id
        WHERE d.child_run_id IS NOT NULL)
     SELECT t.occurrence_id IN recent, json_extract(r.record,'$.started_at'),
            SUM(u.input_tokens+u.output_tokens)
     FROM tree t
     JOIN engine_runs r ON r.run_id=t.root
     LEFT JOIN engine_response_usage u ON u.run_id=t.run_id
     GROUP BY t.occurrence_id";
/// `?1`: start of the weekly window, in ms.
const LOCAL_TOKENS: &str = "SELECT COALESCE(SUM(u.input_tokens+u.output_tokens),0)
     FROM engine_runs r JOIN engine_response_usage u ON u.run_id=r.run_id
     WHERE json_extract(r.record,'$.started_at')>=?1";

/// Tokens per percent of the weekly Codex window, or why it cannot be known.
enum Calibration {
    TokensPerPercent(f64),
    Unknown(&'static str),
}

/// What the passages measured: `week` is `None` when no run of the week sent usage.
struct Measured {
    average: Option<u64>,
    sample: u64,
    week: Option<u64>,
    week_passages: u64,
}

pub(super) struct Costing<'a> {
    now_ms: i64,
    provider: &'a Provider,
    /// Since when the tick or a request has seen the signed-in Codex account (now, if it has not).
    account_since: i64,
    /// Read once, and only if a routine has a forecast to turn into a share.
    calibration: OnceCell<Calibration>,
}

impl<'a> Costing<'a> {
    pub(super) fn new(
        conn: &Connection,
        provider: &'a Provider,
        now_ms: u64,
    ) -> Result<Self, ApiError> {
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        Ok(Self {
            now_ms,
            provider,
            account_since: account_since(conn, &provider.demonstration_identity(), now_ms)?,
            calibration: OnceCell::new(),
        })
    }

    fn calibration(&self, conn: &Connection) -> Result<&Calibration, ApiError> {
        if self.calibration.get().is_none() {
            // The same source as the Usage panel.
            let calibration =
                calibrate(conn, &self.provider.codex_usage(false), self.account_since)?;
            let _ = self.calibration.set(calibration);
        }
        Ok(self.calibration.get().expect("calibration just set"))
    }

    pub(super) fn of(&self, conn: &Connection, routine: &Routine) -> Result<Value, ApiError> {
        let Measured {
            average,
            sample,
            week,
            week_passages,
        } = self.measured(conn, routine)?;
        let Some(average) = average else {
            return Ok(json!({
                "state":"no_measure_yet","average_tokens":null,"average_sample":0,"tokens_7d":week,
                "forecast":{"state":"no_measure_yet","reason":null,"runs_next_7d":null,"tokens_per_day":null,"tokens_next_7d":null},
                "quota_share":{"state":"unknown","reason":"forecast_unavailable","percent":null},
            }));
        };
        let (forecast, tokens_next_7d) = match self.runs_next_7d(conn, routine) {
            Ok(runs) => {
                let tokens = average * runs;
                (
                    json!({"state":"estimated","reason":null,"runs_next_7d":runs,
                        "tokens_per_day":(tokens + 3) / 7,"tokens_next_7d":tokens}),
                    Some(tokens),
                )
            }
            Err(reason) => (
                json!({"state":"impossible","reason":reason,"runs_next_7d":null,"tokens_per_day":null,"tokens_next_7d":null}),
                None,
            ),
        };
        let quota_share = match tokens_next_7d {
            None => json!({"state":"unknown","reason":"forecast_unavailable","percent":null}),
            Some(tokens) => match self.calibration(conn)? {
                Calibration::Unknown(reason) => {
                    json!({"state":"unknown","reason":reason,"percent":null})
                }
                Calibration::TokensPerPercent(per_percent) => {
                    json!({"state":"estimated","reason":null,"percent":(tokens as f64 / per_percent * 10.0).round() / 10.0})
                }
            },
        };
        Ok(json!({
            "state":"measured","average_tokens":average,"average_sample":sample,"tokens_7d":week.or((week_passages == 0).then_some(0)),
            "forecast":forecast,"quota_share":quota_share,
        }))
    }

    /// Average of the latest finished, measured passages and the tokens of the week's. A passage
    /// whose run sent no usage is unknown, not zero.
    fn measured(&self, conn: &Connection, routine: &Routine) -> Result<Measured, ApiError> {
        let mut query = conn.prepare(PASSAGES)?;
        let since = self.now_ms.saturating_sub(WEEK_MS);
        let rows = query.query_map(
            rusqlite::params![
                routine.routine_id,
                since,
                AVERAGED_PASSAGES,
                routine.conversation_id
            ],
            |r| {
                Ok((
                    r.get::<_, bool>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<u64>>(2)?,
                ))
            },
        )?;
        let (mut recent, mut sum, mut week, mut week_passages) = (0_u64, 0_u64, None::<u64>, 0_u64);
        for row in rows {
            let (is_recent, started, tokens) = row?;
            if is_recent {
                recent += 1;
                sum += tokens.unwrap_or(0);
            }
            if started >= since {
                week_passages += 1;
                week = tokens.map(|tokens| week.unwrap_or(0) + tokens).or(week);
            }
        }
        Ok(Measured {
            average: (recent > 0).then(|| (sum + recent / 2) / recent),
            sample: recent,
            week,
            week_passages,
        })
    }

    /// Passages the scheduler plans in the next 7 days: its own `next_after` walk on each trigger.
    fn runs_next_7d(&self, conn: &Connection, routine: &Routine) -> Result<u64, &'static str> {
        if routine.trigger.has_delivered() {
            return Err("event_trigger");
        }
        occurrences::planned(conn, routine, self.now_ms, WEEK_MS)
            .ok()
            .flatten()
            .ok_or("schedule_unusable")
    }
}

/// Since when `identity` has been the signed-in account, as the tick or a request noted it; now if it is not the
/// one noted. A read never writes: the state and the routine list must stay read-only.
fn account_since(conn: &Connection, identity: &str, now_ms: i64) -> Result<i64, ApiError> {
    let known: Option<(String, i64)> = conn
        .query_row(
            "SELECT identity, since FROM wrapper_routine_cost_account WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match known {
        Some((known, since)) if known == identity => since,
        _ => now_ms,
    })
}

/// Called by every tick, window or not, and by every model request as it is sent: a different account than the one noted dates its sign-in now,
/// so another account's runs never calibrate this one's quota. Writes only on a change.
pub(super) fn note_account(conn: &Connection, identity: &str, now_ms: i64) -> Result<(), ApiError> {
    if account_since(conn, identity, now_ms)? == now_ms {
        conn.execute(
            "INSERT OR REPLACE INTO wrapper_routine_cost_account(id,identity,since) VALUES (1,?1,?2)",
            rusqlite::params![identity, now_ms],
        )?;
    }
    Ok(())
}

/// Codex only reports windows as percentages. Dividing the tokens measured locally since the
/// weekly window began by its `used_percent` gives tokens per percent. An estimate: the window
/// is shared with other Codex apps and auxiliary calls are not measured here, which makes the
/// denominator too small and the share too high; the history of another account would make it
/// too low, so the share is only given once this account has been the signed-in one since the
/// window began (the tick, and each model request, note the account they see).
fn calibrate(
    conn: &Connection,
    quota: &Value,
    account_since: i64,
) -> Result<Calibration, ApiError> {
    let week = ["primary", "secondary"]
        .into_iter()
        .map(|name| &quota["quota"][name])
        .find(|w| w["limit_window_seconds"].as_u64() == Some(WEEK_S));
    let (Some(used), Some(reset)) = (
        week.and_then(|w| w["used_percent"].as_f64()),
        week.and_then(|w| w["reset_at"].as_u64()),
    ) else {
        return Ok(Calibration::Unknown("quota_unavailable"));
    };
    let began = i64::try_from(reset.saturating_sub(WEEK_S) * 1000).unwrap_or(i64::MAX);
    let local: u64 = conn.query_row(LOCAL_TOKENS, [began], |r| r.get(0))?;
    Ok(if used < MIN_USED_PERCENT || local < MIN_LOCAL_TOKENS {
        Calibration::Unknown("calibration_weak")
    } else if account_since > began {
        Calibration::Unknown("account_unconfirmed")
    } else {
        Calibration::TokensPerPercent(local as f64 / used)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    /// A read must not grow with old history: every table the two queries touch is searched
    /// through an index, none is scanned.
    #[test]
    fn reads_search_by_index_and_never_scan_history() {
        let conn = Connection::open_in_memory().unwrap();
        grokbot_engine::store::migrate(&conn).unwrap();
        grokbot_delegation::graph::migrate(&conn).unwrap();
        grokbot_scheduler::migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        for (sql, params) in [(PASSAGES, 4), (LOCAL_TOKENS, 1)] {
            let mut plan = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let bound: Vec<&dyn rusqlite::ToSql> = vec![&0_i64; params];
            let details: Vec<String> = plan
                .query_map(&bound[..], |r| r.get(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            // Only the recursive tree is scanned.
            let scans: Vec<_> = details
                .iter()
                .filter(|d| d.starts_with("SCAN") && d.as_str() != "SCAN t")
                .collect();
            assert!(scans.is_empty(), "{scans:?} in {details:?}");
        }
    }

    const NOW: i64 = 2_000_000_000_000;
    /// Written threshold for the whole read of one routine, instructions.
    const READ_STEPS: u64 = 5_000;

    /// Routine `r1`, every 30 minutes, whose cursor is just behind `NOW`.
    fn create_routine(conn: &Connection) {
        grokbot_scheduler::routine::create(
            conn,
            &Routine {
                routine_id: "r1".into(),
                bot_id: "b".into(),
                conversation_id: "c1".into(),
                name: "R".into(),
                body: grokbot_scheduler::routine::RoutineBody::InlinePrompt { prompt: "p".into() },
                trigger: grokbot_scheduler::cron::Schedule::Every {
                    interval_ms: 1_800_000,
                    phase_ms: None,
                },
                time_zone: "UTC".into(),
                enabled: true,
                origin: grokbot_scheduler::routine::Origin::User,
                untrusted: false,
                schedule_version: 0,
                modified_at: 0,
                last_served_at: None,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE scheduler_routines SET materialized_after=?1",
            [NOW - 1_800_000],
        )
        .unwrap();
    }

    /// Five recent passages, each with a subtask, and `old` finished and `old` missed ones long ago.
    fn history(old: i64) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        grokbot_engine::store::migrate(&conn).unwrap();
        grokbot_delegation::graph::migrate(&conn).unwrap();
        grokbot_scheduler::migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        create_routine(&conn);
        let counted = "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i+1 FROM n WHERE i<?1-1)";
        for (prefix, at, count) in [("old", 1_000_000_i64, old), ("new", NOW - 3_600_000, 5)] {
            conn.execute(
                &format!("{counted} INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record)
                 SELECT ?2||i,'b','c1','occurrence',?2||i,0,'finished',json_object('started_at',?3+i*1000) FROM n"),
                rusqlite::params![count, prefix, at],
            ).unwrap();
            conn.execute(
                &format!("{counted} INSERT INTO engine_response_usage(run_id,response,input_tokens,output_tokens,cached_input_tokens)
                 SELECT ?2||i,1,10,5,0 FROM n"),
                rusqlite::params![count, prefix],
            ).unwrap();
            conn.execute(
                &format!("{counted} INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,run_id,slot_key)
                 SELECT ?2||i,'r1',1,?3+i*1000,'done',?2||i,?2||i FROM n"),
                rusqlite::params![count, prefix, at],
            ).unwrap();
        }
        conn.execute(
            &format!("{counted} INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,slot_key)
             SELECT 'missed'||i,'r1',1,500+i*1000,'missed','missed'||i FROM n"),
            [old],
        ).unwrap();
        conn.execute(
            "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record)
             SELECT 'child'||i,'b','c2','delegation','child'||i,0,'finished',json_object('started_at',?1) FROM (SELECT 0 i UNION SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4)",
            [NOW - 3_600_000],
        ).unwrap();
        conn.execute(
            "INSERT INTO engine_response_usage SELECT run_id,1,10,5,0 FROM engine_runs WHERE run_id LIKE 'child%'",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO delegation_graph(id,parent_run_id,root_run_id,parent_bot,child_bot,depth,child_run_id,conversation_id,selected_context,status)
             SELECT 'edge'||i,'new'||i,'new'||i,'b','b',1,'child'||i,NULL,'{}','{}' FROM (SELECT 0 i UNION SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4)",
            [],
        ).unwrap();
        conn
    }

    /// The measured list holds a passage once it is finished and measured, whichever comes last,
    /// and is filled from the history when it is first created.
    #[test]
    fn a_passage_is_listed_once_it_is_finished_and_measured_in_either_order() {
        let conn = Connection::open_in_memory().unwrap();
        grokbot_engine::store::migrate(&conn).unwrap();
        grokbot_delegation::graph::migrate(&conn).unwrap();
        grokbot_scheduler::migrate(&conn).unwrap();
        create_routine(&conn);
        let run = |id: &str| {
            conn.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES (?1,'b','c1','occurrence',?1,0,'finished','{}')", [id]).unwrap();
        };
        let occurrence = |id: &str, status: &str| {
            conn.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,run_id,slot_key) VALUES (?1,'r1',1,1000,?2,?1,?1)", [id, status]).unwrap();
        };
        let usage = |id: &str| {
            conn.execute(
                "INSERT INTO engine_response_usage VALUES (?1,1,10,5,0)",
                [id],
            )
            .unwrap();
        };
        let listed = || -> Vec<String> {
            conn.prepare(
                "SELECT occurrence_id FROM wrapper_measured_passages ORDER BY occurrence_id",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
        };
        // History from before the list existed: finished and measured, finished only, measured only.
        for (id, status, measured) in [
            ("old-ok", "done", true),
            ("old-blind", "done", false),
            ("old-running", "running", true),
        ] {
            run(id);
            occurrence(id, status);
            if measured {
                usage(id);
            }
        }
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(listed(), ["old-ok"]);
        // Finished, then measured.
        run("late-usage");
        occurrence("late-usage", "done");
        assert_eq!(listed(), ["old-ok"]);
        usage("late-usage");
        // Measured, then finished.
        conn.execute(
            "UPDATE scheduler_occurrences SET status='done' WHERE occurrence_id='old-running'",
            [],
        )
        .unwrap();
        run("late-done");
        occurrence("late-done", "running");
        usage("late-done");
        assert_eq!(listed(), ["late-usage", "old-ok", "old-running"]);
        conn.execute(
            "UPDATE scheduler_occurrences SET status='done' WHERE occurrence_id='late-done'",
            [],
        )
        .unwrap();
        assert_eq!(
            listed(),
            ["late-done", "late-usage", "old-ok", "old-running"]
        );
    }

    /// SQLite instructions the whole read of a routine's cost takes (`Costing::new` and
    /// `Costing::of`: the account, the passages, the window's tokens, the schedule's cursors),
    /// counted one by one by a progress handler on the connection.
    /// `blind`: no passage sent usage, so there is nothing to average.
    fn read_steps(old: i64, blind: bool) -> u64 {
        let conn = history(old);
        if blind {
            conn.execute_batch(
                "DELETE FROM engine_response_usage; DELETE FROM wrapper_measured_passages",
            )
            .unwrap();
        }
        let provider = Provider::new(std::path::Path::new("routine-cost-scale.sqlite"));
        let steps = Arc::new(AtomicU64::new(0));
        let counted = steps.clone();
        conn.progress_handler(
            1,
            Some(move || {
                counted.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        let costing = Costing::new(&conn, &provider, NOW as u64).unwrap();
        // 20 % of the weekly window used: the quota gauge is the only input not read from the database.
        let reset = NOW as u64 / 1000 + 3 * 86_400;
        let quota = json!({"quota":{"primary":{"limit_window_seconds":WEEK_S,"used_percent":20.0,"reset_at":reset}}});
        let _ = costing
            .calibration
            .set(calibrate(&conn, &quota, 0).unwrap());
        let routine = grokbot_scheduler::routine::get(&conn, "r1")
            .unwrap()
            .unwrap();
        let cost = costing.of(&conn, &routine).unwrap();
        if blind {
            assert_eq!(cost["state"], "no_measure_yet", "{cost}");
        } else {
            assert_eq!(cost["state"], "measured", "{cost}");
            assert_eq!(cost["forecast"]["state"], "estimated", "{cost}");
            assert_eq!(cost["quota_share"]["reason"], "calibration_weak", "{cost}");
        }
        steps.load(Ordering::Relaxed)
    }

    /// A routine's cost is read on every GET: ten times the old history must not cost ten times
    /// the work, whether its passages sent usage or not. The whole read stays under `READ_STEPS`.
    #[test]
    fn old_history_ten_times_larger_costs_about_the_same_for_the_whole_read() {
        for blind in [false, true] {
            let (read, read_10x) = (read_steps(1500, blind), read_steps(15_000, blind));
            eprintln!("VMSTEPS whole read, blind={blind}: {read} -> {read_10x}");
            assert!(read_10x <= read + read / 10, "{read} -> {read_10x}");
            assert!(read_10x < READ_STEPS, "{read_10x}");
        }
    }
}
