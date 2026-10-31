//! What the side tables hold for the history a projection shows, read once for it (#515).
//!
//! `/api/state` projects every message of the history on each poll, and a reply that streams is
//! polled every 100 ms. Each message used to cost about sixteen lookups, one statement each: its
//! mentions, demonstration, attachments of four kinds, meeting card, the run it started, that
//! run's events, errors, usage, sources and recalled facts. One statement per table, read before
//! the projection, now gives the answer for every message: the lookups of a message are replaced
//! by a read of these maps, and those of a message the tables hold nothing for are skipped.
//!
//! Every read is bounded to what the projection shows, never to the whole table: the
//! conversations of the account that are not deleted, and in them the latest attempt of each
//! user message. Each table is entered from those conversations or runs through an index, so
//! what a deleted conversation, an older attempt or another account holds costs nothing.
//!
//! A map answers a lookup only for what it covers. A conversation or a run outside the
//! projection (a recovered conversation, a task or a routine, not started by a user message), a
//! table that cannot be read, or a row that does not decode leaves the lookup to the statement it
//! replaced, which reports its own error. Outside a projection, nothing is held and every lookup
//! runs.
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
    /// The conversations the projection shows: the account's, not deleted.
    conversations: Option<HashSet<String>>,
    /// The runs it shows: the latest attempt of each user message in those conversations.
    runs: Option<HashSet<String>>,
    /// `mentions` stored for a message.
    mentions: Option<HashMap<Key, String>>,
    /// `(ordinal, created_at)` of a submission.
    submissions: Option<HashMap<Key, (i64, u64)>>,
    page_anchors: Option<HashMap<Key, Value>>,
    demonstrations: Option<HashSet<Key>>,
    /// Messages with a staged image, a document, a voice memo or a meeting.
    attachments: Option<HashSet<Key>>,
    meetings: Option<HashSet<Key>>,
    run_sources: Option<HashSet<String>>,
    recalled: Option<HashSet<String>>,
    /// Record of the latest attempt of each run a user message started, by `trigger_ref`.
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

const ACCOUNT_ONLY: &[&dyn rusqlite::ToSql] = &[&crate::ACCOUNT];

fn pairs(connection: &Connection, sql: &str) -> Option<HashSet<Key>> {
    let both = |row: &Row<'_>| Ok((row.get(0)?, row.get(1)?));
    Some(
        rows(connection, sql, ACCOUNT_ONLY, both, |_| Ok(()))?
            .into_iter()
            .map(|(key, ())| key)
            .collect(),
    )
}

fn ids(
    connection: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Option<HashSet<String>> {
    Some(
        rows(connection, sql, params, |row| row.get(0), |_| Ok(()))?
            .into_iter()
            .map(|(id, ())| id)
            .collect(),
    )
}

fn by_run<V>(
    connection: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
    value: impl Fn(&Row<'_>) -> rusqlite::Result<V>,
) -> Option<HashMap<String, V>> {
    Some(
        rows(connection, sql, params, |row| row.get(0), value)?
            .into_iter()
            .collect(),
    )
}

/// `$tail` over `projected`, the latest attempt of each displayed submission. The index on
/// `(trigger_kind, trigger_ref, attempt)` reaches the last attempt without visiting older ones.
/// `CROSS JOIN` keeps the visible conversations and their submissions as the outer loops.
macro_rules! projected {
    ($tail:literal) => {
        concat!(
            "WITH projected AS (
               SELECT r.run_id AS run_id, r.trigger_ref AS trigger_ref, r.record AS record
               FROM wrapper_conversations c
               CROSS JOIN submissions s ON s.account_id = c.account_id AND s.conversation_id = c.id
               CROSS JOIN engine_runs r ON r.run_id = (
                   SELECT m.run_id FROM engine_runs m
                   WHERE m.trigger_kind = 'user_message' AND m.trigger_ref = c.id || '/' || s.message_id
                   ORDER BY m.attempt DESC LIMIT 1)
               WHERE c.account_id = ?1 AND c.deleted = 0) ",
            $tail
        )
    };
}

/// The rows of `table` for the messages of the conversations the projection shows. `table` has
/// `conversation_id` and `message_id` and an index that starts with `conversation_id` (after
/// `account_id` for the tables that have one). `CROSS JOIN` keeps the conversations as the outer
/// loop: the other order would walk the whole table, rows of deleted conversations included.
macro_rules! of_conversations {
    ($columns:literal, $table:literal, $extra:literal) => {
        concat!(
            "SELECT ",
            $columns,
            " FROM wrapper_conversations c CROSS JOIN ",
            $table,
            " t ON t.conversation_id = c.id WHERE c.account_id = ?1 AND c.deleted = 0",
            $extra
        )
    };
}

const SQL_CONVERSATIONS: &str =
    "SELECT id FROM wrapper_conversations WHERE account_id = ?1 AND deleted = 0";
const SQL_RUNS: &str = projected!("SELECT run_id, trigger_ref, record FROM projected");
const SQL_MENTIONS: &str = of_conversations!(
    "t.conversation_id, t.message_id, t.mentions",
    "wrapper_group_inputs",
    " AND t.account_id = ?1"
);
const SQL_SUBMISSIONS: &str = of_conversations!(
    "t.conversation_id, t.message_id, t.ordinal, t.created_at",
    "wrapper_messages",
    " AND t.account_id = ?1 AND t.kind = 'submission'"
);
const SQL_PAGE_ANCHORS: &str =
    "SELECT a.page_id, a.passage, p.title, a.source_version, a.conversation_id, a.message_id
     FROM wrapper_conversations c
     CROSS JOIN wrapper_page_anchors a ON a.conversation_id = c.id
     CROSS JOIN bot_pages p ON p.id = a.page_id
     WHERE c.account_id = ?1 AND c.deleted = 0";
const SQL_DEMONSTRATIONS: &str = of_conversations!(
    "t.conversation_id, t.message_id",
    "wrapper_demonstrations",
    " AND t.message_id IS NOT NULL"
);
const SQL_MEETINGS: &str = of_conversations!(
    "t.conversation_id, t.message_id",
    "wrapper_meetings",
    " AND t.message_id IS NOT NULL"
);
/// What `attachments::for_message` gathers: staged images, documents, voice memos, meetings.
const SQL_ATTACHMENTS: [&str; 4] = [
    of_conversations!(
        "t.conversation_id, t.message_id",
        "attachments_message_links",
        ""
    ),
    of_conversations!(
        "t.conversation_id, t.message_id",
        "attachments_document_links",
        ""
    ),
    of_conversations!(
        "t.conversation_id, t.message_id",
        "wrapper_voice_memos",
        " AND t.message_id IS NOT NULL"
    ),
    SQL_MEETINGS,
];
/// The reads driven by the projected runs: `?1` is the JSON array of their ids, built from the
/// one evaluation of `SQL_RUNS`, so the set is not computed again for each table.
const SQL_SOURCES: &str = "SELECT DISTINCT s.run_id FROM json_each(?1) j JOIN wrapper_run_sources s ON s.run_id = j.value";
const SQL_RECALLED: &str =
    "SELECT DISTINCT s.run_id FROM json_each(?1) j JOIN memory_recalled s ON s.run_id = j.value";
const SQL_EVENTS: &str = "SELECT e.run_id, e.seq, e.kind, e.payload FROM json_each(?1) j
     JOIN engine_run_events e ON e.run_id = j.value ORDER BY e.run_id, e.seq";
const SQL_METADATA: &str = "SELECT w.run_id, w.error, w.error_kind, w.notice FROM json_each(?1) j
     JOIN wrapper_engine_runs w ON w.run_id = j.value";
const SQL_USAGE: &str =
    "SELECT u.run_id, SUM(u.input_tokens), SUM(u.output_tokens), SUM(u.cached_input_tokens)
     FROM json_each(?1) j JOIN engine_response_usage u ON u.run_id = j.value GROUP BY u.run_id";

/// The events of the projected runs. `None` if one cannot be decoded: `store::events` then
/// reports it for the run it belongs to.
fn events(connection: &Connection, run_ids: &str) -> Option<HashMap<String, Rc<Vec<RunEvent>>>> {
    let all = rows(
        connection,
        SQL_EVENTS,
        &[&run_ids],
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

/// Read the maps of the history the projection shows and hold them on this thread.
pub(crate) fn hold(connection: &Connection) -> Held {
    let projected = rows(
        connection,
        SQL_RUNS,
        ACCOUNT_ONLY,
        |row| row.get::<_, String>(0),
        |row| Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
    );
    let run_ids = serde_json::to_string(
        &projected
            .iter()
            .flatten()
            .map(|(run, _)| run.as_str())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_default();
    let by_runs: &[&dyn rusqlite::ToSql] = &[&run_ids];
    let hints = Hints {
        conversations: ids(connection, SQL_CONVERSATIONS, ACCOUNT_ONLY),
        runs: projected
            .as_ref()
            .map(|rows| rows.iter().map(|(run, _)| run.clone()).collect()),
        user_runs: projected.map(|rows| {
            rows.into_iter()
                .map(|(_, (reference, record))| (reference, record))
                .collect()
        }),
        mentions: rows(
            connection,
            SQL_MENTIONS,
            ACCOUNT_ONLY,
            |row| Ok((row.get(0)?, row.get(1)?)),
            |row| row.get(2),
        )
        .map(|rows| rows.into_iter().collect()),
        submissions: rows(
            connection,
            SQL_SUBMISSIONS,
            ACCOUNT_ONLY,
            |row| Ok((row.get(0)?, row.get(1)?)),
            |row| Ok((row.get(2)?, row.get(3)?)),
        )
        .map(|rows| rows.into_iter().collect()),
        page_anchors: rows(
            connection,
            SQL_PAGE_ANCHORS,
            ACCOUNT_ONLY,
            |row| Ok((row.get(4)?, row.get(5)?)),
            |row| {
                Ok(serde_json::json!({
                    "page_id": row.get::<_, String>(0)?,
                    "passage": row.get::<_, String>(1)?,
                    "title": row.get::<_, String>(2)?,
                    "source_version": row.get::<_, String>(3)?,
                }))
            },
        )
        .map(|rows| rows.into_iter().collect()),
        demonstrations: pairs(connection, SQL_DEMONSTRATIONS),
        meetings: pairs(connection, SQL_MEETINGS),
        attachments: attachments(connection),
        run_sources: ids(connection, SQL_SOURCES, by_runs),
        recalled: ids(connection, SQL_RECALLED, by_runs),
        events: events(connection, &run_ids),
        run_metadata: by_run(connection, SQL_METADATA, by_runs, |row| {
            Ok((row.get(1)?, row.get(2)?, row.get(3)?))
        }),
        usage: by_run(connection, SQL_USAGE, by_runs, |row| {
            Ok(TokenUsage {
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cached_input_tokens: row.get(3)?,
            })
        }),
    };
    let outer = ACTIVE.with(|active| active.borrow_mut().replace(Rc::new(hints)));
    Held { outer }
}

fn attachments(connection: &Connection) -> Option<HashSet<Key>> {
    let mut all = HashSet::new();
    for sql in SQL_ATTACHMENTS {
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

fn shows_conversation(hints: &Hints, conversation: &str) -> bool {
    hints
        .conversations
        .as_ref()
        .is_some_and(|set| set.contains(conversation))
}

fn shows_run(hints: &Hints, run_id: &str) -> bool {
    hints.runs.as_ref().is_some_and(|set| set.contains(run_id))
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

/// What a map of messages knows of one: nothing outside the conversations it read.
fn of_message<V: Clone>(
    hints: &Hints,
    map: &Option<HashMap<Key, V>>,
    conversation: &str,
    message: &str,
) -> Known<V> {
    if shows_conversation(hints, conversation) {
        lookup(map, &key(conversation, message))
    } else {
        Known::Unknown
    }
}

fn no_row_for_message(
    hints: &Hints,
    set: &Option<HashSet<Key>>,
    conversation: &str,
    message: &str,
) -> bool {
    shows_conversation(hints, conversation) && lacks(set, &key(conversation, message))
}

fn no_row_for_run(hints: &Hints, set: &Option<HashSet<String>>, run_id: &str) -> bool {
    shows_run(hints, run_id) && lacks(set, &run_id.to_owned())
}

/// The `mentions` stored for a message.
pub(crate) fn mentions(conversation: &str, message: &str) -> Known<String> {
    held(|h| of_message(h, &h.mentions, conversation, message)).unwrap_or(Known::Unknown)
}

/// `(ordinal, created_at)` of a submission.
pub(crate) fn submission(conversation: &str, message: &str) -> Known<(i64, u64)> {
    held(|h| of_message(h, &h.submissions, conversation, message)).unwrap_or(Known::Unknown)
}

pub(crate) fn page_anchor(conversation: &str, message: &str) -> Known<Value> {
    held(|h| of_message(h, &h.page_anchors, conversation, message)).unwrap_or(Known::Unknown)
}

/// The message is known to have no demonstration.
pub(crate) fn no_demonstration(conversation: &str, message: &str) -> bool {
    held(|h| no_row_for_message(h, &h.demonstrations, conversation, message)).unwrap_or(false)
}

/// The message is known to have no attachment of any kind.
pub(crate) fn no_attachments(conversation: &str, message: &str) -> bool {
    held(|h| no_row_for_message(h, &h.attachments, conversation, message)).unwrap_or(false)
}

/// The message is known not to be a meeting's summary.
pub(crate) fn no_meeting(conversation: &str, message: &str) -> bool {
    held(|h| no_row_for_message(h, &h.meetings, conversation, message)).unwrap_or(false)
}

/// The run is known to have no source.
pub(crate) fn no_sources(run_id: &str) -> bool {
    held(|h| no_row_for_run(h, &h.run_sources, run_id)).unwrap_or(false)
}

/// The run is known to have recalled no fact.
pub(crate) fn no_recalled(run_id: &str) -> bool {
    held(|h| no_row_for_run(h, &h.recalled, run_id)).unwrap_or(false)
}

/// The latest run a user message started.
pub(crate) fn user_run(conversation: &str, message: &str) -> Known<Run> {
    let reference = format!("{conversation}/{message}");
    match held(|h| of_run_reference(h, conversation, &reference)) {
        Some(Known::Row(record)) => {
            serde_json::from_str(&record).map_or(Known::Unknown, Known::Row)
        }
        Some(Known::Absent) => Known::Absent,
        Some(Known::Unknown) | None => Known::Unknown,
    }
}

fn of_run_reference(hints: &Hints, conversation: &str, reference: &str) -> Known<String> {
    if shows_conversation(hints, conversation) {
        lookup(&hints.user_runs, &reference.to_owned())
    } else {
        Known::Unknown
    }
}

/// Events of a projected run, in order; `None` when the caller must ask the database (a run
/// without event yet, or one the projection does not show).
pub(crate) fn events_of(run_id: &str) -> Option<Rc<Vec<RunEvent>>> {
    held(|h| match shows_run(h, run_id) {
        true => lookup(&h.events, &run_id.to_owned()),
        false => Known::Unknown,
    })
    .and_then(|known| match known {
        Known::Row(events) => Some(events),
        Known::Absent | Known::Unknown => None,
    })
}

/// `(error, error_kind, notice)` of a projected run; `None` when the caller must ask the
/// database.
pub(crate) fn run_metadata(run_id: &str) -> Option<RunMetadata> {
    held(|h| match shows_run(h, run_id) {
        true => lookup(&h.run_metadata, &run_id.to_owned()),
        false => Known::Unknown,
    })
    .and_then(|known| match known {
        Known::Row(metadata) => Some(metadata),
        Known::Absent | Known::Unknown => None,
    })
}

/// Token counts a projected run reported; `Absent` when it reported none.
pub(crate) fn usage(run_id: &str) -> Known<TokenUsage> {
    held(|h| match shows_run(h, run_id) {
        true => lookup(&h.usage, &run_id.to_owned()),
        false => Known::Unknown,
    })
    .unwrap_or(Known::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every read enters its tables through an index, from the conversations or the runs the
    /// projection shows (rule 23). A scan is allowed only of the conversations of the account
    /// and of the list of projected run ids itself: never of a table that grows with the history, with
    /// what deleted conversations or older attempts hold.
    #[test]
    fn no_read_scans_a_table_that_grows_with_the_history() {
        let directory = grokbot_test_support::TestDir::new("projection-hints");
        let app = crate::App::open(&directory.join("app.sqlite")).unwrap();
        let reads = [
            SQL_CONVERSATIONS,
            SQL_RUNS,
            SQL_MENTIONS,
            SQL_SUBMISSIONS,
            SQL_PAGE_ANCHORS,
            SQL_DEMONSTRATIONS,
            SQL_MEETINGS,
            SQL_ATTACHMENTS[0],
            SQL_ATTACHMENTS[1],
            SQL_ATTACHMENTS[2],
            SQL_SOURCES,
            SQL_RECALLED,
            SQL_EVENTS,
            SQL_METADATA,
            SQL_USAGE,
        ];
        for sql in reads {
            let mut plan = app
                .connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let count = plan.parameter_count();
            let details: Vec<String> = plan
                .query_map(rusqlite::params_from_iter(vec!["x"; count]), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            eprintln!(
                "PLAN {} => {}",
                sql.split_whitespace().collect::<Vec<_>>().join(" "),
                details.join(" | ")
            );
            for step in &details {
                // Entered by the conversation or the run being read, never by what else the
                // table holds (a kind, an account).
                if let Some(searched) = step.strip_prefix("SEARCH ") {
                    let table = searched.split_whitespace().next().unwrap_or_default();
                    assert!(
                        matches!(table, "c" | "wrapper_conversations")
                            || ["conversation_id=?", "run_id=?", "trigger_ref=?", "id=?"]
                                .iter()
                                .any(|key| step.contains(key)),
                        "{sql}: {step}"
                    );
                }
                if let Some(scanned) = step.strip_prefix("SCAN ") {
                    let table = scanned.split_whitespace().next().unwrap_or_default();
                    assert!(
                        matches!(
                            table,
                            "c" | "j" | "p" | "projected" | "wrapper_conversations"
                        ),
                        "{sql}: {step}"
                    );
                }
            }
        }
    }
}
