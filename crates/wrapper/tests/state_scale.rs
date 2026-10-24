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

/// VM instructions of one `/api/state` on the app connection, every statement included.
fn state_steps(app: &mut App) -> i64 {
    let steps = Steps::on(app);
    // The first read also clears what cached statements counted before the trace.
    app.api("GET", "/api/state", b"").unwrap();
    steps.reset();
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
