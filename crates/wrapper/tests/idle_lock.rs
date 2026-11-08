mod support;

use grokbot_wrapper::{features, App, Provider};
use rusqlite::Connection;
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use support::{FakeOpenAi, Seen, CLIENT};

type Tick = fn(&features::Context<'_>);

#[test]
fn idle_hooks_and_app_tick_leave_the_writer_lock_alone() {
    let dir = grokbot_test_support::TestDir::new("idle-lock");
    let path = dir.join("app.sqlite");
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    std::fs::write(
        path.with_extension("chatgpt-auth.json"),
        json!({
            "host_id":"test", "client_id":CLIENT,
            "access_token":"fake-access", "refresh_token":"fake-refresh",
            "expires_at":now.as_secs()+3600, "scopes":"chatgpt.tokens.use.direct"
        })
        .to_string(),
    )
    .unwrap();
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    let provider = Provider::with_endpoints(&path, server.endpoints(), false);
    let mut app = App::open_with(&path, provider.clone()).unwrap();
    assert!(provider.connected());
    let ctx = app.feature_context();
    assert_eq!(
        ctx.connection
            .query_row("PRAGMA busy_timeout", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        3000
    );
    for table in [
        "engine_runs",
        "memory_auto_jobs",
        "scheduler_routines",
        "scheduler_occurrences",
        "forms_requests",
        "delegation_graph",
        "wrapper_group_turns",
        "wrapper_meetings",
        "wrapper_demonstrations",
        "wrapper_demonstration_images",
        "wrapper_turn_log",
    ] {
        let count: i64 = ctx
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "fixture must start with no work in {table}");
    }
    ctx.connection
        .execute(
            "UPDATE scheduler_presence SET last_tick_at=?1,absent_since=NULL WHERE id=1",
            [now.as_millis() as u64],
        )
        .unwrap();
    let writer = Connection::open(&path).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let contender = Connection::open(&path).unwrap();
    contender.busy_timeout(Duration::ZERO).unwrap();
    assert!(matches!(contender.execute_batch("BEGIN IMMEDIATE"),
        Err(rusqlite::Error::SqliteFailure(error, _))
        if error.code == rusqlite::ErrorCode::DatabaseBusy));

    let hooks: &[(&str, Tick)] = &[
        ("engine", features::engine::tick),
        ("multibot", features::multibot::tick),
        ("memory_auto", features::memory_auto::tick),
        ("forms", features::forms::tick),
        ("routines", features::routines::tick),
        ("delegation", features::delegation::tick),
        ("attachments", features::attachments::tick),
        ("meetings", features::meetings::tick),
        ("demonstrations", features::demonstrations::tick),
        ("turns", features::turns::tick),
    ];
    let mut slow = Vec::new();
    for (name, tick) in hooks {
        let mut ctx = app.feature_context();
        ctx.now_ms = now.as_millis() as u64;
        let changed = ctx.connection.total_changes();
        let start = Instant::now();
        tick(&ctx);
        let elapsed = start.elapsed();
        let changes = ctx.connection.total_changes() - changed;
        println!(
            "hook={name} elapsed_ms={:.3} changes={changes} under_200_ms={}",
            elapsed.as_secs_f64() * 1000.0,
            elapsed < Duration::from_millis(200)
        );
        assert_eq!(changes, 0, "no work was seeded for {name}");
        if elapsed >= Duration::from_millis(200) {
            slow.push(*name);
        }
    }
    let mut elapsed = Vec::new();
    for sample in 1..=5 {
        let start = Instant::now();
        app.tick(now.as_millis() as u64);
        let duration = start.elapsed();
        println!(
            "App::tick sample={sample} elapsed_ms={:.3} under_200_ms={}",
            duration.as_secs_f64() * 1000.0,
            duration < Duration::from_millis(200)
        );
        elapsed.push(duration);
    }
    writer.execute_batch("ROLLBACK").unwrap();
    contender
        .execute_batch("BEGIN IMMEDIATE; ROLLBACK")
        .unwrap();
    for (name, tick) in hooks {
        let mut ctx = app.feature_context();
        ctx.now_ms = now.as_millis() as u64;
        let start = Instant::now();
        tick(&ctx);
        println!(
            "unlocked_hook={name} elapsed_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
    assert!(
        elapsed
            .iter()
            .all(|duration| *duration < Duration::from_millis(200)),
        "idle App::tick exceeded 200 ms; slow hooks {slow:?}"
    );
    assert!(slow.is_empty(), "idle hooks exceeded 200 ms {slow:?}");
}
