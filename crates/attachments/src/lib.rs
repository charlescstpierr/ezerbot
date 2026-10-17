//! Image staging and precaption cache. Scaffold operations fail closed;
//! migrations are initially no-ops. File bytes live outside the submission journal.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod precaption;
pub mod staging;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    staging::migrate(conn)?;
    precaption::migrate(conn)
}
