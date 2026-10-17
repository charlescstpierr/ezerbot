//! Routines and durable occurrences. Scaffold operations return `NotImplemented`;
//! migrations are initially no-ops. Only this capability depends on IANA time support.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod cron;
pub mod occurrences;
pub mod routine;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    routine::migrate(conn)?;
    occurrences::migrate(conn)
}
