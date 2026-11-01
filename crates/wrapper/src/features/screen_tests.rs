//! Adapter-boundary race tests use only the compiled debug fixture.
use super::*;
use grokbot_engine::{
    contracts::{with_tx, ApprovalCheck, RunBudgets, RunKind, Trigger},
    model::{self, Run, RunStatus},
};
/// A running call whose card the person approved, journaled as the engine does.
fn approved_call(
    app: &mut App,
    chat: &str,
    i: u32,
    name: &str,
    arguments: Value,
) -> (Run, ToolCall) {
    let run = Run {
        run_id: format!("screen-queue-{i}"),
        bot_id: "grokbot".into(),
        conversation_id: chat.to_owned(),
        trigger: Trigger::UserMessage {
            message_id: format!("m{i}"),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Running,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: crate::now() as i64,
    };
    let call = ToolCall {
        call_id: format!("queued-{i}"),
        name: name.into(),
        arguments,
    };
    with_tx(
        &app.connection,
        |conn| -> grokbot_engine::contracts::Result<()> {
            model::start(conn, &run)?;
            let ctx = Context {
                connection: conn,
                bot_id: Some("grokbot"),
                conversation_id: Some(chat),
                run_id: Some(&run.run_id),
                ..app.feature_context()
            };
            let cause = super::super::forms::request_tool_approval(&ctx, &run, &call).unwrap();
            let grokbot_engine::contracts::SuspendCause::HumanInput { id } = cause else {
                panic!()
            };
            grokbot_forms::approvals::decide(
                conn,
                &id,
                chat,
                "user:queue",
                grokbot_forms::approvals::ApprovalDecision::AllowOnce,
                crate::now() as i64,
            )?;
            let ApprovalCheck::Authorized { by } =
                super::super::forms::check_tool_approval(&ctx, &run, &call).unwrap()
            else {
                panic!()
            };
            conn.execute(
                "INSERT INTO engine_external_tools(run_id,call_id,authorized_by) VALUES (?1,?2,?3)",
                params![run.run_id, call.call_id, json!(by).to_string()],
            )?;
            Ok(())
        },
    )
    .unwrap();
    (run, call)
}

#[test]
fn queued_actions_cannot_outlive_revoke_or_bind_a_new_lease() {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-queue");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let mut app = App::open(&path).unwrap();
    let chat = app
        .api("POST", "/api/conversations", br#"{"title":"Queue"}"#)
        .unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.api(
        "POST",
        "/api/screen/bot",
        br#"{"bot_id":"grokbot","enabled":true}"#,
    )
    .unwrap();
    app.api(
        "POST",
        "/api/screen/grant",
        json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let lease = current(&path).unwrap();
    let prepared: Vec<_> = (0..2)
        .map(|i| {
            approved_call(
                &mut app,
                &chat,
                i,
                "computer_click",
                json!({"x":10,"y":20,"button":"left"}),
            )
        })
        .collect();
    // Both approved calls queue behind a busy helper; revocation must NOT wait
    // for that queue lock, and neither call may use the next session's permit.
    let busy = lease.serial.lock().unwrap();
    let handles: Vec<_> = prepared
        .into_iter()
        .map(|(run, call)| {
            let path = path.clone();
            std::thread::spawn(move || {
                let conn = rusqlite::Connection::open(&path).unwrap();
                conn.busy_timeout(Duration::from_secs(3)).unwrap();
                let provider =
                    crate::Provider::with_endpoints(&path, crate::Endpoints::default(), false);
                let ctx = Context {
                    connection: &conn,
                    provider: &provider,
                    db_path: &path,
                    now_ms: crate::now(),
                    bot_id: Some(&run.bot_id),
                    conversation_id: Some(&run.conversation_id),
                    run_id: Some(&run.run_id),
                };
                execute(&ctx, &call, Arc::new(AtomicBool::new(false)))
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(30));
    lease.stop();
    assert!(!lease.live());
    drop(busy);
    for h in handles {
        assert!(matches!(h.join().unwrap(), ToolOutcome::Failed { .. }));
    }
    assert!(lease.events.lock().unwrap().is_empty());
    assert_eq!(
        app.connection
            .query_row("SELECT count(*) FROM wrapper_screen_shots", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    startup(&path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn expiry_changes_poll_token_and_cancels_pending_screen_approvals() {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-expiry");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let mut app = App::open(&path).unwrap();
    let chat = app
        .api("POST", "/api/conversations", br#"{"title":"Expiration"}"#)
        .unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.api(
        "POST",
        "/api/screen/bot",
        br#"{"bot_id":"grokbot","enabled":true}"#,
    )
    .unwrap();
    app.api(
        "POST",
        "/api/screen/grant",
        json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let run = Run {
        run_id: "screen-expiry".into(),
        bot_id: "grokbot".into(),
        conversation_id: chat.clone(),
        trigger: Trigger::UserMessage {
            message_id: "expiry".into(),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Running,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: crate::now() as i64,
    };
    let call = ToolCall {
        call_id: "expiry".into(),
        name: "computer_screenshot".into(),
        arguments: json!({}),
    };
    let request_id = with_tx(
        &app.connection,
        |conn| -> grokbot_engine::contracts::Result<String> {
            model::start(conn, &run)?;
            let ctx = Context {
                connection: conn,
                bot_id: Some("grokbot"),
                conversation_id: Some(&chat),
                run_id: Some(&run.run_id),
                ..app.feature_context()
            };
            let grokbot_engine::contracts::SuspendCause::HumanInput { id } =
                super::super::forms::request_tool_approval(&ctx, &run, &call).unwrap()
            else {
                panic!()
            };
            Ok(id)
        },
    )
    .unwrap();
    let before = app.api("GET", "/api/ping", b"").unwrap()["revision"].clone();
    // Move only the fixture clock/deadline, without waiting a real minute or
    // triggering any native activity. No database write changes the token.
    Arc::get_mut(leases().lock().unwrap().get_mut(&path).unwrap())
        .unwrap()
        .expires = crate::now() - 1;
    let after = app.api("GET", "/api/ping", b"").unwrap()["revision"].clone();
    assert_ne!(before, after);
    app.tick(crate::now());
    let request = grokbot_forms::requests::get(&app.connection, &request_id)
        .unwrap()
        .unwrap();
    assert!(matches!(
        request.status,
        grokbot_forms::requests::RequestStatus::Cancelled { .. }
    ));
    assert_eq!(state(&app.feature_context())["session"], Value::Null);
    startup(&path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn idle_reconciliation_does_not_take_a_writer_lock() {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-idle");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let app = App::open(&path).unwrap();
    app.connection.busy_timeout(Duration::ZERO).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = app.connection.total_changes();
    // This must succeed while another connection owns the writer lock. A
    // zero-row DELETE would fail, even though screen control was never enabled.
    reconcile(&app.connection, &path).unwrap();
    assert_eq!(app.connection.total_changes(), before);
    writer.execute_batch("ROLLBACK").unwrap();
    startup(&path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn approving_inside_the_shell_does_not_invalidate_an_active_permit_but_outside_input_pauses() {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-input");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let mut app = App::open(&path).unwrap();
    let chat = app
        .api("POST", "/api/conversations", br#"{"title":"Saisie"}"#)
        .unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.api(
        "POST",
        "/api/screen/bot",
        br#"{"bot_id":"grokbot","enabled":true}"#,
    )
    .unwrap();
    app.api(
        "POST",
        "/api/screen/grant",
        json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let lease = current(&path).unwrap();
    let permit = lease.generation.load(Ordering::SeqCst);
    // The card's approve-and-resume step is a no-op while the shell is active.
    let body = json!({"id":lease.id}).to_string();
    app.api("POST", "/api/screen/resume", body.as_bytes())
        .unwrap();
    assert!(!lease.paused());
    assert_eq!(lease.generation.load(Ordering::SeqCst), permit);
    // This debug fixture denotes an input outside Ezerbot, never a real event.
    app.api("POST", "/api/screen/fake-input", body.as_bytes())
        .unwrap();
    assert!(lease.paused());
    assert_ne!(lease.generation.load(Ordering::SeqCst), permit);
    app.api("POST", "/api/screen/resume", body.as_bytes())
        .unwrap();
    assert!(!lease.paused());
    assert_ne!(lease.generation.load(Ordering::SeqCst), permit);
    startup(&path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_yolo_gesture_admitted_under_one_session_never_acts_in_its_replacement() {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-yolo");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    // Le Mode YOLO est actif par défaut : aucun réglage Écran n'est nécessaire.
    let mut app = App::open(&path).unwrap();
    let chat = app
        .api("POST", "/api/conversations", br#"{"title":"YOLO"}"#)
        .unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let grant = |app: &mut App| {
        app.api(
            "POST",
            "/api/screen/grant",
            json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        current(&path).unwrap()
    };
    let first = grant(&mut app);
    let admit = |app: &App, id: &str| {
        let run = Run {
            run_id: format!("screen-yolo-{id}"),
            bot_id: "grokbot".into(),
            conversation_id: chat.clone(),
            trigger: Trigger::UserMessage {
                message_id: format!("m-{id}"),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Running,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: crate::now() as i64,
        };
        let call = ToolCall {
            call_id: format!("click-{id}"),
            name: "computer_click".into(),
            arguments: json!({"x":10,"y":20,"button":"left"}),
        };
        let mut session = None;
        with_tx(
            &app.connection,
            |conn| -> grokbot_engine::contracts::Result<()> {
                model::start(conn, &run)?;
                let ctx = Context {
                    connection: conn,
                    bot_id: Some("grokbot"),
                    conversation_id: Some(&chat),
                    run_id: Some(&run.run_id),
                    ..app.feature_context()
                };
                let ApprovalCheck::Authorized { by } =
                    super::super::forms::check_tool_approval(&ctx, &run, &call).unwrap()
                else {
                    panic!("le Mode YOLO doit admettre le geste sans carte");
                };
                session = match &by {
                    grokbot_engine::contracts::AuthorizedBy::Yolo { session, .. } => *session,
                    other => panic!("{other:?}"),
                };
                conn.execute(
                "INSERT INTO engine_external_tools(run_id,call_id,authorized_by) VALUES (?1,?2,?3)",
                params![run.run_id, call.call_id, json!(by).to_string()],
            )?;
                Ok(())
            },
        )
        .unwrap();
        (run, call, session)
    };
    let (old_run, old_call, bound) = admit(&app, "old");
    assert_eq!(bound, Some(first.id));
    // La session est révoquée, une autre est accordée au même bot et au même fil.
    first.stop();
    let second = grant(&mut app);
    assert_ne!(second.id, first.id);
    let run_now = |run: &Run, call: &ToolCall| {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let provider = crate::Provider::with_endpoints(&path, crate::Endpoints::default(), false);
        let ctx = Context {
            connection: &conn,
            provider: &provider,
            db_path: &path,
            now_ms: crate::now(),
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            run_id: Some(&run.run_id),
        };
        execute(&ctx, call, Arc::new(AtomicBool::new(false)))
    };
    assert!(matches!(
        run_now(&old_run, &old_call),
        ToolOutcome::Failed { .. }
    ));
    assert!(second.events.lock().unwrap().is_empty());
    // Un geste admis sous la session vivante agit.
    let (run, call, bound) = admit(&app, "new");
    assert_eq!(bound, Some(second.id));
    assert!(matches!(run_now(&run, &call), ToolOutcome::Done { .. }));
    assert_eq!(second.events.lock().unwrap().len(), 1);
    second.stop();
    startup(&path);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_see_only_lease_offers_the_model_nothing_but_the_screenshot() {
    let names = |see_only| {
        offered(see_only)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(true), ["computer_screenshot"]);
    assert_eq!(names(false).len(), 6);
}

const KEEPS_RUNNING: &str = "exec cat >/dev/null";

/// A helper process that stays silent until `go` exists, then runs `clicked`, like
/// one whose button is clicked while the wrapper stores a capture.
fn clicked_helper(go: &Path, clicked: &str) -> std::process::Command {
    let mut helper = std::process::Command::new("sh");
    helper.args([
        "-c",
        &format!(
            r#"echo '{{"ready":true}}'
            while [ ! -e "$0" ]; do sleep 0.01; done
            {clicked}"#
        ),
    ]);
    helper.arg(go);
    helper
}

/// Runs a real capture of the debug fixture. While its row is being inserted, so
/// after the check and before the COMMIT, `during_insert` acts. Returns the outcome
/// and the number of stored rows.
fn capture_interrupted(
    clicked: &str,
    during_insert: impl Fn(&Arc<Lease>) + Send + 'static,
) -> (ToolOutcome, i64) {
    let _mac = TEST_MAC.lock().unwrap();
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-stop-before-commit");
    let root = guard.to_path_buf();
    let path = root.join("workspace.sqlite");
    let mut app = App::open(&path).unwrap();
    let chat = app
        .api("POST", "/api/conversations", br#"{"title":"Stop"}"#)
        .unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.api(
        "POST",
        "/api/screen/bot",
        br#"{"bot_id":"grokbot","enabled":true}"#,
    )
    .unwrap();
    app.api(
        "POST",
        "/api/screen/grant",
        json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let (run, call) = approved_call(&mut app, &chat, 0, "computer_screenshot", json!({}));
    let lease = current(&path).unwrap();
    let go = root.join("go");
    let client = native::Client::launch(
        clicked_helper(&go, clicked),
        lease.permit.clone(),
        lease.generation.clone(),
    )
    .unwrap();

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.busy_timeout(Duration::from_secs(3)).unwrap();
    let inserting = lease.clone();
    let inserted = go.clone();
    conn.update_hook(Some(
        move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
            if table == "wrapper_screen_shots" {
                std::fs::write(&inserted, b"").unwrap();
                during_insert(&inserting);
            }
        },
    ));
    let provider = crate::Provider::with_endpoints(&path, crate::Endpoints::default(), false);
    let ctx = Context {
        connection: &conn,
        provider: &provider,
        db_path: &path,
        now_ms: crate::now(),
        bot_id: Some(&run.bot_id),
        conversation_id: Some(&run.conversation_id),
        run_id: Some(&run.run_id),
    };
    let outcome = execute(&ctx, &call, Arc::new(AtomicBool::new(false)));
    let rows = app
        .connection
        .query_row("SELECT count(*) FROM wrapper_screen_shots", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    drop(client);
    startup(&path);
    drop(app);
    let _ = std::fs::remove_dir_all(root);
    (outcome, rows)
}

/// Returns once the helper's line has reached the wrapper, while the store goes on.
fn until_heard(from: u64) -> impl Fn(&Arc<Lease>) + Send + 'static {
    move |lease| {
        let start = std::time::Instant::now();
        while lease.generation.load(Ordering::SeqCst) == from {
            assert!(start.elapsed() < Duration::from_secs(5), "line never read");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn a_capture_nothing_interrupts_is_stored_and_returned() {
    let (outcome, rows) = capture_interrupted(KEEPS_RUNNING, |_| {});
    assert!(matches!(outcome, ToolOutcome::Done { .. }));
    assert_eq!(rows, 1);
}

#[test]
fn a_stop_from_the_helper_before_the_commit_stores_and_returns_nothing() {
    for clicked in [
        // Arrêter.
        r#"echo '{"event":"revoked"}'; exec cat >/dev/null"#,
        // Pause.
        r#"echo '{"event":"paused"}'; exec cat >/dev/null"#,
        // Pause, and a resume the reader has not reached yet.
        r#"echo '{"event":"paused"}'; echo '{"event":"resumed"}'; exec cat >/dev/null"#,
        // The helper dies.
        "exit 0",
    ] {
        let (outcome, rows) = capture_interrupted(clicked, until_heard(1));
        assert!(matches!(outcome, ToolOutcome::Failed { .. }), "{clicked}");
        assert_eq!(rows, 0, "{clicked}");
    }
}

#[test]
fn a_stop_from_the_app_before_the_commit_stores_and_returns_nothing() {
    let (outcome, rows) = capture_interrupted(KEEPS_RUNNING, |lease| {
        let stopping = lease.clone();
        std::thread::spawn(move || stopping.stop());
        let start = std::time::Instant::now();
        while lease.live() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "stop never landed"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    assert!(matches!(outcome, ToolOutcome::Failed { .. }));
    assert_eq!(rows, 0);
}

#[test]
fn a_helper_stop_reads_as_stopped_only_once_the_decision_in_flight_is_over() {
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-stop-waits");
    let go = guard.to_path_buf().join("go");
    let permit = Arc::new(Mutex::new(()));
    let generation = Arc::new(AtomicU64::new(1));
    let client = native::Client::launch(
        clicked_helper(&go, r#"echo '{"event":"revoked"}'; exec cat >/dev/null"#),
        permit.clone(),
        generation.clone(),
    )
    .unwrap();
    let deciding = permit.lock().unwrap();
    std::fs::write(&go, b"").unwrap();
    let start = std::time::Instant::now();
    while generation.load(Ordering::SeqCst) == 1 {
        assert!(start.elapsed() < Duration::from_secs(5), "line never read");
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(client.live(), "a decision in flight is not over yet");
    drop(deciding);
    while client.live() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "stop never applied"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_resume_the_helper_refuses_names_the_banner_and_stays_paused() {
    let guard = grokbot_test_support::TestDir::new("ezerbot-screen-unproven");
    let go = guard.to_path_buf().join("go");
    let client = native::Client::launch(
        clicked_helper(
            &go,
            r#"echo '{"event":"paused","reason":"banner"}'; exec cat >/dev/null"#,
        ),
        Arc::default(),
        Arc::default(),
    )
    .unwrap();
    std::fs::write(&go, b"").unwrap();
    let start = std::time::Instant::now();
    while !client.paused() {
        assert!(start.elapsed() < Duration::from_secs(5), "line never read");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(client.unproven());
    let refusal = client.resume().unwrap_err();
    assert!(refusal.message.contains("bandeau"), "{}", refusal.message);
    assert!(client.paused());
}
