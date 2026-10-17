//! Human input, approvals and secrets outside the transcript. Scaffold operations
//! fail closed; migrations are initially no-ops. Secret values stay in the vault.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod approvals;
pub mod redact;
pub mod requests;
pub mod vault;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    requests::migrate(conn)?;
    approvals::migrate(conn)
}
