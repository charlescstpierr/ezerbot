use super::{demonstrations, meetings, memory_auto, turns};
use rusqlite::Connection;
use std::time::{Duration, Instant};

const NOW: u64 = 40 * 86_400_000;

#[derive(Clone, Copy, Debug)]
enum Case {
    Memory,
    Meetings,
    Demonstrations,
    Turns,
}
const CASES: [Case; 4] = [
    Case::Memory,
    Case::Meetings,
    Case::Demonstrations,
    Case::Turns,
];

struct Fixture {
    conn: Connection,
    other: Connection,
    path: std::path::PathBuf,
    purge: turns::Purge,
    deadline: Instant,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let dir = grokbot_test_support::TestDir::new("idle-writes");
        let path = dir.join("db.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
            CREATE TABLE memory_auto_jobs(run_id TEXT PRIMARY KEY,state TEXT,updated_at INTEGER);
            CREATE TABLE wrapper_meetings(id TEXT PRIMARY KEY,audio BLOB,expires_at INTEGER);
            CREATE TABLE wrapper_demonstration_images(demonstration_id TEXT PRIMARY KEY,image TEXT);
            CREATE TABLE wrapper_demonstrations(id TEXT PRIMARY KEY,skill_id TEXT,bot_id TEXT);
            CREATE TABLE skills_library(skill_id TEXT PRIMARY KEY,deleted INTEGER);
            CREATE TABLE wrapper_bots(id TEXT PRIMARY KEY,deleted INTEGER);
            INSERT INTO skills_library VALUES('s',1);
            INSERT INTO wrapper_bots VALUES('b',0);
            CREATE TABLE wrapper_turn_log(run_id TEXT PRIMARY KEY,at_ms INTEGER,entry TEXT);",
        )
        .unwrap();
        let other = Connection::open(&path).unwrap();
        for db in [&conn, &other] {
            db.busy_timeout(Duration::from_secs(3)).unwrap();
        }
        Self {
            conn,
            other,
            path,
            purge: Default::default(),
            deadline: Instant::now() + Duration::from_secs(10),
            _dir: dir,
        }
    }
    fn run(
        &self,
        case: Case,
        db: &Connection,
        after: impl FnOnce(bool),
    ) -> Result<Option<String>, crate::ApiError> {
        assert!(Instant::now() < self.deadline);
        let result = match case {
            Case::Memory => memory_auto::claim_job(db, NOW, after),
            Case::Meetings => meetings::purge_after_probe(db, NOW, after).map(|()| None),
            Case::Demonstrations => demonstrations::purge_after_probe(db, after).map(|()| None),
            Case::Turns => {
                turns::tick_after_probe(db, &self.path, NOW, &self.purge, after).map(|()| None)
            }
        };
        assert!(Instant::now() < self.deadline);
        result
    }
}

fn seed(db: &Connection, case: Case, id: &str) {
    match case {
        Case::Memory => {
            db.execute("INSERT INTO memory_auto_jobs VALUES(?1,'queued',1)", [id])
                .unwrap();
        }
        Case::Meetings => {
            db.execute("INSERT INTO wrapper_meetings VALUES(?1,X'01',1)", [id])
                .unwrap();
        }
        Case::Demonstrations => {
            db.execute(
                "INSERT INTO wrapper_demonstrations VALUES(?1,'s','b')",
                [id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO wrapper_demonstration_images VALUES(?1,'image')",
                [id],
            )
            .unwrap();
        }
        Case::Turns => {
            db.execute("INSERT INTO wrapper_turn_log VALUES(?1,1,'{}')", [id])
                .unwrap();
        }
    }
}
fn pending(db: &Connection, case: Case) -> i64 {
    let query = match case {
        Case::Memory => "SELECT COUNT(*) FROM memory_auto_jobs WHERE state='queued'",
        Case::Meetings => "SELECT COUNT(*) FROM wrapper_meetings WHERE audio IS NOT NULL",
        Case::Demonstrations => "SELECT COUNT(*) FROM wrapper_demonstration_images",
        Case::Turns => "SELECT COUNT(*) FROM wrapper_turn_log",
    };
    db.query_row(query, [], |r| r.get(0)).unwrap()
}

#[test]
fn work_after_negative_probe_is_durable_until_the_next_tick() {
    for case in CASES {
        let f = Fixture::new();
        let changes = f.conn.total_changes();
        f.run(case, &f.conn, |available| {
            assert!(!available, "{case:?}");
            seed(&f.other, case, "late");
        })
        .unwrap();
        assert_eq!(f.conn.total_changes(), changes, "{case:?}");
        assert_eq!(pending(&f.other, case), 1, "{case:?}");
        f.run(case, &f.conn, |available| assert!(available))
            .unwrap();
        assert_eq!(pending(&f.other, case), 0, "{case:?}");
    }
}

#[test]
fn positive_probe_preserves_current_predicates_and_queue_order() {
    for case in CASES {
        let f = Fixture::new();
        seed(&f.other, case, "a");
        let claimed = f
            .run(case, &f.conn, |available| {
                assert!(available, "{case:?}");
                seed(&f.other, case, "b");
            })
            .unwrap();
        if let Case::Memory = case {
            assert_eq!(claimed.as_deref(), Some("a"));
            assert_eq!(pending(&f.other, case), 1);
            f.other
                .execute(
                    "UPDATE memory_auto_jobs SET state='done' WHERE run_id='a'",
                    [],
                )
                .unwrap();
            assert_eq!(f.run(case, &f.conn, |_| {}).unwrap().as_deref(), Some("b"));
        }
        assert_eq!(pending(&f.other, case), 0, "{case:?}");
    }
}

#[test]
fn targets_changed_after_probe_are_rechecked_before_mutation() {
    for case in CASES {
        let f = Fixture::new();
        seed(&f.other, case, "a");
        let changes = f.conn.total_changes();
        f.run(case, &f.conn, |available| {
            assert!(available);
            let query = match case {
                Case::Memory => "UPDATE memory_auto_jobs SET state='done'",
                Case::Meetings => "UPDATE wrapper_meetings SET expires_at=9999999999999",
                Case::Demonstrations => "UPDATE skills_library SET deleted=0",
                Case::Turns => "UPDATE wrapper_turn_log SET at_ms=9999999999999",
            };
            f.other.execute(query, []).unwrap();
        })
        .unwrap();
        assert_eq!(f.conn.total_changes(), changes, "{case:?}");
        if !matches!(case, Case::Memory) {
            assert_eq!(pending(&f.other, case), 1, "{case:?}");
        }
    }
}

#[test]
fn competing_purges_do_not_consume_the_same_media_twice() {
    for case in [Case::Meetings, Case::Demonstrations, Case::Turns] {
        let f = Fixture::new();
        seed(&f.other, case, "a");
        let changes = f.conn.total_changes();
        f.run(case, &f.conn, |available| {
            assert!(available);
            f.run(case, &f.other, |other| assert!(other)).unwrap();
        })
        .unwrap();
        assert_eq!(pending(&f.conn, case), 0, "{case:?}");
        assert_eq!(f.conn.total_changes(), changes, "{case:?}");
    }
}

#[test]
fn failed_mutations_roll_back_and_remain_due() {
    for case in CASES {
        let f = Fixture::new();
        seed(&f.other, case, "a");
        if let Case::Memory = case {
            f.other
                .execute(
                    "INSERT INTO memory_auto_jobs VALUES('crash','running',0)",
                    [],
                )
                .unwrap();
        }
        let (event, table) = match case {
            Case::Memory => ("UPDATE", "memory_auto_jobs"),
            Case::Meetings => ("UPDATE", "wrapper_meetings"),
            Case::Demonstrations => ("DELETE", "wrapper_demonstration_images"),
            Case::Turns => ("DELETE", "wrapper_turn_log"),
        };
        let when = if matches!(case, Case::Memory) {
            "WHEN NEW.run_id='a'"
        } else {
            ""
        };
        f.other.execute_batch(&format!("CREATE TRIGGER fail AFTER {event} ON {table} {when} BEGIN SELECT RAISE(ABORT,'forced mutation failure'); END;")).unwrap();
        assert!(
            f.run(case, &f.conn, |available| assert!(available))
                .is_err(),
            "{case:?}"
        );
        assert_eq!(pending(&f.other, case), 1, "{case:?}");
        if let Case::Memory = case {
            assert_eq!(
                f.other
                    .query_row(
                        "SELECT state FROM memory_auto_jobs WHERE run_id='crash'",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                "running"
            );
        }
        f.other.execute_batch("DROP TRIGGER fail").unwrap();
        f.run(case, &f.conn, |available| assert!(available))
            .unwrap();
        assert_eq!(pending(&f.other, case), 0, "{case:?}");
    }
}

#[test]
fn turn_purge_clock_is_per_database_and_only_success_advances_it() {
    let f = Fixture::new();
    let second = Fixture::new();
    seed(&f.conn, Case::Turns, "a");
    turns::tick_after_probe(&f.conn, &f.path, NOW, &f.purge, |_| {}).unwrap();
    seed(&second.conn, Case::Turns, "b");
    turns::tick_after_probe(&second.conn, &second.path, NOW, &f.purge, |_| {}).unwrap();
    assert_eq!(pending(&second.conn, Case::Turns), 0);
    seed(&f.conn, Case::Turns, "next");
    turns::tick_after_probe(&f.conn, &f.path, NOW + 86_400_000 - 1, &f.purge, |_| {
        panic!("daily purge is not due")
    })
    .unwrap();
    assert_eq!(pending(&f.conn, Case::Turns), 1);
    turns::tick_after_probe(&f.conn, &f.path, NOW + 86_400_000, &f.purge, |_| {}).unwrap();
    assert_eq!(pending(&f.conn, Case::Turns), 0);
}

#[test]
fn an_empty_delete_does_not_postpone_newly_arrived_old_turns() {
    let f = Fixture::new();
    seed(&f.other, Case::Turns, "removed");
    f.run(Case::Turns, &f.conn, |available| {
        assert!(available);
        f.other.execute("DELETE FROM wrapper_turn_log", []).unwrap();
    })
    .unwrap();
    seed(&f.other, Case::Turns, "next");
    f.run(Case::Turns, &f.conn, |available| assert!(available))
        .unwrap();
    assert_eq!(pending(&f.other, Case::Turns), 0);
}

#[test]
fn recovery_includes_running_jobs_but_a_live_worker_keeps_its_claim() {
    let f = Fixture::new();
    f.conn
        .execute(
            "INSERT INTO memory_auto_jobs VALUES('crash','running',0)",
            [],
        )
        .unwrap();
    assert!(
        memory_auto::claim_job(&f.conn, NOW, |available| assert!(available))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.conn
            .query_row("SELECT state FROM memory_auto_jobs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "done"
    );
    let expiry = crate::now() / 1000 + 3600;
    std::fs::write(f.path.with_extension("chatgpt-auth.json"), serde_json::json!({"host_id":"test","client_id":"test","access_token":"fake-access","refresh_token":"fake-refresh","expires_at":expiry,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    let provider = crate::Provider::with_endpoints(
        &f.path,
        crate::Endpoints {
            api: "http://127.0.0.1:1".into(),
            ..Default::default()
        },
        false,
    );
    assert!(provider.connected());
    let ctx = super::Context {
        connection: &f.conn,
        provider: &provider,
        db_path: &f.path,
        now_ms: NOW,
        bot_id: None,
        conversation_id: None,
        run_id: None,
    };
    assert!(provider.claim_run("memory-auto-worker").is_some());
    seed(&f.other, Case::Memory, "live");
    assert_eq!(
        memory_auto::claim_job(&f.conn, NOW, |available| {
            assert!(available);
            memory_auto::tick(&ctx);
            assert_eq!(pending(&f.other, Case::Memory), 1);
        })
        .unwrap()
        .as_deref(),
        Some("live")
    );
    memory_auto::tick(&ctx);
    assert_eq!(
        f.other
            .query_row(
                "SELECT state FROM memory_auto_jobs WHERE run_id='live'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    provider.release_run("memory-auto-worker");
    f.conn
        .execute("UPDATE memory_auto_jobs SET state='done'", [])
        .unwrap();
    memory_auto::tick(&ctx);
    assert!(provider.claim_run("memory-auto-worker").is_some());
    provider.release_run("memory-auto-worker");
}

#[test]
fn negative_probes_have_at_most_linear_history_cost() {
    use rusqlite::StatementStatus::VmStep;
    for case in CASES {
        let f = Fixture::new();
        f.conn
            .execute_batch(
                "UPDATE skills_library SET deleted=0;
            CREATE INDEX wrapper_turn_log_at ON wrapper_turn_log(at_ms);",
            )
            .unwrap();
        let query = match case {
            Case::Memory => memory_auto::TICK_WORK,
            Case::Meetings => meetings::PURGE_WORK,
            Case::Demonstrations => demonstrations::PURGE_WORK,
            Case::Turns => turns::PURGE_WORK,
        };
        let mut costs = Vec::new();
        for batch in 0..2 {
            for i in 0..100 {
                let id = format!("{batch}-{i}");
                seed(&f.conn, case, &id);
            }
            match case {
                Case::Memory => {
                    f.conn
                        .execute("UPDATE memory_auto_jobs SET state='done'", [])
                        .unwrap();
                }
                Case::Meetings => {
                    f.conn
                        .execute("UPDATE wrapper_meetings SET expires_at=?1", [NOW + 1])
                        .unwrap();
                }
                Case::Turns => {
                    f.conn
                        .execute("UPDATE wrapper_turn_log SET at_ms=?1", [NOW])
                        .unwrap();
                }
                Case::Demonstrations => {}
            }
            let mut statement = f.conn.prepare(query).unwrap();
            if statement.parameter_count() == 1 {
                let cutoff = if matches!(case, Case::Turns) {
                    NOW - 30 * 86_400_000
                } else {
                    NOW
                };
                assert!(!statement
                    .query_row([cutoff], |r| r.get::<_, bool>(0))
                    .unwrap());
            } else {
                assert!(!statement.query_row([], |r| r.get::<_, bool>(0)).unwrap());
            }
            costs.push(statement.get_status(VmStep));
            let mut plan = f
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
                .unwrap();
            let args: &[u64] = if plan.parameter_count() == 0 {
                &[]
            } else {
                &[NOW]
            };
            let details = plan
                .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            println!("{case:?} negative probe plan {details:?}");
        }
        println!(
            "{case:?} negative probe VmStep 100={} 200={}",
            costs[0], costs[1]
        );
        assert!(
            costs[1] as f64 <= costs[0] as f64 * 2.1,
            "{case:?} {costs:?}"
        );
    }
}
