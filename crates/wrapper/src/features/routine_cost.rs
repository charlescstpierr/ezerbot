//! Coût en tokens d'une routine, sans écran ni prix : ce que ses passages ont
//! mesurément consommé, ce que son horaire promet pour 7 jours, et, quand
//! l'étalonnage tient, la part du quota hebdomadaire Codex que cela représente.
//!
//! Forme renvoyée sous `routine.cost` (aucun nombre à zéro pour « inconnu », aucun
//! champ en dollars) :
//! `{state, average_tokens, average_sample, tokens_7d,
//!   forecast:{state, reason, runs_next_7d, tokens_per_day, tokens_next_7d},
//!   quota_share:{state, reason, percent}}`
//! - `state` : `measured` ou `no_measure_yet` (aucun passage terminé et mesuré).
//! - `forecast.state` : `estimated`, `impossible` (`reason` : `event_trigger`,
//!   `schedule_unusable`) ou `no_measure_yet`.
//! - `quota_share.state` : `estimated` ou `unknown` (`reason` : `forecast_unavailable`,
//!   `quota_unavailable`, `calibration_weak`).
use crate::ApiError;
use grokbot_scheduler::{cron, routine::Routine};
use rusqlite::Connection;
use serde_json::{json, Value};

const WEEK_S: u64 = 7 * 86_400;
const DAY_MS: i64 = 86_400_000;
const WEEK_MS: i64 = WEEK_S as i64 * 1000;
/// Latest finished, measured passages the average is taken over.
const AVERAGED_PASSAGES: u32 = 10;
/// Passages in a week up to which the week itself is counted (an hourly schedule).
const HOURLY_PER_WEEK: u64 = 168;
/// A schedule planning more passages than this in a day (one per minute) is not forecast.
const MAX_PER_DAY: u64 = 1_440;
/// The quota gauge is a rounded percentage: below these, tokens per percent is noise.
const MIN_USED_PERCENT: f64 = 5.0;
const MIN_LOCAL_TOKENS: u64 = 50_000;

/// Tokens per percent of the weekly Codex window, or why it cannot be known.
enum Calibration {
    TokensPerPercent(f64),
    Unknown(&'static str),
}

pub(super) struct Costing {
    now_ms: i64,
    calibration: Calibration,
}

impl Costing {
    /// `quota` is `provider.codex_usage(false)`, the same source as the Usage panel.
    pub(super) fn new(conn: &Connection, quota: &Value, now_ms: u64) -> Result<Self, ApiError> {
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        Ok(Self {
            now_ms,
            calibration: calibrate(conn, quota)?,
        })
    }

    pub(super) fn of(&self, conn: &Connection, routine: &Routine) -> Result<Value, ApiError> {
        let (average, sample, week) = self.measured(conn, &routine.routine_id)?;
        let Some(average) = average else {
            return Ok(json!({
                "state":"no_measure_yet","average_tokens":null,"average_sample":0,"tokens_7d":null,
                "forecast":{"state":"no_measure_yet","reason":null,"runs_next_7d":null,"tokens_per_day":null,"tokens_next_7d":null},
                "quota_share":{"state":"unknown","reason":"forecast_unavailable","percent":null},
            }));
        };
        let (forecast, tokens_next_7d) = match self.runs_next_7d(routine) {
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
        let quota_share = match (&self.calibration, tokens_next_7d) {
            (_, None) => json!({"state":"unknown","reason":"forecast_unavailable","percent":null}),
            (Calibration::Unknown(reason), _) => {
                json!({"state":"unknown","reason":reason,"percent":null})
            }
            (Calibration::TokensPerPercent(per_percent), Some(tokens)) => {
                json!({"state":"estimated","reason":null,"percent":(tokens as f64 / per_percent * 10.0).round() / 10.0})
            }
        };
        Ok(json!({
            "state":"measured","average_tokens":average,"average_sample":sample,"tokens_7d":week,
            "forecast":forecast,"quota_share":quota_share,
        }))
    }

    /// `(average over the latest finished measured passages, their count, tokens of the last 7 days)`.
    /// A passage is its run plus every run below it in the delegation graph, each counted once;
    /// a passage whose run sent no usage is unknown, not zero.
    fn measured(
        &self,
        conn: &Connection,
        routine_id: &str,
    ) -> Result<(Option<u64>, u64, u64), ApiError> {
        let mut query = conn.prepare(
            "WITH RECURSIVE recent(id) AS (
                SELECT occurrence_id FROM scheduler_occurrences
                WHERE routine_id=?1 AND status='done'
                  AND run_id IN (SELECT run_id FROM engine_response_usage)
                ORDER BY scheduled_at_utc DESC, occurrence_id DESC LIMIT ?3),
             tree(occurrence_id, run_id) AS (
                SELECT o.occurrence_id, o.run_id FROM scheduler_occurrences o
                JOIN engine_runs r ON r.run_id=o.run_id
                WHERE o.routine_id=?1
                  AND (json_extract(r.record,'$.started_at')>=?2 OR o.occurrence_id IN recent)
                UNION
                SELECT t.occurrence_id, d.child_run_id FROM tree t
                JOIN delegation_graph d ON d.parent_run_id=t.run_id
                WHERE d.child_run_id IS NOT NULL)
             SELECT t.occurrence_id IN recent, json_extract(r.record,'$.started_at'),
                    SUM(u.input_tokens+u.output_tokens)
             FROM tree t
             JOIN scheduler_occurrences o ON o.occurrence_id=t.occurrence_id
             JOIN engine_runs r ON r.run_id=o.run_id
             LEFT JOIN engine_response_usage u ON u.run_id=t.run_id
             GROUP BY t.occurrence_id",
        )?;
        let since = self.now_ms.saturating_sub(WEEK_MS);
        let rows = query.query_map(
            rusqlite::params![routine_id, since, AVERAGED_PASSAGES],
            |r| {
                Ok((
                    r.get::<_, bool>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<u64>>(2)?.unwrap_or(0),
                ))
            },
        )?;
        let (mut recent, mut sum, mut week) = (0_u64, 0_u64, 0_u64);
        for row in rows {
            let (is_recent, started, tokens) = row?;
            if is_recent {
                recent += 1;
                sum += tokens;
            }
            if started >= since {
                week += tokens;
            }
        }
        Ok((
            (recent > 0).then(|| (sum + recent / 2) / recent),
            recent,
            week,
        ))
    }

    /// Passages the schedule plans in the next 7 days, counted with the scheduler's own `next_after`.
    /// Hourly or sparser schedules are counted over the week itself, so a weekly or weekday-only
    /// routine is exact. Denser ones repeat each day: one day is counted and multiplied, which keeps
    /// the count to a few hundred `next_after` calls on every poll.
    fn runs_next_7d(&self, routine: &Routine) -> Result<u64, &'static str> {
        if routine.trigger.has_delivered() {
            return Err("event_trigger");
        }
        if let Some(runs) = self.planned(routine, WEEK_MS, HOURLY_PER_WEEK)? {
            return Ok(runs);
        }
        self.planned(routine, DAY_MS, MAX_PER_DAY)?
            .map(|runs| runs * 7)
            .ok_or("schedule_unusable")
    }

    /// Passages planned in `(now, now + span_ms]`, or `None` past `cap`.
    fn planned(
        &self,
        routine: &Routine,
        span_ms: i64,
        cap: u64,
    ) -> Result<Option<u64>, &'static str> {
        let (end, mut at, mut runs) = (self.now_ms + span_ms, self.now_ms, 0);
        while let Some(next) = cron::next_after(&routine.trigger, &routine.time_zone, at)
            .map_err(|_| "schedule_unusable")?
            .filter(|next| *next <= end)
        {
            runs += 1;
            if runs > cap {
                return Ok(None);
            }
            at = next;
        }
        Ok(Some(runs))
    }
}

/// Codex only reports windows as percentages. Dividing the tokens measured locally since the
/// weekly window began by its `used_percent` gives tokens per percent. It is an estimate: other
/// Codex apps share the window and auxiliary calls are not measured here, so it overstates the share.
fn calibrate(conn: &Connection, quota: &Value) -> Result<Calibration, ApiError> {
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
    let local: u64 = conn.query_row(
        "SELECT COALESCE(SUM(u.input_tokens+u.output_tokens),0) FROM engine_response_usage u
         JOIN engine_runs r ON r.run_id=u.run_id WHERE json_extract(r.record,'$.started_at')>=?1",
        [began],
        |r| r.get(0),
    )?;
    Ok(if used < MIN_USED_PERCENT || local < MIN_LOCAL_TOKENS {
        Calibration::Unknown("calibration_weak")
    } else {
        Calibration::TokensPerPercent(local as f64 / used)
    })
}
