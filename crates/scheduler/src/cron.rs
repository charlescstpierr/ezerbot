//! Schedule parser/next-occurrence signatures (reports/cron-detail.md CR08–CR13).
//! The parser implementation will use jiff for IANA zones, not OS subprocesses.
use grokbot_engine::contracts::{pending, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Cron {
        expression: String,
        time_zone: Option<String>,
    },
    Every {
        interval_ms: u64,
        phase_ms: Option<u64>,
    },
    Once {
        at: i64,
    },
}

pub fn parse(_expression: &str) -> Result<Schedule> {
    pending("scheduler.cron")
}
pub fn next_after(
    _schedule: &Schedule,
    _time_zone: &str,
    _reference_ms: i64,
) -> Result<Option<i64>> {
    pending("scheduler.cron")
}
