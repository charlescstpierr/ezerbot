//! Cost of a state read as the history grows (#377). `/api/state` replays every message of
//! the history on each poll: its SQLite work must stay proportional to that history, never
//! grow faster.
use grokbot_wrapper::App;
use rusqlite::{ffi, StatementStatus};
use serde_json::json;
use std::cell::Cell;
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

/// The queries a state read runs once per message of the history (#377).
const PER_MESSAGE: [&str; 6] = [
    "SELECT ordinal, created_at FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND kind = 'submission'",
    "SELECT record FROM engine_runs WHERE trigger_kind=?1 AND trigger_ref=?2
         ORDER BY attempt DESC LIMIT 1",
    "SELECT seq,kind,payload FROM engine_run_events WHERE run_id=?1 AND seq>?2 ORDER BY seq",
    "SELECT n,kind,title,account,author,date,locator,range,excerpt
         FROM wrapper_run_sources WHERE run_id=?1 ORDER BY n",
    "SELECT error,error_kind,notice FROM wrapper_engine_runs WHERE run_id=?1",
    "SELECT SUM(input_tokens),SUM(output_tokens),SUM(cached_input_tokens)
        FROM engine_response_usage WHERE run_id=?1 HAVING COUNT(*)>0",
];

std::thread_local! {
    /// VM instructions of the statements run on this thread, by SQL text: what to read when a
    /// count grows more than it should.
    static BY_SQL: std::cell::RefCell<std::collections::HashMap<String, i64>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
    /// Calls of `add_steps` on this thread, whatever counter they reach: what shows the callback
    /// is gone once its guard is.
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

/// SQLITE_TRACE_PROFILE fires when a statement run ends: add its `StatementStatus::VmStep`
/// and reset it, so a cached statement counts each run once.
unsafe extern "C" fn add_steps(
    _event: c_uint,
    total: *mut c_void,
    statement: *mut c_void,
    _elapsed: *mut c_void,
) -> c_int {
    CALLS.set(CALLS.get() + 1);
    let steps = ffi::sqlite3_stmt_status(statement.cast(), StatementStatus::VmStep as c_int, 1);
    let sql = std::ffi::CStr::from_ptr(ffi::sqlite3_sql(statement.cast()))
        .to_string_lossy()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    BY_SQL.with(|by_sql| {
        *by_sql.borrow_mut().entry(sql).or_insert(0) += i64::from(steps);
    });
    (*total.cast::<AtomicI64>()).fetch_add(i64::from(steps), Ordering::Relaxed);
    0
}

/// Counts the VM instructions of every statement run on the app connection while it lives.
/// SQLite keeps a pointer to the counter: dropping the guard unregisters the callback, and only
/// then are its fields (the counter) freed. That order holds when the measure ends and when a
/// panic unwinds it, before the App is dropped (its destructor still runs queries).
struct Steps {
    db: *mut ffi::sqlite3,
    total: Box<AtomicI64>,
}

impl Steps {
    fn on(app: &App) -> Self {
        let db = unsafe { app.feature_context().connection.handle() };
        let total = Box::new(AtomicI64::new(0));
        let context = &*total as *const AtomicI64 as *mut c_void;
        let code = unsafe {
            ffi::sqlite3_trace_v2(
                db,
                ffi::SQLITE_TRACE_PROFILE as c_uint,
                Some(add_steps),
                context,
            )
        };
        assert_eq!(code, ffi::SQLITE_OK);
        Self { db, total }
    }
    fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
    }
    fn count(&self) -> i64 {
        self.total.load(Ordering::Relaxed)
    }
}

impl Drop for Steps {
    fn drop(&mut self) {
        let code = unsafe { ffi::sqlite3_trace_v2(self.db, 0, None, std::ptr::null_mut()) };
        if code != ffi::SQLITE_OK {
            // Still registered, the callback would reach the counter freed right after this.
            std::process::abort();
        }
    }
}

/// The statements that cost the most instructions since the last call, for a failure message.
fn heaviest() -> String {
    BY_SQL.with(|by_sql| {
        let mut all: Vec<_> = by_sql.borrow_mut().drain().collect();
        all.sort_by_key(|(_, steps)| -steps);
        all.iter()
            .take(6)
            .map(|(sql, steps)| format!("{steps} {}", sql.chars().take(140).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// VM instructions of one `/api/state` on the app connection, every statement included.
fn state_steps(app: &mut App) -> i64 {
    let steps = Steps::on(app);
    // The first read also clears what cached statements counted before the trace.
    app.api("GET", "/api/state", b"").unwrap();
    steps.reset();
    BY_SQL.with(|by_sql| by_sql.borrow_mut().clear());
    let state = app.api("GET", "/api/state", b"").unwrap();
    let total = steps.count();
    drop(steps);
    assert!(
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
    total
}

fn history(messages: usize) -> (App, std::path::PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ezerbot-state-scale-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = App::open(&dir.join("app.sqlite")).unwrap();
    let chat = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for turn in 0..messages {
        let body = json!({"conversation_id": chat, "message_id": format!("ancien-{turn}"), "body": "Historique"});
        app.api("POST", "/api/send", body.to_string().as_bytes())
            .unwrap();
    }
    (app, dir)
}

#[test]
fn a_state_read_costs_no_more_than_ten_times_for_ten_times_the_history() {
    let (mut small_app, small_dir) = history(20);
    let small = state_steps(&mut small_app);
    let (mut large_app, large_dir) = history(200);
    let large = state_steps(&mut large_app);
    let messages = |app: &mut App| {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .len()
    };
    let (shown_small, shown_large) = (messages(&mut small_app), messages(&mut large_app));
    // The plans of the per-message queries, published with the PR (rule 23).
    let connection = large_app.feature_context().connection;
    for sql in PER_MESSAGE {
        let mut plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let count = plan.parameter_count();
        let details: Vec<String> = plan
            .query_map(rusqlite::params_from_iter(vec![""; count]), |row| {
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
        // Each is a lookup by key, never a scan of the history.
        assert!(
            details.iter().all(|step| !step.starts_with("SCAN")),
            "{sql}: {details:?}"
        );
    }
    eprintln!(
        "VmStep /api/state : {shown_small} messages {small}, {shown_large} messages {large} ({:.2}x)",
        large as f64 / small as f64
    );
    // Ten times the history replays ten times the messages: linear at most, so never more
    // than ten times the instructions (fixed costs keep the real ratio below that).
    assert!(
        large <= 10 * small,
        "20 messages sent: {small} VM instructions, 200: {large}"
    );
    drop((small_app, large_app));
    let _ = std::fs::remove_dir_all(small_dir);
    let _ = std::fs::remove_dir_all(large_dir);
}

/// Statements run on the app connection by one `/api/state`. `CALLS` counts the end of each.
fn state_statements(app: &mut App) -> usize {
    let steps = Steps::on(app);
    // The first read also fills the cache of prepared statements.
    app.api("GET", "/api/state", b"").unwrap();
    let before = CALLS.get();
    app.api("GET", "/api/state", b"").unwrap();
    let ran = CALLS.get() - before;
    drop(steps);
    ran
}

#[test]
fn a_state_read_runs_the_same_statements_whatever_the_length_of_the_history() {
    // Before #515 every message of the history cost sixteen lookups on each poll: the
    // projection read the side tables (mentions, attachments, meeting, sources, events, usage...)
    // message by message, and a reply that streams is polled every 100 ms. The tables are now
    // read once for the whole history: a message adds rows to read, not statements to run.
    let (mut small_app, small_dir) = history(20);
    let small = state_statements(&mut small_app);
    let (mut large_app, large_dir) = history(200);
    let large = state_statements(&mut large_app);
    eprintln!(
        "STATEMENTS /api/state : 20 messages sent {small}, 200 messages sent {large} ({:+} for 180 more)",
        large as i64 - small as i64
    );
    // One lookup per message more would be 180 statements more.
    assert!(
        large <= small + 20,
        "20 messages sent: {small} statements, 200: {large}"
    );
    drop((small_app, large_app));
    let _ = std::fs::remove_dir_all(small_dir);
    let _ = std::fs::remove_dir_all(large_dir);
}

/// A visible history of `visible` messages and a deleted conversation of `deleted` messages, each
/// with a row in the side tables a state read looks into. Meeting cards, voice memos and
/// demonstrations are left out: the state of those features lists its own table, which is not a
/// read of the projection (the plans in `projection_hints.rs` cover their reads).
fn history_with_deleted(visible: usize, deleted: usize) -> (App, std::path::PathBuf) {
    let (mut app, dir) = history(visible);
    let old = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for turn in 0..deleted {
        let body = json!({"conversation_id": old, "message_id": format!("supprime-{turn}"), "body": "Historique"});
        app.api("POST", "/api/send", body.to_string().as_bytes())
            .unwrap();
    }
    {
        let connection = app.feature_context().connection;
        grokbot_attachments::staging::migrate(connection).unwrap();
        // Rows of tables the fixture does not fill: the links point at nothing.
        connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        for sql in [
            "INSERT INTO engine_response_usage(run_id,response,input_tokens,output_tokens,cached_input_tokens) SELECT run_id,1,1,1,0 FROM engine_runs WHERE conversation_id=?1",
            "INSERT INTO wrapper_run_sources(run_id,n,key,kind,title) SELECT run_id,1,'k','web','t' FROM engine_runs WHERE conversation_id=?1",
            "INSERT INTO memory_recalled(run_id,fact_id) SELECT run_id,'fait' FROM engine_runs WHERE conversation_id=?1",
            "INSERT INTO attachments_message_links(conversation_id,message_id,attachment_id,ordinal) SELECT conversation_id,message_id,'a',0 FROM wrapper_messages WHERE conversation_id=?1",
            "INSERT INTO attachments_document_links(conversation_id,message_id,document_id,ordinal) SELECT conversation_id,message_id,'doc',0 FROM wrapper_messages WHERE conversation_id=?1",
            "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) SELECT run_id||'-2',bot_id,conversation_id,trigger_kind,trigger_ref,attempt+1,status,record FROM engine_runs WHERE conversation_id=?1",
        ] {
            connection.execute(sql, [&old]).unwrap();
        }
    }
    app.api(
        "POST",
        "/api/conversations/delete",
        json!({"id": old}).to_string().as_bytes(),
    )
    .unwrap();
    (app, dir)
}

#[test]
fn what_a_deleted_history_holds_costs_a_state_read_nothing() {
    // Rule 23: ten times the deleted history must not load its runs, events, usage, sources or
    // links during a state read of the same visible history. The query plans separately cover
    // memos, meetings and demonstrations, whose feature state is outside this projection.
    let (mut small_app, small_dir) = history_with_deleted(20, 20);
    let small = state_steps(&mut small_app);
    let small_top = heaviest();
    let (mut large_app, large_dir) = history_with_deleted(20, 200);
    let large = state_steps(&mut large_app);
    let large_top = heaviest();
    eprintln!(
        "VmStep /api/state, 20 messages shown : 20 deleted {small}, 200 deleted {large} ({:.2}x)",
        large as f64 / small as f64
    );
    // The projection walks `wrapper_messages` once more for the entries that are not messages
    // (cards): a few instructions for each message of the history, deleted ones included. Reading
    // the side tables of 180 more deleted messages cost 110 instructions each before they were
    // bounded to what is shown.
    assert!(
        large <= small + 8 * 180,
        "20 deleted messages: {small} VM instructions, 200: {large}\n--- 20 deleted\n{small_top}\n--- 200 deleted\n{large_top}"
    );
    drop((small_app, large_app));
    let _ = std::fs::remove_dir_all(small_dir);
    let _ = std::fs::remove_dir_all(large_dir);
}

#[test]
fn older_attempts_of_visible_messages_do_not_multiply_a_state_reads_work() {
    let (mut app, dir) = history(20);
    let add_attempts = |app: &App, attempts: std::ops::RangeInclusive<i64>| {
        let connection = app.feature_context().connection;
        connection.execute_batch("BEGIN").unwrap();
        for attempt in attempts {
            connection.execute(
                "INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record)
                 SELECT run_id||'-retry-'||?1,bot_id,conversation_id,trigger_kind,trigger_ref,?1,status,
                        json_set(record,'$.run_id',run_id||'-retry-'||?1,'$.trigger.attempt',?1)
                 FROM engine_runs WHERE attempt=0", [attempt]).unwrap();
            connection.execute(
                "INSERT INTO wrapper_engine_runs(run_id,input_json,live,model)
                 SELECT w.run_id||'-retry-'||?1,w.input_json,w.live,w.model
                 FROM wrapper_engine_runs w JOIN engine_runs r ON r.run_id=w.run_id WHERE r.attempt=0",
                [attempt]).unwrap();
            connection.execute(
                "INSERT INTO engine_run_events(run_id,seq,kind,payload)
                 SELECT r.run_id||'-retry-'||?1,e.seq,e.kind,e.payload
                 FROM engine_runs r JOIN engine_run_events e ON e.run_id=r.run_id WHERE r.attempt=0",
                [attempt]).unwrap();
            connection.execute(
                "INSERT INTO engine_response_usage(run_id,response,input_tokens,output_tokens,cached_input_tokens)
                 SELECT run_id||'-retry-'||?1,1,1,1,0 FROM engine_runs WHERE attempt=0",
                [attempt]).unwrap();
        }
        connection.execute_batch("COMMIT").unwrap();
    };
    add_attempts(&app, 1..=20);
    let small = state_steps(&mut app);
    let small_top = heaviest();
    add_attempts(&app, 21..=200);
    let large = state_steps(&mut app);
    let large_top = heaviest();
    eprintln!(
        "VmStep /api/state, 20 visible submissions : 20 retries {small}, 200 retries {large}"
    );
    assert!(
        large <= small + 100,
        "older attempts changed the work: {small} to {large}\n--- 20 retries\n{small_top}\n--- 200 retries\n{large_top}"
    );
    let state = app.api("GET", "/api/state", b"").unwrap();
    let replies: Vec<_> = state["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "assistant")
        .collect();
    assert_eq!(replies.len(), 20);
    for reply in replies {
        assert!(reply["payload"]["run_id"]
            .as_str()
            .unwrap()
            .ends_with("-retry-200"));
        assert_eq!(reply["payload"]["usage"]["input_tokens"], 1);
    }
    drop(app);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_panic_during_the_measure_unregisters_the_callback_before_the_app_is_dropped() {
    let (mut app, dir) = history(1);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _steps = Steps::on(&app);
        app.api("GET", "/api/state", b"").unwrap();
        assert!(CALLS.get() > 0, "the callback counts while measuring");
        panic!("a state read fails in the middle of the measure");
    }));
    assert!(unwound.is_err());
    let after = CALLS.get();
    // Same thread, same connection: another read, then the App's destructor, which records
    // the routines' departure with a query. Neither reaches the callback any more.
    app.api("GET", "/api/state", b"").unwrap();
    drop(app);
    assert_eq!(
        CALLS.get(),
        after,
        "SQLite called the callback after its counter was freed"
    );
    let _ = std::fs::remove_dir_all(dir);
}
