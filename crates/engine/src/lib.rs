//! Local run engine and shared contracts. Implementation modules are scaffolded:
//! migrations are initially no-ops and operations return `Error::NotImplemented`.
//! The wrapper owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod compaction;
pub mod context;
pub mod contracts;
mod loop_guard;
pub mod model;
pub mod run_loop;
pub mod schema;
pub mod store;

pub fn migrate(conn: &rusqlite::Connection) -> contracts::Result<()> {
    store::migrate(conn)?;
    compaction::migrate(conn)
}
