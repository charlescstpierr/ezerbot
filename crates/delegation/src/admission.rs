//! Admission boundary and configurable Q3 defaults, implemented by gb-delegation-core.
use crate::graph::Delegation;
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionLimits {
    pub active_workspace: usize,
    pub active_per_root: usize,
    pub max_depth: u32,
    pub backlog: usize,
}
impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            active_workspace: 4,
            active_per_root: 3,
            max_depth: 3,
            backlog: 16,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Admission {
    Running,
    Queued,
    Refused { reason: String },
}

pub fn admit(
    _conn: &Connection,
    _delegation: &Delegation,
    _limits: &AdmissionLimits,
) -> Result<Admission> {
    pending("delegation.admission")
}
