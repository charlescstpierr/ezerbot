//! Private and local-team facts, targeted recall and versioned derived profiles.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod facts;
pub mod profile;
pub mod promotion;
pub mod recall;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    facts::migrate(conn)?;
    promotion::migrate(conn)?;
    profile::migrate(conn)
}
