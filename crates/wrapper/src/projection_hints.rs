//! What the side tables hold for the whole history, read once for a projection (#515).
//!
//! `/api/state` projects every message of the history on each poll, and a reply that streams is
//! polled every 100 ms. Each message used to cost about sixteen lookups, one statement each: its
//! mentions, demonstration, attachments of four kinds, meeting card, the run it started, that
//! run's events, errors, usage, sources and recalled facts. One statement per table, read before
//! the projection, now gives the answer for every message: the lookups of a message are replaced
//! by a read of these maps, and those of a message the tables hold nothing for are skipped.
//!
//! A map answers a lookup only for what it covers. A table that cannot be read, a run it does
//! not hold (a task or a routine, not started by a user message) or a row that does not decode
//! leaves the lookup to the statement it replaced, which reports its own error. Outside a
//! projection, nothing is held and every lookup runs.
//!
//! The maps are read after the state revision is captured, like the projection itself: a row
//! stored meanwhile is missing from both and changes the revision, so the next poll shows it.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use grokbot_engine::contracts::{RunEvent, TokenUsage};
use grokbot_engine::model::Run;
use rusqlite::{Connection, Row};
use serde_json::Value;

/// `(conversation, message)`.
type Key = (String, String);
/// `(error, error_kind, notice)` of a run.
pub(crate) type RunMetadata = (Option<String>, Option<String>, Option<String>);

/// What a map knows of a key: its row, no row, or nothing (the lookup must run).
pub(crate) enum Known<T> {
    Row(T),
    Absent,
    Unknown,
}

#[derive(Default)]
struct Hints {
    /// `mentions` stored for a message.
    mentions: Option<HashMap<Key, String>>,
    /// `(ordinal, created_at)` of a submission.
    submissions: Option<HashMap<Key, (i64, u64)>>,
    demonstrations: Option<HashSet<Key>>,
    /// Messages with a staged image, a document, a voice memo or a meeting.
    attachments: Option<HashSet<Key>>,
    meetings: Option<HashSet<Key>>,
    run_sources: Option<HashSet<String>>,
    recalled: Option<HashSet<String>>,
    /// Record of the latest attempt of every run a user message started, by `trigger_ref`.
    user_runs: Option<HashMap<String, String>>,
    /// Events of the user-message runs that have some.
    events: Option<HashMap<String, Rc<Vec<RunEvent>>>>,
    run_metadata: Option<HashMap<String, RunMetadata>>,
    /// Token counts of the runs that reported some.
    usage: Option<HashMap<String, TokenUsage>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Hints>>> = const { RefCell::new(None) };
}

/// The maps of the projection running on this thread, until dropped. A projection inside
/// another gets its own, and the outer maps are back when it ends.
pub(crate) struct Held {
    outer: Option<Rc<Hints>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        ACTIVE.with(|active| *active.borrow_mut() = self.outer.take());
    }
}

/// Rows of `sql` as `(first column, read(row))`; `None` if the table cannot be read.
fn rows<K, V>(
    connection: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
    key: impl Fn(&Row<'_>) -> rusqlite::Result<K>,
    value: impl Fn(&Row<'_>) -> rusqlite::Result<V>,
) -> Option<Vec<(K, V)>> {
    let mut statement = connection.prepare(sql).ok()?;
    let rows = statement
        .query_map(params, |row| Ok((key(row)?, value(row)?)))
        .ok()?;
    rows.collect::<rusqlite::Result<_>>().ok()
}

fn pairs(connection: &Connection, sql: &str) -> Option<HashSet<Key>> {
    let both = |row: &Row<'_>| Ok((row.get(0)?, row.get(1)?));
    Some(
        rows(connection, sql, &[], both, |_| Ok(()))?
            .into_iter()
            .map(|(key, ())| key)
            .collect(),
    )
}

fn runs(connection: &Connection, sql: &str) -> Option<HashSet<String>> {
    Some(
        rows(connection, sql, &[], |row| row.get(0), |_| Ok(()))?
            .into_iter()
            .map(|(run, ())| run)
            .collect(),
    )
}

fn by_run<V>(
    connection: &Connection,
    sql: &str,
    value: impl Fn(&Row<'_>) -> rusqlite::Result<V>,
) -> Option<HashMap<String, V>> {
    Some(
        rows(connection, sql, &[], |row| row.get(0), value)?
            .into_iter()
            .collect(),
    )
}

/// The events of every run a user message started. `None` if one cannot be decoded:
/// `store::events` then reports it for the run it belongs to.
fn events(connection: &Connection) -> Option<HashMap<String, Rc<Vec<RunEvent>>>> {
    let all = rows(
        connection,
        // Only what the projection shows: the latest attempt of each message, in a conversation
        // that is not deleted. Any other run is read by `store::events` if it is asked for.
        "SELECT e.run_id, e.seq, e.kind, e.payload FROM engine_run_events e
         JOIN engine_runs r ON r.run_id = e.run_id
         JOIN wrapper_conversations c ON c.account_id = ?1 AND c.id = r.conversation_id AND c.deleted = 0
         WHERE r.trigger_kind = 'user_message'
           AND r.attempt = (SELECT MAX(m.attempt) FROM engine_runs m
                            WHERE m.trigger_kind = 'user_message' AND m.trigger_ref = r.trigger_ref)
         ORDER BY e.run_id, e.seq",
        &[&crate::ACCOUNT],
        |row| row.get::<_, String>(0),
        |row| {
            Ok((
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    let mut runs: HashMap<String, Vec<RunEvent>> = HashMap::new();
    for (run_id, (seq, kind, payload)) in all {
        runs.entry(run_id.clone()).or_default().push(RunEvent {
            run_id,
            seq,
            // The decoding of `store::events`.
            kind: serde_json::from_value(Value::String(kind)).ok()?,
            payload: serde_json::from_str(&payload).ok()?,
        });
    }
    Some(
        runs.into_iter()
            .map(|(run, list)| (run, Rc::new(list)))
            .collect(),
    )
}

/// Read the maps of the whole history and hold them on this thread.
pub(crate) fn hold(connection: &Connection) -> Held {
    let hints = Hints {
        mentions: rows(
            connection,
            "SELECT conversation_id, message_id, mentions FROM wrapper_group_inputs WHERE account_id = ?1",
            &[&crate::ACCOUNT],
            |row| Ok((row.get(0)?, row.get(1)?)),
            |row| row.get(2),
        )
        .map(|rows| rows.into_iter().collect()),
        submissions: rows(
            connection,
            "SELECT conversation_id, message_id, ordinal, created_at FROM wrapper_messages WHERE account_id = ?1 AND kind = 'submission'",
            &[&crate::ACCOUNT],
            |row| Ok((row.get(0)?, row.get(1)?)),
            |row| Ok((row.get(2)?, row.get(3)?)),
        )
        .map(|rows| rows.into_iter().collect()),
        demonstrations: pairs(
            connection,
            "SELECT conversation_id, message_id FROM wrapper_demonstrations WHERE message_id IS NOT NULL",
        ),
        meetings: pairs(
            connection,
            "SELECT conversation_id, message_id FROM wrapper_meetings WHERE message_id IS NOT NULL",
        ),
        attachments: attachments(connection),
        run_sources: runs(connection, "SELECT DISTINCT run_id FROM wrapper_run_sources"),
        recalled: runs(connection, "SELECT DISTINCT run_id FROM memory_recalled"),
        user_runs: by_run(
            connection,
            // Ascending attempts: the last row of a reference is its latest attempt.
            "SELECT trigger_ref, record FROM engine_runs WHERE trigger_kind = 'user_message' ORDER BY trigger_ref, attempt",
            |row| row.get(1),
        ),
        events: events(connection),
        run_metadata: by_run(
            connection,
            "SELECT run_id, error, error_kind, notice FROM wrapper_engine_runs",
            |row| Ok((row.get(1)?, row.get(2)?, row.get(3)?)),
        ),
        usage: by_run(
            connection,
            "SELECT run_id, SUM(input_tokens), SUM(output_tokens), SUM(cached_input_tokens)
             FROM engine_response_usage GROUP BY run_id",
            |row| {
                Ok(TokenUsage {
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cached_input_tokens: row.get(3)?,
                })
            },
        ),
    };
    let outer = ACTIVE.with(|active| active.borrow_mut().replace(Rc::new(hints)));
    Held { outer }
}

/// What `attachments::for_message` gathers: staged images, documents, voice memos, meetings.
fn attachments(connection: &Connection) -> Option<HashSet<Key>> {
    let mut all = HashSet::new();
    for sql in [
        "SELECT conversation_id, message_id FROM attachments_message_links",
        "SELECT conversation_id, message_id FROM attachments_document_links",
        "SELECT conversation_id, message_id FROM wrapper_voice_memos WHERE message_id IS NOT NULL",
        "SELECT conversation_id, message_id FROM wrapper_meetings WHERE message_id IS NOT NULL",
    ] {
        all.extend(pairs(connection, sql)?);
    }
    Some(all)
}

fn key(conversation: &str, message: &str) -> Key {
    (conversation.to_owned(), message.to_owned())
}

fn held<T>(pick: impl FnOnce(&Hints) -> T) -> Option<T> {
    ACTIVE.with(|active| active.borrow().as_deref().map(pick))
}

/// The set is held and does not have the key: the lookup would find nothing.
fn lacks<K: Hash + Eq>(set: &Option<HashSet<K>>, key: &K) -> bool {
    set.as_ref().is_some_and(|set| !set.contains(key))
}

fn lookup<K: Hash + Eq, V: Clone>(map: &Option<HashMap<K, V>>, key: &K) -> Known<V> {
    match map {
        None => Known::Unknown,
        Some(map) => map.get(key).cloned().map_or(Known::Absent, Known::Row),
    }
}

/// The `mentions` stored for a message.
pub(crate) fn mentions(conversation: &str, message: &str) -> Known<String> {
    held(|h| lookup(&h.mentions, &key(conversation, message))).unwrap_or(Known::Unknown)
}

/// `(ordinal, created_at)` of a submission.
pub(crate) fn submission(conversation: &str, message: &str) -> Known<(i64, u64)> {
    held(|h| lookup(&h.submissions, &key(conversation, message))).unwrap_or(Known::Unknown)
}

/// The message is known to have no demonstration.
pub(crate) fn no_demonstration(conversation: &str, message: &str) -> bool {
    held(|h| lacks(&h.demonstrations, &key(conversation, message))).unwrap_or(false)
}

/// The message is known to have no attachment of any kind.
pub(crate) fn no_attachments(conversation: &str, message: &str) -> bool {
    held(|h| lacks(&h.attachments, &key(conversation, message))).unwrap_or(false)
}

/// The message is known not to be a meeting's summary.
pub(crate) fn no_meeting(conversation: &str, message: &str) -> bool {
    held(|h| lacks(&h.meetings, &key(conversation, message))).unwrap_or(false)
}

/// The run is known to have no source.
pub(crate) fn no_sources(run_id: &str) -> bool {
    held(|h| lacks(&h.run_sources, &run_id.to_owned())).unwrap_or(false)
}

/// The run is known to have recalled no fact.
pub(crate) fn no_recalled(run_id: &str) -> bool {
    held(|h| lacks(&h.recalled, &run_id.to_owned())).unwrap_or(false)
}

/// The latest run a user message started, from its `trigger_ref` (`{conversation}/{message}`).
pub(crate) fn user_run(reference: &str) -> Known<Run> {
    match held(|h| lookup(&h.user_runs, &reference.to_owned())) {
        Some(Known::Row(record)) => {
            serde_json::from_str(&record).map_or(Known::Unknown, Known::Row)
        }
        Some(Known::Absent) => Known::Absent,
        Some(Known::Unknown) | None => Known::Unknown,
    }
}

/// Events of a user-message run, in order; `None` when the caller must ask the database (a run
/// without event yet, or one a user message did not start).
pub(crate) fn events_of(run_id: &str) -> Option<Rc<Vec<RunEvent>>> {
    match held(|h| lookup(&h.events, &run_id.to_owned()))? {
        Known::Row(events) => Some(events),
        Known::Absent | Known::Unknown => None,
    }
}

/// `(error, error_kind, notice)` of a run; `None` when the caller must ask the database.
pub(crate) fn run_metadata(run_id: &str) -> Option<RunMetadata> {
    match held(|h| lookup(&h.run_metadata, &run_id.to_owned()))? {
        Known::Row(metadata) => Some(metadata),
        Known::Absent | Known::Unknown => None,
    }
}

/// Token counts a run reported; `Absent` when it reported none.
pub(crate) fn usage(run_id: &str) -> Known<TokenUsage> {
    held(|h| lookup(&h.usage, &run_id.to_owned())).unwrap_or(Known::Unknown)
}
