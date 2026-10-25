//! Cost of the two reads that look at the `quiet` flag of old routine passes (#286, rule 23):
//! the notification outbox projection, read on every `GET /api/notifications`, and the history of
//! a routine. Ten times the old passes, silent and not, must not multiply the SQLite work.
use grokbot_wrapper::App;
use rusqlite::{ffi, Connection, StatementStatus};
use serde_json::{json, Value};
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicI64, Ordering};

/// SQLITE_TRACE_PROFILE fires when a statement run ends: add its `StatementStatus::VmStep`
/// and reset it, so a cached statement counts each run once (same measure as `state_scale.rs`).
unsafe extern "C" fn add_steps(
    _event: c_uint,
    total: *mut c_void,
    statement: *mut c_void,
    _elapsed: *mut c_void,
) -> c_int {
    let steps = ffi::sqlite3_stmt_status(statement.cast(), StatementStatus::VmStep as c_int, 1);
    (*total.cast::<AtomicI64>()).fetch_add(i64::from(steps), Ordering::Relaxed);
    0
}

struct Fixture {
    app: App,
    db: Connection,
    routine: String,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    /// A routine with `passes` old occurrences: every third failed, half of the others quiet.
    /// Returns the fixture and how many of them must notify.
    fn new(passes: usize) -> (Self, i64) {
        let dir = grokbot_test_support::TestDir::new("routine-quiet-scale");
        let path = dir.to_path_buf().join("app.sqlite");
        let mut app = App::open(&path).unwrap();
        let made = app
            .api(
                "POST",
                "/api/routines",
                json!({"bot_id":"grokbot","name":"Veille","prompt":"Surveille","schedule":"@every 1m"})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        let routine = made["routine"]["routine_id"].as_str().unwrap().to_owned();
        let db = Connection::open(&path).unwrap();
        let mut notifying = 0;
        let tx = db.unchecked_transaction().unwrap();
        for i in 0..passes {
            let (status, quiet) = match (i % 3 == 0, i % 2 == 0) {
                (true, _) => ("failed", false),
                (false, quiet) => ("done", quiet),
            };
            notifying += i64::from(!quiet);
            tx.execute(
                "INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,manual_nonce,status,run_id,reason,quiet)
                 VALUES (?1,?2,1,?3,?1,?4,?5,?6,?7)",
                rusqlite::params![format!("o{i}"), routine, 1000 + i as i64, status, format!("run{i}"), (status == "failed").then_some("échec"), quiet],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        (
            Self {
                app,
                db,
                routine,
                _dir: dir,
            },
            notifying,
        )
    }
    fn get(&mut self) -> Value {
        self.app.api("GET", "/api/notifications", b"").unwrap()
    }
    fn history(&mut self) -> Value {
        self.app
            .api(
                "POST",
                "/api/routines/history",
                json!({"bot_id":"grokbot","routine_id":self.routine})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap()
    }
    /// VM instructions of one call on the app connection, every statement included. The callback
    /// is unregistered before the counter is freed, a panic in `call` included.
    fn steps(&mut self, call: impl FnOnce(&mut Self)) -> i64 {
        struct Trace(*mut ffi::sqlite3, Box<AtomicI64>);
        impl Drop for Trace {
            fn drop(&mut self) {
                let code = unsafe { ffi::sqlite3_trace_v2(self.0, 0, None, std::ptr::null_mut()) };
                if code != ffi::SQLITE_OK {
                    std::process::abort();
                }
            }
        }
        let db = unsafe { self.app.feature_context().connection.handle() };
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
        let trace = Trace(db, total);
        call(self);
        trace.1.load(Ordering::Relaxed)
    }
}

#[test]
fn the_notification_read_costs_no_more_than_ten_times_for_ten_times_the_old_passes() {
    let measure = |passes: usize| {
        let (mut f, notifying) = Fixture::new(passes);
        // The first read projects the passes; the ones after it are the steady state.
        assert_eq!(f.get()["unread"], notifying);
        f.get();
        let steps = f.steps(|f| {
            f.get();
        });
        let quiet: i64 =
            f.db.query_row(
                "SELECT COUNT(*) FROM scheduler_occurrences WHERE quiet=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(quiet > 0 && quiet < passes as i64);
        steps
    };
    let (small, large) = (measure(60), measure(600));
    eprintln!(
        "VmStep GET /api/notifications : 60 passes {small}, 600 passes {large} ({:.2}x)",
        large as f64 / small as f64
    );
    assert!(
        large <= 10 * small,
        "60 passes: {small} VM instructions, 600: {large}"
    );
}

#[test]
fn the_history_of_a_routine_costs_no_more_than_three_times_for_ten_times_the_old_passes() {
    let measure = |passes: usize| {
        let (mut f, _) = Fixture::new(passes);
        f.history();
        let mut shown = 0;
        let steps = f.steps(|f| shown = f.history()["occurrences"].as_array().unwrap().len());
        // A page of 30: the old passes are not read, so ten times more of them stays far under ten times the cost.
        assert_eq!(shown, 30);
        steps
    };
    let (small, large) = (measure(60), measure(600));
    eprintln!(
        "VmStep history : 60 passes {small}, 600 passes {large} ({:.2}x)",
        large as f64 / small as f64
    );
    assert!(
        large <= 3 * small,
        "60 passes: {small} VM instructions, 600: {large}"
    );
}
