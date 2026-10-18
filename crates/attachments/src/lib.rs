//! Image staging, precaption cache and local document extraction.
//! File bytes and extracted text live outside the submission journal.
//! The caller owns transactions and `component_versions`; never use `user_version`.
#![forbid(unsafe_code)]

pub mod documents;
pub mod generation;
pub mod precaption;
pub mod staging;

pub fn migrate(conn: &rusqlite::Connection) -> grokbot_engine::contracts::Result<()> {
    staging::migrate(conn)?;
    precaption::migrate(conn)?;
    documents::migrate(conn)
}
