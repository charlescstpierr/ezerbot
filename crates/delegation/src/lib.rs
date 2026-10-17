//! Bounded task delegation. Scaffold operations fail closed and migrations are
//! initially no-ops. Child context must always be explicitly selected.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod admission;
pub mod graph;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    graph::migrate(conn)
}
