use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use grokbot_domain::{ConversationId, MessageId};
use grokbot_session::SubmissionTransport;
use serde_json::json;

use super::{validate_headers, App};

static NEXT: AtomicUsize = AtomicUsize::new(0);

// Seed only metadata: linking does not read image bytes. This exercises the real
// attachment core without implementing the downstream upload/thumbnail adapter.
fn seed_attachments(app: &App) {
    let tx = super::features::write_transaction(&app.connection).unwrap();
    grokbot_attachments::staging::migrate(&tx).unwrap();
    for (id, digest) in [("image-a", "a".repeat(64)), ("image-b", "b".repeat(64))] {
        tx.execute("INSERT INTO attachments_staged(id, sha256, image_type, byte_length, staged_at) VALUES (?1, ?2, 'image/png', 24, 0)", rusqlite::params![id, digest]).unwrap();
    }
    tx.commit().unwrap();
}

fn attachment_link_hook(
    ctx: &super::features::Context<'_>,
    conversation: &str,
    message: &str,
    ids: &[String],
) -> Result<(), super::ApiError> {
    let existing =
        grokbot_attachments::staging::for_message(ctx.connection, conversation, message)?;
    let old: std::collections::BTreeSet<_> = existing.iter().map(|item| item.id.as_str()).collect();
    let new: std::collections::BTreeSet<_> = ids.iter().map(String::as_str).collect();
    if !old.is_empty() && old != new {
        return Err(super::ApiError::new(409, "Attachment set changed"));
    }
    grokbot_attachments::staging::link_attachments(ctx.connection, conversation, message, ids)?;
    Ok(())
}

#[test]
fn rejected_attachment_send_does_not_create_submission() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    for (message, text) in [("empty", ""), ("text", "Original")] {
        let result = super::features::test_hooks::with_link(attachment_link_hook, || {
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id": id, "message_id": message, "body": text, "attachments": ["expired-attachment"]}),
            )
        });
        assert_eq!(result, Err(400));
        assert!(app.reader.list_submissions().unwrap().is_empty());
        assert_eq!(
            app.connection
                .query_row("SELECT count(*) FROM wrapper_messages", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            app.connection
                .query_row("SELECT count(*) FROM attachments_message_links", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            app.connection
                .query_row("SELECT count(*) FROM wrapper_remote", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Refused input did not reserve the nonce/body in the journal.
    assert!(post(
        &mut app,
        "/api/send",
        json!({"conversation_id": id, "message_id": "empty", "body": "Corrected"})
    )
    .is_ok());
}

#[test]
fn queued_image_only_retry_without_attachment_ids_is_recoverable() {
    fn must_not_relink(
        _: &super::features::Context<'_>,
        _: &str,
        _: &str,
        _: &[String],
    ) -> Result<(), super::ApiError> {
        panic!("a body-only retry must keep existing links, not validate an empty replacement set")
    }
    for with_links in [false, true] {
        let db = Database::new();
        let mut app = App::open(&db.0).unwrap();
        let id = conversation(&mut app);
        seed_attachments(&app);
        if with_links {
            let tx = super::features::write_transaction(&app.connection).unwrap();
            grokbot_attachments::staging::link_attachments(
                &tx,
                &id,
                "queued-image",
                &["image-a".into()],
            )
            .unwrap();
            super::features::conversations::append_entry(&tx, &id, "submission", "queued-image", 1)
                .unwrap();
            tx.commit().unwrap();
        }
        app.session
            .prepare(
                ConversationId::new(&id).unwrap(),
                MessageId::new("queued-image").unwrap(),
                String::new(),
            )
            .unwrap();
        drop(app); // Simulate a crash between prepare and transport (also the old journal-only gap).
        let mut app = App::open(&db.0).unwrap();
        let result = super::features::test_hooks::with_link(must_not_relink, || {
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id": id, "message_id": "queued-image", "body": ""}),
            )
            .unwrap()
        });
        assert_eq!(result["conversation"]["messages"][0]["state"], "echoed");
        assert!(app.reader.list_submissions().unwrap()[0].body.is_empty());
        assert_eq!(
            grokbot_attachments::staging::for_message(&app.connection, &id, "queued-image")
                .unwrap()
                .len(),
            usize::from(with_links)
        );
    }
}

#[test]
fn crash_after_link_commit_before_journal_prepare_is_invisible_and_retryable() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    let tx = super::features::write_transaction(&app.connection).unwrap();
    grokbot_attachments::staging::link_attachments(&tx, &id, "orphan", &["image-a".into()])
        .unwrap();
    super::features::conversations::append_entry(&tx, &id, "submission", "orphan", 1).unwrap();
    tx.commit().unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let result = super::features::test_hooks::with_link(attachment_link_hook, || {
        post(&mut app, "/api/send", json!({"conversation_id": id, "message_id": "orphan", "body": "", "attachments": ["image-a"]})).unwrap()
    });
    assert_eq!(result["conversation"]["messages"][0]["state"], "echoed");
    assert_eq!(app.reader.list_submissions().unwrap().len(), 1);
    assert_eq!(
        grokbot_attachments::staging::for_message(&app.connection, &id, "orphan")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn timeline_kind_collisions_return_conflict_in_both_directions() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    super::features::conversations::append_entry(&app.connection, &id, "form", "same", 1).unwrap();
    assert_eq!(
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id": id, "message_id": "same", "body": "Not journaled"})
        ),
        Err(409)
    );
    assert!(app.reader.list_submissions().unwrap().is_empty());
    assert_eq!(
        super::features::conversations::append_entry(&app.connection, &id, "task", "same", 2)
            .unwrap_err()
            .status,
        409
    );
    send(&mut app, &id, "Submission first", false);
    assert_eq!(
        super::features::conversations::append_entry(
            &app.connection,
            &id,
            "form",
            "stable-nonce",
            3
        )
        .unwrap_err()
        .status,
        409
    );
}

#[test]
fn attachment_link_contract_rejects_changed_nonempty_retry_sets() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    super::features::test_hooks::with_link(attachment_link_hook, || {
        let request = json!({"conversation_id": id, "message_id": "image", "body": "", "attachments": ["image-a"]});
        post(&mut app, "/api/send", request.clone()).unwrap();
        post(&mut app, "/api/send", request).unwrap();
        assert_eq!(
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id": id, "message_id": "image", "body": "", "attachments": ["image-b"]})
            ),
            Err(409)
        );
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id": id, "message_id": "image", "body": ""}),
        )
        .unwrap();
    });
    let links = grokbot_attachments::staging::for_message(&app.connection, &id, "image").unwrap();
    assert_eq!(
        links
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["image-a"]
    );
}

#[test]
fn unknown_entry_kind_is_projected_in_ordinal_order() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    super::features::conversations::append_entry(
        &app.connection,
        &id,
        "future-card",
        "before",
        900,
    )
    .unwrap();
    send(&mut app, &id, "First", false);
    super::features::conversations::append_entry(
        &app.connection,
        &id,
        "future-card",
        "between",
        100,
    )
    .unwrap();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id": id, "message_id": "second", "body": "Second"}),
    )
    .unwrap();
    super::features::conversations::append_entry(&app.connection, &id, "future-card", "after", 1)
        .unwrap();
    // Appending the same entry never consumes a second ordinal.
    super::features::conversations::append_entry(&app.connection, &id, "future-card", "after", 1)
        .unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    let messages = state["conversations"][0]["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .map(|message| message["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "before",
            "stable-nonce",
            "reply-stable-nonce",
            "between",
            "second",
            "reply-second",
            "after"
        ]
    );
    assert_eq!(messages[0]["kind"], "future-card");
    assert_eq!(messages[1]["kind"], "message");
    assert_eq!(state["conversations"][0]["kind"], "chat");
    assert!(state["conversations"][0]["parent_id"].is_null());
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"],
        state["conversations"]
    );
    post(&mut app, "/api/conversations/delete", json!({"id": id})).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"],
        json!([])
    );
}

#[test]
fn old_database_gains_kind_columns() {
    let db = Database::new();
    let conn = rusqlite::Connection::open(&db.0).unwrap();
    conn.execute_batch("CREATE TABLE wrapper_conversations (
        account_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL,
        favorite INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY(account_id, id));
        CREATE TABLE wrapper_messages (
        ordinal INTEGER PRIMARY KEY, account_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
        message_id TEXT NOT NULL, created_at INTEGER NOT NULL, UNIQUE(account_id, conversation_id, message_id));
        INSERT INTO wrapper_conversations (account_id, id, title, created_at, updated_at)
        VALUES ('local-workspace', 'older', 'Ancienne', 123, 123);
        INSERT INTO wrapper_messages VALUES (17, 'local-workspace', 'older', 'stable-nonce', 123);").unwrap();
    drop(conn);
    for _ in 0..2 {
        let mut app = App::open(&db.0).unwrap();
        let response = send(&mut app, "older", "Préservé", false);
        assert_eq!(response["conversation"]["kind"], "chat");
        assert!(response["conversation"]["parent_id"].is_null());
        let metadata: (i64, String) = app
            .connection
            .query_row(
                "SELECT ordinal, kind FROM wrapper_messages WHERE message_id = 'stable-nonce'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(metadata, (17, "submission".into()));
        assert_eq!(
            app.connection
                .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            app.connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(app.connection.query_row("SELECT count(*) FROM component_versions WHERE component = 'wrapper' AND version = 1", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }
}

#[test]
fn image_only_message_is_accepted() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    let request = json!({"conversation_id": id, "message_id": "image-only", "body": "  ", "attachments": ["image-a"]});
    let first = post(&mut app, "/api/send", request.clone()).unwrap();
    assert_eq!(first["conversation"]["messages"][0]["state"], "echoed");
    // The journal accepts the real empty body, so no [image] fallback is needed.
    assert_eq!(first["conversation"]["messages"][0]["body"], "");
    assert_eq!(
        first["conversation"]["messages"],
        post(&mut app, "/api/send", request).unwrap()["conversation"]["messages"]
    );
    let submissions = app.reader.list_submissions().unwrap();
    assert_eq!(submissions.len(), 1);
    assert!(submissions[0].body.is_empty());
    assert_eq!(
        app.connection
            .query_row("SELECT send_count FROM wrapper_remote", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id": id, "message_id": "empty", "body": "", "attachments": []})
        ),
        Err(400)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id": id, "message_id": "bad-image", "body": "", "attachments": ["../image"]})
        ),
        Err(400)
    );
    assert_eq!(app.reader.list_submissions().unwrap().len(), 1);
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"],
        first["conversation"]["messages"]
    );
}

#[test]
fn two_writers_do_not_return_busy() {
    use std::sync::mpsc;
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    // Use the actual constructor shared by send, tool probes, migration and
    // LocalTransport, then establish a read snapshot before attempting a write.
    let tx = super::features::write_transaction(&app.connection).unwrap();
    let _: i64 = tx
        .query_row("SELECT count(*) FROM wrapper_messages", [], |row| {
            row.get(0)
        })
        .unwrap();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let path = db.0.clone();
    let worker_id = id.clone();
    let worker = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(path).unwrap();
        // Deliberately disable the default busy handler: observe the contested
        // write now, and explicitly retry it only after the wrapper commits.
        conn.busy_timeout(std::time::Duration::ZERO).unwrap();
        let first = conn.execute("INSERT INTO wrapper_messages(account_id, conversation_id, message_id, kind, created_at) VALUES ('local-workspace', ?1, 'worker', 'task', 1)", [&worker_id]);
        let retry = first.is_err();
        attempt_tx.send(first.map(|_| ())).unwrap();
        release_rx.recv().unwrap();
        if retry {
            super::features::conversations::append_entry(&conn, &worker_id, "task", "worker", 1)
                .unwrap();
        }
    });
    let competing_attempt = attempt_rx.recv().unwrap();
    let result = super::features::conversations::append_entry(&tx, &id, "tool", "wrapper", 2);
    if result.is_ok() {
        tx.commit().unwrap();
    } else {
        tx.rollback().unwrap();
    }
    release_tx.send(()).unwrap();
    worker.join().unwrap();
    // DEFERRED instead of IMMEDIATE lets the other writer commit after our read:
    // this write then fails with BUSY_SNAPSHOT (517), independent of timeouts.
    result.unwrap();
    assert!(
        matches!(competing_attempt, Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::DatabaseBusy)
    );
    assert_eq!(
        app.connection
            .query_row("SELECT count(*) FROM wrapper_messages", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn queued_current_message_does_not_drop_the_previous_assistant_from_preview() {
    use grokbot_engine::contracts::ContextSource;
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let response = send(&mut app, &id, "First question", false);
    let previous_reply = response["conversation"]["messages"][1]["body"]
        .as_str()
        .unwrap();
    app.session
        .prepare(
            ConversationId::new(&id).unwrap(),
            MessageId::new("queued").unwrap(),
            "Still queued".into(),
        )
        .unwrap();
    let blocks = app.context_preview(&id).unwrap();
    let history = blocks
        .iter()
        .find(|block| block.source == ContextSource::History)
        .unwrap();
    assert!(history.text.contains("First question"));
    assert!(history.text.contains(previous_reply));
    assert!(!history.text.contains("Still queued"));
    assert_eq!(blocks.last().unwrap().text, "Still queued");
}

#[test]
fn hidden_context_and_tool_probes_use_the_feature_dispatcher() {
    use grokbot_engine::contracts::ContextSource;
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "First", false);
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id": id, "message_id": "second", "body": "Current"}),
    )
    .unwrap();
    let blocks = app.context_preview(&id).unwrap();
    assert_eq!(blocks.first().unwrap().source, ContextSource::AppContract);
    assert!(blocks
        .windows(2)
        .all(|pair| pair[0].priority <= pair[1].priority));
    assert!(blocks
        .iter()
        .any(|block| block.source == ContextSource::BotIdentity));
    assert!(blocks
        .iter()
        .any(|block| block.source == ContextSource::History && block.text.contains("First")));
    assert_eq!(blocks.last().unwrap().source, ContextSource::CurrentMessage);
    assert_eq!(blocks.last().unwrap().text, "Current");
    assert_eq!(app.context_preview("absent").unwrap_err().status, 404);
    assert_eq!(
        app.invoke_tool("grokbot", "unimplemented", json!({}))
            .unwrap_err()
            .status,
        404
    );
    assert!(app.connection.is_autocommit());
    assert_eq!(
        app.invoke_tool("absent", "unimplemented", json!({}))
            .unwrap_err()
            .status,
        404
    );
    app.tick(123);
    let state = app.api("GET", "/api/state", b"").unwrap();
    for capability in [
        "engine",
        "skills",
        "memory",
        "forms",
        "routines",
        "delegation",
        "attachments",
    ] {
        assert!(
            state.get(capability).is_some(),
            "missing namespaced state: {capability}"
        );
    }
    assert!(state["conversations"].is_array());
}

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "grokbot-wrapper-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

fn conversation(app: &mut App) -> String {
    app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .expect("conversation ID")
        .to_owned()
}

fn send(app: &mut App, id: &str, body: &str, loss: bool) -> serde_json::Value {
    app.api(
        "POST",
        "/api/send",
        &serde_json::to_vec(&json!({
            "conversation_id": id, "message_id": "stable-nonce", "body": body, "simulate_loss": loss
        }))
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn lost_response_is_reconciled_after_restart_without_second_remote_effect() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let lost = send(&mut app, &id, "Mon message", true);
    assert_eq!(lost["conversation"]["messages"][0]["state"], "unknown");
    assert_eq!(
        lost["conversation"]["messages"].as_array().unwrap().len(),
        1
    );
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let reconciled = app
        .api(
            "POST",
            "/api/reconcile",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "stable-nonce"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(reconciled["conversation"]["messages"][0]["state"], "echoed");
    let same = send(&mut app, &id, "Mon message", false);
    assert_eq!(
        same["conversation"]["messages"].as_array().unwrap().len(),
        2
    );
    let count: i64 = rusqlite::Connection::open(&db.0).unwrap()
        .query_row("SELECT send_count FROM wrapper_remote WHERE account_id = 'local-workspace' AND conversation_id = ?1", [&id], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn immutable_nonce_conflict_preserves_original_body() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "Original", false);
    let err = app
        .api(
            "POST",
            "/api/send",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "stable-nonce", "body": "Changed"
            }))
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.status, 409);
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["conversations"][0]["messages"][0]["body"], "Original");
}

#[test]
fn rename_and_favorite_survive_restart_and_delete_hides_core_records() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "Durable", false);
    app.api(
        "POST",
        "/api/conversations/update",
        &serde_json::to_vec(&json!({
            "id": id, "title": "Carnet", "favorite": true
        }))
        .unwrap(),
    )
    .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["conversations"][0]["title"], "Carnet");
    assert_eq!(state["conversations"][0]["favorite"], true);
    app.api(
        "POST",
        "/api/conversations/delete",
        &serde_json::to_vec(&json!({"id": id})).unwrap(),
    )
    .unwrap();
    let err = app
        .api(
            "POST",
            "/api/send",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "fresh", "body": "Late"
            }))
            .unwrap(),
        )
        .unwrap_err();
    assert_eq!(err.status, 409);
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"],
        json!([])
    );
    let conn = rusqlite::Connection::open(&db.0).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM submissions WHERE account_id = 'local-workspace'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn boundaries_reject_invalid_input_before_creating_submission() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    for body in [" ".to_owned(), "a".repeat(16_385)] {
        let err = app
            .api(
                "POST",
                "/api/send",
                &serde_json::to_vec(&json!({
                    "conversation_id": id, "message_id": "n", "body": body
                }))
                .unwrap(),
            )
            .unwrap_err();
        assert_eq!(err.status, 400);
    }
    for title in ["", " ", &"é".repeat(81)] {
        assert_eq!(
            app.api(
                "POST",
                "/api/conversations",
                &serde_json::to_vec(&json!({"title": title})).unwrap()
            )
            .unwrap_err()
            .status,
            400
        );
    }
    assert_eq!(
        app.api("POST", "/api/send", &vec![b'x'; 65_537])
            .unwrap_err()
            .status,
        413
    );
    assert_eq!(app.api("POST", "/api/send", b"{").unwrap_err().status, 400);
    assert_eq!(app.api("GET", "/not-found", b"").unwrap_err().status, 404);
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"],
        json!([])
    );
}

#[test]
fn cross_origin_and_rebinding_requests_are_rejected() {
    let good = vec![
        ("Host".into(), "127.0.0.1:7777".into()),
        ("Origin".into(), "http://127.0.0.1:7777".into()),
        (
            "Content-Type".into(),
            "application/json; charset=utf-8".into(),
        ),
        ("X-Grokbot-Request".into(), "local".into()),
    ];
    assert!(validate_headers(7777, "POST", &good).is_ok());
    for (index, bad) in [
        (0, "evil.test:7777"),
        (1, "https://127.0.0.1:7777"),
        (2, "text/plain"),
        (3, ""),
    ] {
        let mut headers = good.clone();
        headers[index].1 = bad.into();
        assert_eq!(
            validate_headers(7777, "POST", &headers).unwrap_err().status,
            403
        );
    }
    let mut duplicate = good.clone();
    duplicate.push(("Host".into(), "evil.test:7777".into()));
    assert_eq!(
        validate_headers(7777, "POST", &duplicate)
            .unwrap_err()
            .status,
        403
    );
    assert_eq!(
        validate_headers(7777, "POST", &good[..3])
            .unwrap_err()
            .status,
        403
    );
}

#[test]
fn theme_preference_is_account_scoped_and_survives_restart() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["theme"],
        "dark"
    );
    app.api("POST", "/api/preferences", br#"{"theme":"light"}"#)
        .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["theme"],
        "light"
    );
    assert_eq!(
        app.api("POST", "/api/preferences", br#"{"theme":"invalid"}"#)
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["theme"],
        "light"
    );
}

#[test]
fn accent_preference_survives_restart_and_keeps_the_theme() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["accent"],
        "citron"
    );
    app.api("POST", "/api/preferences", br#"{"theme":"light"}"#)
        .unwrap();
    app.api("POST", "/api/preferences", br#"{"accent":"menthe"}"#)
        .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let preferences = app.api("GET", "/api/state", b"").unwrap()["preferences"].clone();
    assert_eq!(preferences["accent"], "menthe");
    assert_eq!(preferences["theme"], "light");
    for body in [&br#"{"accent":"terracotta"}"#[..], &br#"{}"#[..]] {
        assert_eq!(
            app.api("POST", "/api/preferences", body)
                .unwrap_err()
                .status,
            400
        );
    }
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["accent"],
        "menthe"
    );
}

#[test]
fn crash_before_transport_keeps_journal_body_visible_and_lookup_allows_same_nonce_send() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let conversation = ConversationId::new(&id).unwrap();
    let message = MessageId::new("stable-nonce").unwrap();
    app.session
        .prepare(
            conversation.clone(),
            message.clone(),
            "Saved before effect".into(),
        )
        .unwrap();
    let _unfinished = app.session.begin_send(&conversation, &message).unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        state["conversations"][0]["messages"][0]["body"],
        "Saved before effect"
    );
    assert_eq!(
        state["conversations"][0]["messages"][0]["state"],
        "dispatching"
    );
    let queued = app
        .api(
            "POST",
            "/api/reconcile",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "stable-nonce"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(queued["conversation"]["messages"][0]["state"], "queued");
    let echoed = send(&mut app, &id, "Saved before effect", false);
    assert_eq!(echoed["conversation"]["messages"][0]["state"], "echoed");
    assert_eq!(
        echoed["conversation"]["messages"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn acceptance_saved_before_echo_finishes_after_restart_without_remote_resend() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let conversation = ConversationId::new(&id).unwrap();
    let message = MessageId::new("stable-nonce").unwrap();
    app.session
        .prepare(conversation.clone(), message.clone(), "Accepted".into())
        .unwrap();
    let request = app.session.begin_send(&conversation, &message).unwrap();
    let mut transport = super::features::engine::LocalTransport {
        connection: &app.connection,
        lose_reply: false,
        live: false,
        error: None,
    };
    let outcome = transport.send(&request);
    app.session.apply_send(&request, outcome).unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let result = app
        .api(
            "POST",
            "/api/reconcile",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "stable-nonce"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(result["conversation"]["messages"][0]["state"], "echoed");
    let count: i64 = app
        .connection
        .query_row("SELECT sum(send_count) FROM wrapper_remote", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        result["conversation"]["messages"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn first_send_names_default_conversation_from_first_line() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let result = send(
        &mut app,
        &id,
        "  Mon prochain projet\nLes détails suivent.",
        false,
    );
    assert_eq!(result["conversation"]["title"], "Mon prochain projet");
    let subsequent = app
        .api(
            "POST",
            "/api/send",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "second", "body": "Un autre message"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(subsequent["conversation"]["title"], "Mon prochain projet");
}

#[test]
fn first_send_preserves_explicit_and_renamed_titles_even_if_matching_default() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    for title in ["Titre choisi", "Nouvelle conversation"] {
        let created = app
            .api(
                "POST",
                "/api/conversations",
                &serde_json::to_vec(&json!({"title": title})).unwrap(),
            )
            .unwrap();
        let id = created["conversation"]["id"].as_str().unwrap();
        assert_eq!(
            send(&mut app, id, "Première phrase", false)["conversation"]["title"],
            title
        );
    }
    let id = conversation(&mut app);
    app.api(
        "POST",
        "/api/conversations/update",
        &serde_json::to_vec(&json!({"id": id, "title": "Nouvelle conversation"})).unwrap(),
    )
    .unwrap();
    assert_eq!(
        send(&mut app, &id, "Titre suggéré", false)["conversation"]["title"],
        "Nouvelle conversation"
    );
}

#[test]
fn automatic_title_truncation_preserves_unicode_and_eighty_character_limit() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    let result = send(&mut app, &id, &"é".repeat(100), false);
    let title = result["conversation"]["title"].as_str().unwrap();
    assert_eq!(title.chars().count(), 80);
    assert!(title.starts_with(&"é".repeat(79)));
    assert!(title.ends_with('…'));
}

#[test]
fn existing_wrapper_database_migrates_title_metadata_without_losing_conversations() {
    let db = Database::new();
    let connection = rusqlite::Connection::open(&db.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE wrapper_conversations (
        account_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL,
        favorite INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        PRIMARY KEY(account_id, id)
    );
    INSERT INTO wrapper_conversations (account_id, id, title, created_at, updated_at)
    VALUES ('local-workspace', 'older', 'Nouvelle conversation', 123, 123);",
        )
        .unwrap();
    drop(connection);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["conversations"][0]["id"], "older");
    assert_eq!(
        send(&mut app, "older", "Titre après migration", false)["conversation"]["title"],
        "Titre après migration"
    );
}

impl Drop for Database {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = self.0.as_os_str().to_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(path));
        }
    }
}

#[test]
fn conversation_and_echo_are_readable_after_restart() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["mode"],
        "simulated"
    );
    let created = app
        .api("POST", "/api/conversations", br#"{"title":"Mon travail"}"#)
        .unwrap();
    let id = created["conversation"]["id"].as_str().unwrap().to_owned();
    let response = app
        .api(
            "POST",
            "/api/send",
            &serde_json::to_vec(&json!({
                "conversation_id": id, "message_id": "nonce-one", "body": "Bonjour"
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(response["conversation"]["messages"][0]["state"], "echoed");
    assert_eq!(
        response["conversation"]["messages"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(app);
    let mut reopened = App::open(&db.0).unwrap();
    let state = reopened.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["mode"], "simulated");
    assert_eq!(state["conversations"][0]["title"], "Mon travail");
    assert_eq!(state["conversations"][0]["messages"][0]["body"], "Bonjour");
    assert_eq!(
        state["conversations"][0]["messages"][1]["role"],
        "assistant"
    );
}

fn post(app: &mut App, path: &str, body: serde_json::Value) -> Result<serde_json::Value, u16> {
    app.api("POST", path, &serde_json::to_vec(&body).unwrap())
        .map_err(|error| error.status)
}

fn bot_ids(app: &mut App) -> Vec<String> {
    app.api("GET", "/api/state", b"").unwrap()["bots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|bot| bot["deleted"] == false)
        .map(|bot| bot["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn fresh_database_has_exactly_the_default_bot_at_its_template_version() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["bots"].as_array().unwrap().len(), 1);
    let bot = &state["bots"][0];
    assert_eq!(
        (
            bot["id"].clone(),
            bot["name"].clone(),
            bot["color"].clone(),
            bot["shape"].clone(),
            bot["is_default"].clone(),
            bot["template"].clone(),
            bot["customized"].clone(),
        ),
        (
            json!("grokbot"),
            json!("Ezerbot"),
            json!("citron"),
            json!("galet"),
            json!(true),
            json!("grokbot"),
            json!(false),
        )
    );
    let version: i64 = app
        .connection
        .query_row(
            "SELECT template_version FROM wrapper_bots WHERE id = 'grokbot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
    conversation(&mut app);
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["bot_id"],
        "grokbot"
    );
}

#[test]
fn default_bot_reconciliation_converges_and_never_overwrites_customization() {
    let db = Database::new();
    drop(App::open(&db.0).unwrap());
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(bot_ids(&mut app), ["grokbot"]);

    // Une copie non modifiée d'une ancienne version du template est mise à niveau.
    app.connection
        .execute(
            "UPDATE wrapper_bots SET name = 'Ancien', instructions = 'Vieux', template_version = 0 WHERE id = 'grokbot'",
            [],
        )
        .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let bot = app.api("GET", "/api/state", b"").unwrap()["bots"][0].clone();
    assert_eq!(bot["name"], "Ezerbot");
    assert_ne!(bot["instructions"], "Vieux");

    // Une copie personnalisée garde ses valeurs, même face à un template plus récent.
    post(
        &mut app,
        "/api/bots/update",
        json!({"id": "grokbot", "name": "Mon Grok", "instructions": "Parle en vers."}),
    )
    .unwrap();
    app.connection
        .execute(
            "UPDATE wrapper_bots SET template_version = 0 WHERE id = 'grokbot'",
            [],
        )
        .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["bots"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            state["bots"][0]["name"].clone(),
            state["bots"][0]["instructions"].clone(),
            state["bots"][0]["customized"].clone(),
        ),
        (json!("Mon Grok"), json!("Parle en vers."), json!(true))
    );
}

#[test]
fn main_bot_name_migration_is_idempotent_and_preserves_customization_and_workspace() {
    for (old_name, customized) in [
        ("Grokbot", false),
        ("Grokbot", true),
        ("Mon assistant", true),
    ] {
        let db = Database::new();
        let mut app = App::open(&db.0).unwrap();
        // Reconstitue une base de l'ancienne app, sans marque de migration.
        app.connection
            .execute("DROP TABLE wrapper_migrations", [])
            .unwrap();
        app.connection.execute(
            "UPDATE wrapper_bots SET name=?1, customized=?2, color='violet', shape='etoile',
             description='Ma présentation', instructions='Parle en vers.', updated_at=123 WHERE id='grokbot'",
            rusqlite::params![old_name, customized],
        ).unwrap();
        post(
            &mut app,
            "/api/bots",
            json!({"id":"other", "name":"Grokbot", "color":"cyan"}),
        )
        .unwrap();
        let conversation = conversation(&mut app);
        send(&mut app, &conversation, "Historique à conserver", false);
        post(
            &mut app,
            "/api/preferences",
            json!({"theme":"light", "accent":"violet"}),
        )
        .unwrap();
        let before = app.api("GET", "/api/state", b"").unwrap();
        drop(app);

        let mut app = App::open(&db.0).unwrap();
        let after = app.api("GET", "/api/state", b"").unwrap();
        let mut expected = before["bots"].clone();
        if old_name == "Grokbot" {
            expected[0]["name"] = json!("Ezerbot");
        }
        assert_eq!(after["bots"], expected);
        assert_eq!(after["conversations"], before["conversations"]);
        assert_eq!(after["preferences"], before["preferences"]);
        assert_eq!(
            app.connection
                .query_row(
                    "SELECT updated_at FROM wrapper_bots WHERE id='grokbot'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            123
        );
        drop(app);

        let mut app = App::open(&db.0).unwrap();
        assert_eq!(app.api("GET", "/api/state", b"").unwrap()["bots"], expected);
        // Une décision de nom prise après la migration ne doit plus être réécrite.
        post(
            &mut app,
            "/api/bots/update",
            json!({"id":"grokbot", "name":"Grokbot"}),
        )
        .unwrap();
        drop(app);
        let mut app = App::open(&db.0).unwrap();
        assert_eq!(
            app.api("GET", "/api/state", b"").unwrap()["bots"][0]["name"],
            "Grokbot"
        );
        assert_eq!(
            app.connection
                .query_row(
                    "SELECT COUNT(*) FROM wrapper_migrations WHERE id='ezerbot-name-v1'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}

#[test]
fn database_without_bot_column_is_migrated_to_the_default_bot() {
    let db = Database::new();
    let connection = rusqlite::Connection::open(&db.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE wrapper_conversations (
        account_id TEXT NOT NULL, id TEXT NOT NULL, title TEXT NOT NULL,
        title_customized INTEGER NOT NULL DEFAULT 0,
        favorite INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        PRIMARY KEY(account_id, id)
    );
    INSERT INTO wrapper_conversations (account_id, id, title, created_at, updated_at)
    VALUES ('local-workspace', 'avant-les-bots', 'Ancienne', 123, 123);",
        )
        .unwrap();
    drop(connection);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["conversations"][0]["id"], "avant-les-bots");
    assert_eq!(state["conversations"][0]["bot_id"], "grokbot");
    let reply = send(&mut app, "avant-les-bots", "Toujours là ?", false);
    assert_eq!(reply["conversation"]["messages"][0]["state"], "echoed");
}

#[test]
fn bot_creation_is_idempotent_on_its_client_id() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let request = json!({
        "id": "bot-1", "name": "  Juriste  ", "color": "violet",
        "description": "Relit vos contrats.", "instructions": "Cite toujours l'article."
    });
    let first = post(&mut app, "/api/bots", request.clone()).unwrap();
    let replay = post(&mut app, "/api/bots", request.clone()).unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        first["bot"],
        json!({
            "id": "bot-1", "name": "Juriste", "color": "violet", "shape": "galet",
            "description": "Relit vos contrats.", "instructions": "Cite toujours l'article.",
            "is_default": false, "template": null, "customized": false, "deleted": false,
        })
    );
    assert_eq!(bot_ids(&mut app), ["grokbot", "bot-1"]);
    let mut renamed = request.clone();
    renamed["name"] = json!("Notaire");
    assert_eq!(post(&mut app, "/api/bots", renamed), Err(409));
    let mut reshaped = request;
    reshaped["shape"] = json!("etoile");
    assert_eq!(post(&mut app, "/api/bots", reshaped), Err(409));
    assert_eq!(bot_ids(&mut app), ["grokbot", "bot-1"]);
}

#[test]
fn bot_boundaries_reject_invalid_input() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    for body in [
        json!({"id": "b", "name": "   ", "color": "citron"}),
        json!({"id": "b", "name": "x".repeat(41), "color": "citron"}),
        json!({"id": "b", "name": "Bot", "color": "terracotta"}),
        json!({"id": "b", "name": "Bot", "color": "citron", "avatar": "🙂"}),
        json!({"id": "b", "name": "Bot", "color": "citron", "description": "é".repeat(281)}),
        json!({"id": "b", "name": "Bot", "color": "citron", "instructions": "a".repeat(8_001)}),
        json!({"id": "pas valide", "name": "Bot", "color": "citron"}),
        json!({"id": "b", "name": "Bot", "color": "citron", "shape": "triangle"}),
        json!({"id": "b", "name": "Bot", "color": "citron", "shape": "Nuage"}),
    ] {
        assert_eq!(post(&mut app, "/api/bots", body), Err(400));
    }
    assert_eq!(
        post(&mut app, "/api/bots/update", json!({"id": "grokbot"})),
        Err(400)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/bots/update",
            json!({"id": "absent", "name": "X"})
        ),
        Err(404)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/bots/update",
            json!({"id": "grokbot", "shape": "cercle"})
        ),
        Err(400)
    );
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["bots"][0]["shape"],
        "galet"
    );
    assert_eq!(
        post(&mut app, "/api/conversations", json!({"bot_id": "absent"})),
        Err(404)
    );
    assert_eq!(bot_ids(&mut app), ["grokbot"]);
}

#[test]
fn deleting_a_bot_keeps_its_conversations_but_blocks_new_ones() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        post(&mut app, "/api/bots/delete", json!({"id": "grokbot"})),
        Err(409)
    );
    post(
        &mut app,
        "/api/bots",
        json!({"id": "chef", "name": "Chef", "color": "orange"}),
    )
    .unwrap();
    let created = post(&mut app, "/api/conversations", json!({"bot_id": "chef"})).unwrap();
    let id = created["conversation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(created["conversation"]["bot_id"], "chef");
    post(&mut app, "/api/bots/delete", json!({"id": "chef"})).unwrap();
    assert_eq!(bot_ids(&mut app), ["grokbot"]);
    let kept = app.api("GET", "/api/state", b"").unwrap()["bots"][1].clone();
    assert_eq!(
        (
            kept["id"].clone(),
            kept["name"].clone(),
            kept["color"].clone(),
            kept["deleted"].clone()
        ),
        (json!("chef"), json!("Chef"), json!("orange"), json!(true))
    );
    assert_eq!(
        post(&mut app, "/api/conversations", json!({"bot_id": "chef"})),
        Err(409)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/bots/update",
            json!({"id": "chef", "name": "X"})
        ),
        Err(409)
    );
    let reply = send(&mut app, &id, "Encore une recette", false);
    assert_eq!(reply["conversation"]["bot_id"], "chef");
    assert_eq!(reply["conversation"]["messages"][1]["role"], "assistant");
}

#[test]
fn bot_shape_is_created_read_back_and_updated() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let created = post(
        &mut app,
        "/api/bots",
        json!({"id": "nuageux", "name": "Nuageux", "color": "cyan", "shape": "nuage"}),
    )
    .unwrap();
    assert_eq!(created["bot"]["shape"], "nuage");
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let shapes = |app: &mut App| -> Vec<(String, String)> {
        app.api("GET", "/api/state", b"").unwrap()["bots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|bot| {
                (
                    bot["id"].as_str().unwrap().to_owned(),
                    bot["shape"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    assert_eq!(
        shapes(&mut app),
        [
            ("grokbot".to_owned(), "galet".to_owned()),
            ("nuageux".to_owned(), "nuage".to_owned())
        ]
    );
    let updated = post(
        &mut app,
        "/api/bots/update",
        json!({"id": "nuageux", "shape": "fantome"}),
    )
    .unwrap();
    assert_eq!(
        (
            updated["bot"]["shape"].clone(),
            updated["bot"]["color"].clone(),
            updated["bot"]["name"].clone()
        ),
        (json!("fantome"), json!("cyan"), json!("Nuageux"))
    );
    assert_eq!(
        shapes(&mut app)[1],
        ("nuageux".to_owned(), "fantome".to_owned())
    );
}

#[test]
fn database_without_shape_column_gives_every_bot_the_pebble() {
    let db = Database::new();
    let connection = rusqlite::Connection::open(&db.0).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE wrapper_bots (
        account_id TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, color TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '', instructions TEXT NOT NULL DEFAULT '',
        template TEXT, template_version INTEGER,
        customized INTEGER NOT NULL DEFAULT 0, deleted INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
        PRIMARY KEY (account_id, id)
    );
    INSERT INTO wrapper_bots (account_id, id, name, color, created_at, updated_at)
    VALUES ('local-workspace', 'ancien', 'Ancien', 'menthe', 1, 1);",
        )
        .unwrap();
    drop(connection);
    let mut app = App::open(&db.0).unwrap();
    let bots = app.api("GET", "/api/state", b"").unwrap()["bots"].clone();
    assert_eq!(
        bots.as_array()
            .unwrap()
            .iter()
            .map(|bot| (bot["id"].clone(), bot["shape"].clone()))
            .collect::<Vec<_>>(),
        [
            (json!("grokbot"), json!("galet")),
            (json!("ancien"), json!("galet"))
        ]
    );
}

/// Enregistre une proposition comme le ferait le flux, sur la réponse simulée d'un message.
fn propose(
    app: &mut App,
    conversation: &str,
    message: &str,
    call_id: &str,
    name: &str,
    arguments: serde_json::Value,
) -> Option<String> {
    super::record_proposal(
        &app.connection,
        conversation,
        message,
        &super::ToolCall {
            item_id: format!("fc_{call_id}"),
            call_id: call_id.into(),
            name: name.into(),
            arguments: arguments.to_string(),
        },
    )
    .unwrap()
}

fn proposals(app: &mut App, conversation: &str) -> Vec<serde_json::Value> {
    let state = app.api("GET", "/api/state", b"").unwrap();
    let found = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == conversation)
        .unwrap();
    found["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "assistant")
        .flat_map(|message| message["proposals"].as_array().cloned().unwrap_or_default())
        .collect()
}

const RELECTEUR: &str = r#"{"name":"Relecteur","shape":"nuage","color":"menthe","description":"Relit vos textes.","instructions":"Sois bref et précis.","reason":"Vous relisez souvent."}"#;

#[test]
#[ignore = "propositions de bots : chantier suspendu, test inachevé"]
fn model_arguments_are_validated_at_the_boundary_and_recorded_once_per_call_id() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "Crée un relecteur", false);
    let good: serde_json::Value = serde_json::from_str(RELECTEUR).unwrap();

    // Forme ou couleur inconnues, nom vide, outil inconnu, arguments illisibles : une note, pas de carte.
    let mut triangle = good.clone();
    triangle["shape"] = json!("triangle");
    let mut terracotta = good.clone();
    terracotta["color"] = json!("terracotta");
    let mut unnamed = good.clone();
    unnamed["name"] = json!("   ");
    for (call, name, arguments) in [
        ("call_1", "propose_bot", triangle),
        ("call_2", "propose_bot", terracotta),
        ("call_3", "propose_bot", unnamed),
        ("call_4", "propose_dog", good.clone()),
        (
            "call_5",
            "propose_bot_update",
            json!({"bot_id": "absent", "changes": {"name": "X"}, "reason": ""}),
        ),
        (
            "call_6",
            "propose_bot_update",
            json!({"bot_id": "grokbot", "changes": {"name": "Ezerbot"}, "reason": "rien"}),
        ),
    ] {
        let notice = propose(&mut app, &id, "stable-nonce", call, name, arguments);
        assert!(
            notice.as_deref().is_some_and(
                |notice| notice.starts_with("Une proposition de bot n'a pas été retenue")
            ),
            "{call}: {notice:?}"
        );
    }
    assert!(proposals(&mut app, &id).is_empty());
    let illegible = super::record_proposal(
        &app.connection,
        &id,
        "stable-nonce",
        &super::ToolCall {
            call_id: "call_7".into(),
            name: "propose_bot".into(),
            arguments: "{".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(illegible.unwrap().contains("illisibles"));

    // Un appel valide crée une proposition en attente ; le même call_id rejoué n'en crée pas de seconde.
    assert_eq!(
        propose(
            &mut app,
            &id,
            "stable-nonce",
            "call_ok",
            "propose_bot",
            good.clone()
        ),
        None
    );
    assert_eq!(
        propose(
            &mut app,
            &id,
            "stable-nonce",
            "call_ok",
            "propose_bot",
            good
        ),
        None
    );
    let listed = proposals(&mut app, &id);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], "call_ok");
    assert_eq!(listed[0]["kind"], "create");
    assert_eq!(listed[0]["status"], "pending");
    assert_eq!(listed[0]["reason"], "Vous relisez souvent.");
    assert_eq!(
        listed[0]["payload"],
        json!({"name": "Relecteur", "shape": "nuage", "color": "menthe", "description": "Relit vos textes.", "instructions": "Sois bref et précis."})
    );
    // L'historique rejoue la proposition en texte court, après la réponse.
    let history = app.history(&id, "stable-nonce").unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0], ("user", "Crée un relecteur".to_owned()));
    assert!(
        history[1]
            .1
            .ends_with("\n\n[Proposition de bot « Relecteur » : en attente]"),
        "{}",
        history[1].1
    );
}

#[test]
#[ignore = "propositions de bots : chantier suspendu, test inachevé"]
fn proposal_transitions_are_idempotent_and_refuse_a_dismissed_one() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "Crée un relecteur", false);
    let good: serde_json::Value = serde_json::from_str(RELECTEUR).unwrap();
    propose(
        &mut app,
        &id,
        "stable-nonce",
        "call_a",
        "propose_bot",
        good.clone(),
    );
    propose(&mut app, &id, "stable-nonce", "call_b", "propose_bot", good);
    assert_eq!(
        post(
            &mut app,
            "/api/proposals/accept",
            json!({"id": "call_absent"})
        ),
        Err(404)
    );

    // Accepter crée le bot avec un identifiant dérivé, et le rend tel quel au second clic.
    let first = post(&mut app, "/api/proposals/accept", json!({"id": "call_a"})).unwrap();
    assert_eq!(first["bot"]["id"], "proposition-call_a");
    assert_eq!(first["bot"]["name"], "Relecteur");
    assert_eq!(first["bot"]["shape"], "nuage");
    let replay = post(&mut app, "/api/proposals/accept", json!({"id": "call_a"})).unwrap();
    assert_eq!(replay["bot"], first["bot"]);
    assert_eq!(bot_ids(&mut app), ["grokbot", "proposition-call_a"]);
    let listed = proposals(&mut app, &id);
    assert_eq!(
        (
            listed[0]["status"].clone(),
            listed[0]["result_bot_id"].clone()
        ),
        (json!("accepted"), json!("proposition-call_a"))
    );
    assert_eq!(
        post(&mut app, "/api/proposals/dismiss", json!({"id": "call_a"})),
        Err(409)
    );

    // Écarter, écarter encore, puis accepter : 409.
    eprintln!("AVANT {:?}", proposals(&mut app, &id));
    let rows: Vec<(String, String, String)> = app
        .connection
        .prepare("SELECT id, status, message_id FROM wrapper_proposals")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    eprintln!("ROWS {rows:?}");
    post(&mut app, "/api/proposals/dismiss", json!({"id": "call_b"})).unwrap();
    post(&mut app, "/api/proposals/dismiss", json!({"id": "call_b"})).unwrap();
    eprintln!("APRES {:?}", proposals(&mut app, &id));
    assert_eq!(proposals(&mut app, &id)[1]["status"], "dismissed");
    assert_eq!(
        post(&mut app, "/api/proposals/accept", json!({"id": "call_b"})),
        Err(409)
    );
    assert_eq!(bot_ids(&mut app), ["grokbot", "proposition-call_a"]);
    let history = app.history(&id, "stable-nonce").unwrap();
    assert!(history[1].1.ends_with("[Proposition de bot « Relecteur » : acceptée]\n\n[Proposition de bot « Relecteur » : refusée]"), "{}", history[1].1);

    // « Modifier avant » : les valeurs retenues remplacent celles proposées, avec les mêmes bornes.
    let good: serde_json::Value = serde_json::from_str(RELECTEUR).unwrap();
    propose(&mut app, &id, "stable-nonce", "call_c", "propose_bot", good);
    assert_eq!(
        post(
            &mut app,
            "/api/proposals/accept",
            json!({"id": "call_c", "overrides": {"shape": "cercle"}})
        ),
        Err(400)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/proposals/accept",
            json!({"id": "call_c", "overrides": {"avatar": "x"}})
        ),
        Err(400)
    );
    let edited = post(
        &mut app,
        "/api/proposals/accept",
        json!({"id": "call_c", "overrides": {"name": "Correcteur", "color": "violet"}}),
    )
    .unwrap()["bot"]
        .clone();
    assert_eq!(
        (
            edited["name"].clone(),
            edited["color"].clone(),
            edited["shape"].clone(),
            edited["instructions"].clone()
        ),
        (
            json!("Correcteur"),
            json!("violet"),
            json!("nuage"),
            json!("Sois bref et précis.")
        )
    );
}

#[test]
#[ignore = "propositions de bots : chantier suspendu, test inachevé"]
fn update_proposal_applies_the_changes_or_goes_stale_when_the_bot_moves() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    post(
        &mut app,
        "/api/bots",
        json!({"id": "chef", "name": "Chef", "color": "orange", "description": "Cuisine."}),
    )
    .unwrap();
    let id = conversation(&mut app);
    send(&mut app, &id, "Rends le chef plus doux", false);
    let update = |changes: serde_json::Value| json!({"bot_id": "chef", "changes": changes, "reason": "Plus doux."});
    // Les champs égaux à la valeur actuelle sont retirés, et `before` mémorise les autres.
    assert_eq!(
        propose(
            &mut app,
            &id,
            "stable-nonce",
            "call_u1",
            "propose_bot_update",
            update(
                json!({"name": "Chef", "shape": null, "color": null, "description": "Cuisine avec douceur.", "instructions": "Tutoie."})
            )
        ),
        None
    );
    let listed = proposals(&mut app, &id);
    assert_eq!(listed[0]["kind"], "update");
    assert_eq!(listed[0]["bot_id"], "chef");
    assert_eq!(
        listed[0]["payload"],
        json!({"changes": {"description": "Cuisine avec douceur.", "instructions": "Tutoie."}, "before": {"description": "Cuisine.", "instructions": ""}})
    );
    assert!(app.history(&id, "stable-nonce").unwrap()[1]
        .1
        .ends_with("[Proposition de modification du bot « chef » : en attente]"));
    let applied =
        post(&mut app, "/api/proposals/accept", json!({"id": "call_u1"})).unwrap()["bot"].clone();
    assert_eq!(
        (
            applied["description"].clone(),
            applied["instructions"].clone(),
            applied["customized"].clone()
        ),
        (
            json!("Cuisine avec douceur."),
            json!("Tutoie."),
            json!(true)
        )
    );
    // Le bot a changé depuis : la même proposition relue reste acceptée, pas périmée.
    assert_eq!(proposals(&mut app, &id)[0]["status"], "accepted");

    // Une seconde proposition se périme dès qu'un champ visé change ailleurs, et ne s'applique plus.
    propose(
        &mut app,
        &id,
        "stable-nonce",
        "call_u2",
        "propose_bot_update",
        update(
            json!({"name": "Chef pâtissier", "shape": null, "color": null, "description": null, "instructions": null}),
        ),
    );
    post(
        &mut app,
        "/api/bots/update",
        json!({"id": "chef", "color": "ambre"}),
    )
    .unwrap();
    assert_eq!(
        proposals(&mut app, &id)[1]["status"],
        "pending",
        "un champ non visé ne périme pas"
    );
    post(
        &mut app,
        "/api/bots/update",
        json!({"id": "chef", "name": "Cheffe"}),
    )
    .unwrap();
    assert_eq!(proposals(&mut app, &id)[1]["status"], "stale");
    assert_eq!(
        post(&mut app, "/api/proposals/accept", json!({"id": "call_u2"})),
        Err(409)
    );
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["bots"][1]["name"],
        "Cheffe"
    );

    // La suppression du bot périme aussi.
    propose(
        &mut app,
        &id,
        "stable-nonce",
        "call_u3",
        "propose_bot_update",
        update(
            json!({"name": null, "shape": "etoile", "color": null, "description": null, "instructions": null}),
        ),
    );
    post(&mut app, "/api/bots/delete", json!({"id": "chef"})).unwrap();
    assert_eq!(proposals(&mut app, &id)[2]["status"], "stale");
    assert!(app.history(&id, "stable-nonce").unwrap()[1]
        .1
        .ends_with("[Proposition de modification du bot « chef » : périmée]"));
}

#[test]
#[ignore = "propositions de bots : désactivées par PROPOSALS_ENABLED"]
fn only_the_default_bot_gets_the_proposal_tools_and_the_roster() {
    let default = super::Bot {
        id: "grokbot".into(),
        name: "Ezerbot".into(),
        color: "citron".into(),
        shape: "galet".into(),
        description: String::new(),
        instructions: String::new(),
        is_default: true,
        template: None,
        customized: false,
        deleted: false,
    };
    let other = super::Bot {
        id: "chef".into(),
        name: "Chef".into(),
        is_default: false,
        description: "Cuisine.".into(),
        ..default.clone()
    };
    let gone = super::Bot {
        id: "parti".into(),
        name: "Parti".into(),
        is_default: false,
        deleted: true,
        ..default.clone()
    };
    assert!(super::features::engine::tools_for(&other).is_empty());
    let tools = super::features::engine::tools_for(&default);
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["propose_bot", "propose_bot_update"]);
    for tool in &tools {
        assert_eq!(
            (tool["type"].clone(), tool["strict"].clone()),
            (json!("function"), json!(true))
        );
        assert_eq!(tool["parameters"]["additionalProperties"], false);
    }
    let create = &tools[0]["parameters"];
    assert_eq!(
        create["required"],
        json!([
            "name",
            "shape",
            "color",
            "description",
            "instructions",
            "reason"
        ])
    );
    assert_eq!(create["properties"]["shape"]["enum"], json!(super::SHAPES));
    assert_eq!(create["properties"]["color"]["enum"], json!(super::ACCENTS));
    let update = &tools[1]["parameters"];
    assert_eq!(update["required"], json!(["bot_id", "changes", "reason"]));
    assert_eq!(
        update["properties"]["changes"]["additionalProperties"],
        false
    );
    assert_eq!(
        update["properties"]["changes"]["required"],
        json!(["name", "shape", "color", "description", "instructions"])
    );
    assert_eq!(
        update["properties"]["changes"]["properties"]["shape"]["anyOf"][0]["enum"],
        json!(super::SHAPES)
    );
    assert_eq!(
        update["properties"]["changes"]["properties"]["name"]["anyOf"][1],
        json!({"type": "null"})
    );

    let instructions = super::features::engine::compose_instructions(
        &default,
        &[default.clone(), other.clone(), gone],
    );
    assert!(instructions.contains("propose_bot_update"));
    assert!(instructions.ends_with("Bots existants (identifiant : nom — présentation) :\n- grokbot : Ezerbot\n- chef : Chef — Cuisine."), "{instructions}");
    assert!(
        !super::features::engine::compose_instructions(&other, &[default]).contains("propose_bot")
    );
}
