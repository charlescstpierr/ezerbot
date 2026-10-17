//! Per-bot skills. Implementation slices fill the public modules without changing
//! this registry. Migrations are initially no-ops; operations fail closed.
//! The caller owns transactions. Version schemas via `component_versions`, not `user_version`.
#![forbid(unsafe_code)]

pub mod import;
pub mod library;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    library::migrate(conn)
}
