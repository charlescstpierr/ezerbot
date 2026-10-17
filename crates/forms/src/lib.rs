//! Human input, approvals and secrets outside the transcript. Unimplemented
//! capabilities fail closed; secret values stay in the vault. The caller owns
//! transactions; rules use the engine's component version helpers, never user_version.
#![forbid(unsafe_code)]

pub mod approvals;
pub mod redact;
pub mod requests;
pub mod rules;
pub mod vault;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    requests::migrate(conn)?;
    approvals::migrate(conn)?;
    rules::migrate(conn)
}
