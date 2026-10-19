//! Adapter-boundary race tests use only the compiled debug fixture.
use super::*;
use grokbot_engine::{
    contracts::{with_tx, ApprovalCheck, RunBudgets, RunKind, Trigger},
    model::{self, Run, RunStatus},
};
static TEST_MAC: Mutex<()> = Mutex::new(());
#[test]
fn queued_actions_cannot_outlive_revoke_or_bind_a_new_lease() {
    let _mac = TEST_MAC.lock().unwrap();
    let root = std::env::temp_dir().join(format!(
        "ezerbot-screen-queue-{}-{}",
        std::process::id(),
        crate::now()
    ));
    std::fs::create_dir_all(&root).unwrap();
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
    let mut prepared = Vec::new();
    for i in 0..2 {
        let run = Run {
            run_id: format!("screen-queue-{i}"),
            bot_id: "grokbot".into(),
            conversation_id: chat.clone(),
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
            name: "computer_click".into(),
            arguments: json!({"x":10,"y":20,"button":"left"}),
        };
        with_tx(&app.connection,|conn|->grokbot_engine::contracts::Result<()>{
            model::start(conn,&run)?;
            let ctx=Context{connection:conn,bot_id:Some("grokbot"),conversation_id:Some(&chat),run_id:Some(&run.run_id),..app.feature_context()};
            let cause=super::super::forms::request_tool_approval(&ctx,&run,&call).unwrap();
            let grokbot_engine::contracts::SuspendCause::HumanInput{id}=cause else {panic!()};
            grokbot_forms::approvals::decide(conn,&id,&chat,"user:queue",grokbot_forms::approvals::ApprovalDecision::AllowOnce,crate::now() as i64)?;
            let ApprovalCheck::Authorized{by}=super::super::forms::check_tool_approval(&ctx,&run,&call).unwrap() else {panic!()};
            conn.execute("INSERT INTO engine_external_tools(run_id,call_id,authorized_by) VALUES (?1,?2,?3)",params![run.run_id,call.call_id,json!(by).to_string()])?;
            Ok(())
        }).unwrap();
        prepared.push((run, call));
    }
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
    let root = std::env::temp_dir().join(format!(
        "ezerbot-screen-expiry-{}-{}",
        std::process::id(),
        crate::now()
    ));
    std::fs::create_dir_all(&root).unwrap();
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
fn approving_inside_the_shell_does_not_invalidate_an_active_permit_but_outside_input_pauses() {
    let _mac = TEST_MAC.lock().unwrap();
    let root = std::env::temp_dir().join(format!(
        "ezerbot-screen-input-{}-{}",
        std::process::id(),
        crate::now()
    ));
    std::fs::create_dir_all(&root).unwrap();
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
