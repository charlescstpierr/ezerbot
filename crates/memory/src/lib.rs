//! Private and local-team memory. Migrations are initially no-ops and operations
//! return `NotImplemented` until their implementation slice lands.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod facts;
pub mod promotion;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    facts::migrate(conn)?;
    promotion::migrate(conn)
}
