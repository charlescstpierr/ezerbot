use std::path::PathBuf;

use grokbot_domain::{ConversationId, MessageId};
use grokbot_session::SubmissionTransport;
use serde_json::json;

use super::{validate_headers, App};

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

/// Ce que `/api/send` valide en une transaction avant le journal : capture
/// multibot, ancre éventuelle, entrée du fil, pièces jointes. Puis le crash.
fn crash_after_commit_before_prepare(app: App, id: &str, message: &str, passage: Option<&str>) {
    let tx = super::features::write_transaction(&app.connection).unwrap();
    super::features::multibot::capture(&tx, id, message, &[], "Retouche", false).unwrap();
    if let Some(passage) = passage {
        tx.execute(
            "INSERT INTO wrapper_page_anchors VALUES (?1, ?2, 'page_one', ?3, ?4, 'open', 1)",
            rusqlite::params![id, message, passage, "0".repeat(64)],
        )
        .unwrap();
    }
    super::features::conversations::append_entry(&tx, id, "submission", message, 1).unwrap();
    grokbot_attachments::staging::link_attachments(&tx, id, message, &["image-a".into()]).unwrap();
    tx.commit().unwrap();
    drop(app);
}

fn anchor_rows(app: &App) -> Vec<(String, String)> {
    let mut statement = app
        .connection
        .prepare("SELECT message_id, passage FROM wrapper_page_anchors ORDER BY message_id")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn anchor(passage: &str) -> serde_json::Value {
    json!({"page_id": "page_one", "source_version": "0".repeat(64), "passage": passage})
}

fn resend(id: &str, message: &str, anchor: Option<serde_json::Value>) -> serde_json::Value {
    let mut body = json!({"conversation_id": id, "message_id": message, "body": "Retouche", "attachments": ["image-a"]});
    if let Some(anchor) = anchor {
        body["page_anchor"] = anchor;
    }
    body
}

/// Un envoi ancré (« ancre ») et un envoi sans ancre (« sans-ancre ») durables, journal vide.
fn crashed_before_journal(db: &Database) -> (App, String) {
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    app.connection
        .execute(
            "INSERT INTO bot_pages (id, bot_id, conversation_id, title, filename, source, status, diagnostic, document, preview, created_at, updated_at)
             VALUES ('page_one', 'grokbot', ?1, 'Forfaits', 'forfaits.page.tsx', '', 'ready', NULL, NULL, 'Essentiel Confort Famille', 1, 1)",
            [&id],
        )
        .unwrap();
    crash_after_commit_before_prepare(app, &id, "ancre", Some("Confort"));
    let app = App::open(&db.0).unwrap();
    crash_after_commit_before_prepare(app, &id, "sans-ancre", None);
    let app = App::open(&db.0).unwrap();
    assert!(app.reader.list_submissions().unwrap().is_empty());
    (app, id)
}

#[test]
fn identical_resend_after_crash_before_journal_prepare_is_idempotent() {
    let db = Database::new();
    let (mut app, id) = crashed_before_journal(&db);
    let durable = anchor_rows(&app);
    assert_eq!(durable, [("ancre".to_owned(), "Confort".to_owned())]);
    super::features::test_hooks::with_link(attachment_link_hook, || {
        for (message, sent) in [("ancre", Some(anchor(" Confort "))), ("sans-ancre", None)] {
            let result = post(&mut app, "/api/send", resend(&id, message, sent)).unwrap();
            assert!(result["conversation"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["id"] == message && m["state"] == "echoed"));
            assert_eq!(anchor_rows(&app), durable);
            assert_eq!(
                grokbot_attachments::staging::for_message(&app.connection, &id, message)
                    .unwrap()
                    .len(),
                1
            );
        }
    });
    assert_eq!(app.reader.list_submissions().unwrap().len(), 2);
}

#[test]
fn resend_after_crash_before_journal_prepare_cannot_add_remove_or_change_the_anchor() {
    let db = Database::new();
    let (mut app, id) = crashed_before_journal(&db);
    let durable = anchor_rows(&app);
    super::features::test_hooks::with_link(attachment_link_hook, || {
        for (message, sent) in [
            ("sans-ancre", Some(anchor("Confort"))),
            ("ancre", None),
            ("ancre", Some(anchor("Famille"))),
        ] {
            assert_eq!(
                post(&mut app, "/api/send", resend(&id, message, sent)),
                Err(409)
            );
            assert!(app.reader.list_submissions().unwrap().is_empty());
            assert_eq!(anchor_rows(&app), durable);
        }
    });
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

/// Chemin de la base, puis le dossier unique qui la contient : `-wal`, `-shm`,
/// `.secrets.json`, `.lock` et `logs/` y vivent aussi et partent avec lui.
struct Database(PathBuf, #[allow(dead_code)] grokbot_test_support::TestDir);

impl Database {
    fn new() -> Self {
        let directory = grokbot_test_support::TestDir::new("grokbot-wrapper");
        Self(directory.join("workspace.sqlite"), directory)
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
    assert!(first["bot"]["created_at"].as_u64().unwrap() > 0);
    assert_eq!(
        first["bot"],
        json!({
            "id": "bot-1", "name": "Juriste", "color": "violet", "shape": "galet",
            "description": "Relit vos contrats.", "instructions": "Cite toujours l'article.",
            "is_default": false, "template": null, "customized": false, "deleted": false,
            "created_at": first["bot"]["created_at"],
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

fn find_message<'a>(conversation: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    conversation["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("message {id} absent"))
}

fn remote_sends(path: &std::path::Path, conversation: &str, message: &str) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT send_count FROM wrapper_remote WHERE account_id = 'local-workspace' AND conversation_id = ?1 AND message_id = ?2",
            [conversation, message],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn user_copies(conversation: &serde_json::Value, body: &str) -> usize {
    conversation["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user" && message["body"] == body)
        .count()
}

/// Réaction persistante, un envoi après Renvoyer, zéro après Supprimer,
/// et aucune de ces deux actions sur un message déjà livré.
#[test]
fn reaction_persistante_renvoi_unique_et_suppression_sans_envoi() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);

    let livre = post(
        &mut app,
        "/api/send",
        json!({"conversation_id": id, "message_id": "livre", "body": "Déjà livré"}),
    )
    .unwrap();
    assert_eq!(
        find_message(&livre["conversation"], "livre")["state"],
        "echoed"
    );
    assert_eq!(remote_sends(&db.0, &id, "livre"), 1);

    let reacted = post(
        &mut app,
        "/api/messages/react",
        json!({"conversation_id": id, "message_id": "livre", "emoji": "👍"}),
    )
    .unwrap();
    assert_eq!(
        find_message(&reacted["conversation"], "livre")["payload"]["reaction"],
        "👍"
    );
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        find_message(&state["conversations"][0], "livre")["payload"]["reaction"],
        "👍"
    );
    let cleared = post(
        &mut app,
        "/api/messages/react",
        json!({"conversation_id": id, "message_id": "livre", "emoji": "👍"}),
    )
    .unwrap();
    assert!(find_message(&cleared["conversation"], "livre")["payload"]
        .get("reaction")
        .is_none());
    post(
        &mut app,
        "/api/messages/react",
        json!({"conversation_id": id, "message_id": "livre", "emoji": "❤️"}),
    )
    .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        find_message(&state["conversations"][0], "livre")["payload"]["reaction"],
        "❤️"
    );
    assert_eq!(
        post(
            &mut app,
            "/api/messages/resend",
            json!({"conversation_id": id, "message_id": "livre"})
        ),
        Err(409)
    );
    assert_eq!(
        post(
            &mut app,
            "/api/messages/delete",
            json!({"conversation_id": id, "message_id": "livre"})
        ),
        Err(409)
    );
    assert_eq!(remote_sends(&db.0, &id, "livre"), 1);

    let tx = super::features::write_transaction(&app.connection).unwrap();
    grokbot_attachments::staging::link_attachments(&tx, &id, "coupe", &["image-a".into()]).unwrap();
    super::features::conversations::append_entry(&tx, &id, "submission", "coupe", 2).unwrap();
    tx.commit().unwrap();
    app.session
        .prepare(
            ConversationId::new(&id).unwrap(),
            MessageId::new("coupe").unwrap(),
            "Phrase coupée".into(),
        )
        .unwrap();
    assert_eq!(remote_sends(&db.0, &id, "coupe"), 0);
    let resent = post(
        &mut app,
        "/api/messages/resend",
        json!({"conversation_id": id, "message_id": "coupe"}),
    )
    .unwrap();
    let coupe = find_message(&resent["conversation"], "coupe");
    assert_eq!(coupe["state"], "echoed");
    assert_eq!(coupe["body"], "Phrase coupée");
    assert_eq!(coupe["attachments"][0]["id"], "image-a");
    assert_eq!(remote_sends(&db.0, &id, "coupe"), 1);
    assert_eq!(user_copies(&resent["conversation"], "Phrase coupée"), 1);
    assert_eq!(
        post(
            &mut app,
            "/api/messages/resend",
            json!({"conversation_id": id, "message_id": "coupe"})
        ),
        Err(409)
    );
    assert_eq!(remote_sends(&db.0, &id, "coupe"), 1);

    let cut = post(
        &mut app,
        "/api/send",
        json!({
            "conversation_id": id,
            "message_id": "coupure",
            "body": "Coupure réseau",
            "attachments": ["image-b"],
            "simulate_loss": true
        }),
    )
    .unwrap();
    let coupure = find_message(&cut["conversation"], "coupure");
    assert_eq!(coupure["state"], "unknown");
    assert_eq!(coupure["attachments"][0]["id"], "image-b");
    assert_eq!(remote_sends(&db.0, &id, "coupure"), 1);
    let replayed = post(
        &mut app,
        "/api/messages/resend",
        json!({"conversation_id": id, "message_id": "coupure"}),
    )
    .unwrap();
    let coupure = find_message(&replayed["conversation"], "coupure");
    assert_eq!(coupure["state"], "echoed");
    assert_eq!(coupure["attachments"][0]["id"], "image-b");
    assert_eq!(user_copies(&replayed["conversation"], "Coupure réseau"), 1);
    assert_eq!(remote_sends(&db.0, &id, "coupure"), 1);
    assert_eq!(
        post(
            &mut app,
            "/api/messages/resend",
            json!({"conversation_id": id, "message_id": "coupure"})
        ),
        Err(409)
    );
    assert_eq!(remote_sends(&db.0, &id, "coupure"), 1);

    let forgotten = post(
        &mut app,
        "/api/send",
        json!({
            "conversation_id": id,
            "message_id": "oublie",
            "body": "À oublier",
            "simulate_loss": true
        }),
    )
    .unwrap();
    assert_eq!(
        find_message(&forgotten["conversation"], "oublie")["state"],
        "unknown"
    );
    assert_eq!(remote_sends(&db.0, &id, "oublie"), 1);
    let deleted = post(
        &mut app,
        "/api/messages/delete",
        json!({"conversation_id": id, "message_id": "oublie"}),
    )
    .unwrap();
    assert!(deleted["conversation"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["id"] != "oublie"));
    assert_eq!(remote_sends(&db.0, &id, "oublie"), 0);

    app.session
        .prepare(
            ConversationId::new(&id).unwrap(),
            MessageId::new("efface").unwrap(),
            "À effacer".into(),
        )
        .unwrap();
    assert_eq!(remote_sends(&db.0, &id, "efface"), 0);
    let removed = post(
        &mut app,
        "/api/messages/delete",
        json!({"conversation_id": id, "message_id": "efface"}),
    )
    .unwrap();
    assert!(removed["conversation"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["id"] != "efface"));
    assert_eq!(remote_sends(&db.0, &id, "efface"), 0);

    drop(app);
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    let messages = &state["conversations"][0];
    assert_eq!(find_message(messages, "livre")["payload"]["reaction"], "❤️");
    assert!(messages["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["id"] != "oublie" && message["id"] != "efface"));
    assert_eq!(remote_sends(&db.0, &id, "oublie"), 0);
    assert_eq!(remote_sends(&db.0, &id, "coupe"), 1);
}

/// Supprimer un envoi non livré retire ses liens de pièces jointes. Les objets
/// et les liens d’un autre message restent, pour que le nettoyage des orphelins
/// puisse voir un fichier qui n’a plus aucun message.
#[test]
fn suppression_retire_les_liens_de_pieces_du_message_non_envoye() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let id = conversation(&mut app);
    seed_attachments(&app);
    for document in ["document_shared", "document_only"] {
        let record = serde_json::json!({
            "id": document,
            "name": "note.txt",
            "format": "txt",
            "byte_length": 4,
            "page_count": 1,
            "page_kind": "pages estimées",
            "pages_read": 1,
            "truncated": false,
            "note": null,
            "text": "note"
        })
        .to_string();
        app.connection
            .execute(
                "INSERT INTO attachments_documents(id, record) VALUES (?1, ?2)",
                rusqlite::params![document, record],
            )
            .unwrap();
    }

    let kept = post(
        &mut app,
        "/api/send",
        json!({
            "conversation_id": id,
            "message_id": "garde",
            "body": "Conservé",
            "attachments": ["image-a", "document_shared"]
        }),
    )
    .unwrap();
    assert_eq!(
        find_message(&kept["conversation"], "garde")["state"],
        "echoed"
    );

    let unsent = post(
        &mut app,
        "/api/send",
        json!({
            "conversation_id": id,
            "message_id": "efface",
            "body": "Pas encore parti",
            "attachments": ["image-a", "image-b", "document_shared", "document_only"],
            "simulate_loss": true
        }),
    )
    .unwrap();
    assert_eq!(
        find_message(&unsent["conversation"], "efface")["state"],
        "unknown"
    );
    assert_eq!(remote_sends(&db.0, &id, "efface"), 1);
    assert_eq!(
        link_rows(&app, "attachments_message_links", &id, "efface"),
        2
    );
    assert_eq!(
        link_rows(&app, "attachments_document_links", &id, "efface"),
        2
    );
    assert_eq!(
        link_rows(&app, "wrapper_attachment_order", &id, "efface"),
        4
    );

    let deleted = post(
        &mut app,
        "/api/messages/delete",
        json!({"conversation_id": id, "message_id": "efface"}),
    )
    .unwrap();
    assert!(deleted["conversation"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["id"] != "efface"));
    assert_eq!(remote_sends(&db.0, &id, "efface"), 0);
    for table in [
        "attachments_message_links",
        "attachments_document_links",
        "wrapper_attachment_order",
    ] {
        assert_eq!(link_rows(&app, table, &id, "efface"), 0, "{table}");
    }
    assert_eq!(
        link_rows(&app, "attachments_message_links", &id, "garde"),
        1
    );
    assert_eq!(
        link_rows(&app, "attachments_document_links", &id, "garde"),
        1
    );
    assert_eq!(link_rows(&app, "wrapper_attachment_order", &id, "garde"), 2);
    for (table, attachment) in [
        ("attachments_staged", "image-a"),
        ("attachments_staged", "image-b"),
        ("attachments_documents", "document_shared"),
        ("attachments_documents", "document_only"),
    ] {
        let present: bool = app
            .connection
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1)"),
                [attachment],
                |row| row.get(0),
            )
            .unwrap();
        assert!(present, "{attachment}");
    }
    let roots = attachment_roots(&app);
    assert!(roots.contains("image-a"));
    assert!(roots.contains("document_shared"));
    assert!(!roots.contains("image-b"));
    assert!(!roots.contains("document_only"));
    let kept = find_message(&deleted["conversation"], "garde");
    assert_eq!(kept["attachments"][0]["id"], "image-a");
}

fn link_rows(app: &App, table: &str, conversation: &str, message: &str) -> i64 {
    app.connection
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE conversation_id = ?1 AND message_id = ?2"),
            rusqlite::params![conversation, message],
            |row| row.get(0),
        )
        .unwrap()
}

fn attachment_roots(app: &App) -> std::collections::HashSet<String> {
    let mut statement = app
        .connection
        .prepare(
            "SELECT attachment_id FROM wrapper_storage_drafts
             UNION SELECT attachment_id FROM attachments_message_links
             UNION SELECT document_id FROM attachments_document_links
             UNION SELECT attachment_id FROM wrapper_attachment_order",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

#[test]
fn connector_install_text_is_served_once_not_with_every_state() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let state = app.api("GET", "/api/state", b"").unwrap();
    let cards = state["mcp"]["catalogue"].as_array().unwrap();
    assert!(cards.len() >= 10);
    for card in cards {
        for key in ["setup", "scopes_label", "license", "source"] {
            assert!(
                card.get(key).is_none(),
                "{} still carries {key}",
                card["id"]
            );
        }
    }
    let served = app.api("POST", "/api/connectors/details", b"{}").unwrap();
    let details = served["details"].as_array().unwrap();
    // Every card of the state has its install text, none is empty.
    for card in cards {
        let id = card["id"].as_str().unwrap();
        let found = details
            .iter()
            .find(|d| d["id"] == id)
            .unwrap_or_else(|| panic!("no details for {id}"));
        for key in ["setup", "license", "source"] {
            assert!(
                found[key].as_str().is_some_and(|text| !text.is_empty()),
                "{id} has no {key}"
            );
        }
    }
    let by = |id: &str| details.iter().find(|d| d["id"] == id).unwrap();
    assert_eq!(
        by("slack")["scopes_label"],
        "Lecture des canaux et publication de messages, choisies pendant le consentement Slack"
    );
    assert_eq!(
        by("linear")["scopes_label"],
        "read issues:create comments:create"
    );
}

/// Two replies of the history that both quote `secret`, as a tool output renewed meanwhile
/// would (#377). The vault does not hold it yet.
fn history_quoting(app: &mut App, secret: &str) -> String {
    use grokbot_engine::{
        contracts::{RunEventKind, Trigger},
        store,
    };
    let id = conversation(app);
    for message in ["premier", "second"] {
        post(
            app,
            "/api/send",
            json!({"conversation_id": id, "message_id": message, "body": "Question"}),
        )
        .unwrap();
        let trigger = Trigger::UserMessage {
            message_id: format!("{id}/{message}"),
            attempt: 0,
        };
        let run = store::latest_run_for_trigger(&app.connection, &trigger)
            .unwrap()
            .expect("the message started a run");
        // The run has ended (no provider here): record its text as it had streamed it.
        app.connection
            .execute(
                "INSERT INTO engine_run_events(run_id,seq,kind,payload)
                 SELECT ?1, COALESCE(MAX(seq),0)+1, ?2, ?3 FROM engine_run_events WHERE run_id=?1",
                rusqlite::params![
                    run.run_id,
                    serde_json::to_value(RunEventKind::TextDelta)
                        .unwrap()
                        .as_str(),
                    json!({"text": format!("Le jeton renouvelé est {secret}."), "response": 1})
                        .to_string()
                ],
            )
            .unwrap();
    }
    id
}

fn replies(conversations: &[super::Conversation]) -> Vec<String> {
    conversations
        .iter()
        .flat_map(|c| &c.messages)
        .filter(|m| m.role == "assistant")
        .map(|m| serde_json::to_string(m).unwrap())
        .collect()
}

#[test]
fn a_secret_stored_while_the_replies_are_masked_never_leaves_in_clear() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let secret = "jeton-oauth-renouvele-pendant-la-lecture";
    history_quoting(&mut app, secret);
    let vault = super::features::engine::event_vault(&app.connection).unwrap();
    // Synchronisation point: right after the first reply is masked with the snapshot taken
    // for it, a tool worker stores the renewed token, before the second reply is masked.
    let mut stored = false;
    let conversations = app
        .conversations_settled(None, &mut || {
            if !stored {
                vault.set("OAUTH_RENOUVELE", "", secret).unwrap();
                stored = true;
            }
        })
        .unwrap();
    assert!(stored, "the vault changed during the projection");
    let replies = replies(&conversations);
    assert_eq!(replies.len(), 2);
    for reply in &replies {
        assert!(!reply.contains(secret), "secret in clear: {reply}");
        assert!(reply.contains("[...]"), "{reply}");
    }
    // A vault that changes after every reply never settles: the read is refused, not served
    // with a stale snapshot.
    let mut renewals = 0;
    let refused = app
        .conversations_settled(None, &mut || {
            renewals += 1;
            vault
                .set("OAUTH_RENOUVELE", "", &format!("{secret}-{renewals}"))
                .unwrap();
        })
        .unwrap_err();
    assert_eq!(refused.status, 503);
}

#[test]
fn a_send_response_projecting_one_conversation_is_masked_with_a_settled_vault() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let secret = "jeton-oauth-renouvele-pendant-la-reponse";
    let id = history_quoting(&mut app, secret);
    let vault = super::features::engine::event_vault(&app.connection).unwrap();
    let mut stored = false;
    let conversations = app
        .conversations_settled(Some(&id), &mut || {
            if !stored {
                vault.set("OAUTH_RENOUVELE", "", secret).unwrap();
                stored = true;
            }
        })
        .unwrap();
    assert!(stored, "the vault changed during the projection");
    assert_eq!(conversations.len(), 1);
    let replies = replies(&conversations);
    assert_eq!(replies.len(), 2);
    for reply in &replies {
        assert!(!reply.contains(secret), "secret in clear: {reply}");
        assert!(reply.contains("[...]"), "{reply}");
    }
}

/// A conversation without any direct reply (#377): one routine reply (`engine_reply`) and one
/// group reply (`group_reply`), both quoting `secret`, which the vault does not hold yet.
fn routine_and_group_quoting(app: &mut App, secret: &str) -> String {
    use grokbot_engine::{
        contracts::{RunEventKind, RunKind, TaskStatus, Trigger},
        model::{self, RunStatus},
        store,
    };
    // A sent message gives a complete run to copy. Its conversation is then deleted: the
    // state read below projects no direct reply at all.
    let source = history_quoting(app, "pas-un-secret");
    let template = store::latest_run_for_trigger(
        &app.connection,
        &Trigger::UserMessage {
            message_id: format!("{source}/premier"),
            attempt: 0,
        },
    )
    .unwrap()
    .expect("the message started a run");
    app.connection
        .execute(
            "UPDATE wrapper_conversations SET deleted=1 WHERE id=?1",
            [&source],
        )
        .unwrap();
    let id = conversation(app);
    for (kind, entry, run_id) in [
        ("engine_reply", "routine-du-matin", "routine-du-matin"),
        ("group_reply", "tour-de-groupe", "run-du-tour"),
    ] {
        let mut run = template.clone();
        run.run_id = run_id.into();
        run.conversation_id = id.clone();
        run.status = RunStatus::Queued;
        run.turn_end = None;
        run.task_status = None;
        if kind == "engine_reply" {
            run.trigger = Trigger::Occurrence {
                occurrence_id: format!("occurrence-{run_id}"),
            };
            run.kind = RunKind::Task;
            run.task_status = Some(TaskStatus::Working);
        } else {
            run.trigger = Trigger::UserMessage {
                message_id: format!("{id}/{entry}"),
                attempt: 0,
            };
        }
        let tx = app.connection.unchecked_transaction().unwrap();
        model::start(&tx, &run).unwrap();
        tx.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) SELECT ?1,input_json,0,'simulated' FROM wrapper_engine_runs WHERE run_id=?2",rusqlite::params![run_id, template.run_id]).unwrap();
        tx.execute(
            "INSERT INTO engine_run_events(run_id,seq,kind,payload)
             SELECT ?1, COALESCE(MAX(seq),0)+1, ?2, ?3 FROM engine_run_events WHERE run_id=?1",
            rusqlite::params![
                run_id,
                serde_json::to_value(RunEventKind::TextDelta)
                    .unwrap()
                    .as_str(),
                json!({"text": format!("Le jeton renouvelé est {secret}."), "response": 1})
                    .to_string()
            ],
        )
        .unwrap();
        if kind == "group_reply" {
            tx.execute("INSERT INTO wrapper_group_turns(account_id,conversation_id,id,root_id,bot_id,cause_id,prompt,priority,status,run_id) VALUES (?1,?2,?3,'racine',?4,'cause','Avis',1,'completed',?5)",rusqlite::params![super::ACCOUNT, id, entry, run.bot_id, run_id]).unwrap();
        }
        super::features::conversations::append_entry(&tx, &id, kind, entry, 1).unwrap();
        tx.commit().unwrap();
    }
    id
}

#[test]
fn a_secret_stored_while_routine_and_group_replies_are_masked_never_leaves_in_clear() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let secret = "jeton-oauth-renouvele-pendant-la-routine";
    let id = routine_and_group_quoting(&mut app, secret);
    let vault = super::features::engine::event_vault(&app.connection).unwrap();
    let state = |conversations: &[super::Conversation]| {
        let conversation = conversations
            .iter()
            .find(|c| c.id == id)
            .expect("the conversation is listed");
        assert!(
            conversations
                .iter()
                .flat_map(|c| &c.messages)
                .all(|m| m.role == "assistant"),
            "no message was sent, hence no direct reply"
        );
        conversation
            .messages
            .iter()
            .map(|m| serde_json::to_string(m).unwrap())
            .collect::<Vec<_>>()
    };
    // Without a vault change, both replies are projected and quote the secret in clear:
    // the fixture really renders the quoted text through both paths.
    let plain = state(&app.conversations().unwrap());
    assert_eq!(plain.len(), 2);
    assert!(
        plain.iter().all(|reply| reply.contains(secret)),
        "{plain:?}"
    );
    // Synchronisation point: right after the routine reply is masked, a tool worker stores the
    // renewed token, before the group reply is masked.
    let mut stored = false;
    let conversations = app
        .conversations_settled(None, &mut || {
            if !stored {
                vault.set("OAUTH_RENOUVELE", "", secret).unwrap();
                stored = true;
            }
        })
        .unwrap();
    assert!(stored, "the vault changed during the projection");
    let replies = state(&conversations);
    assert_eq!(replies.len(), 2);
    for reply in &replies {
        assert!(!reply.contains(secret), "secret in clear: {reply}");
        assert!(reply.contains("[...]"), "{reply}");
    }
    let state = app.api("GET", "/api/state", b"").unwrap().to_string();
    assert!(!state.contains(secret));
}

/// `/api/state` looks up the side tables of the whole history at once (#515); one conversation
/// read looks them up message by message. Both must project the same history, whichever side
/// tables hold rows for it.
#[test]
fn a_state_read_projects_the_history_like_a_conversation_read_message_by_message() {
    let db = Database::new();
    let mut app = App::open(&db.0).unwrap();
    let chat = conversation(&mut app);
    let other = conversation(&mut app);
    seed_attachments(&app);
    for (conversation, message) in [
        (&chat, "m1"),
        (&chat, "m2"),
        (&chat, "m3"),
        (&chat, "m4"),
        (&other, "m5"),
    ] {
        post(
            &mut app,
            "/api/send",
            json!({"conversation_id": conversation, "message_id": message, "body": format!("Question {message}")}),
        )
        .unwrap();
    }
    let run_of = |app: &mut App, message: &str| -> String {
        let state = app.api("GET", "/api/state", b"").unwrap();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["messages"].as_array().unwrap())
            .find(|m| m["id"] == format!("reply-{message}"))
            .unwrap()["payload"]["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (run1, run2) = (run_of(&mut app, "m1"), run_of(&mut app, "m2"));
    {
        let tx = super::features::write_transaction(&app.connection).unwrap();
        grokbot_attachments::staging::link_attachments(&tx, &chat, "m1", &["image-a".into()])
            .unwrap();
        tx.execute("INSERT OR REPLACE INTO wrapper_group_inputs(account_id,conversation_id,message_id,mentions,targets,group_mode) VALUES (?1,?2,'m2','[\"grokbot\"]','[]',0)", rusqlite::params![super::ACCOUNT, chat]).unwrap();
        tx.execute("INSERT INTO wrapper_voice_memos(id,conversation_id,message_id,transcript,language,duration_ms,audio,created_at) VALUES ('memo-1',?1,'m2','Bonjour','fr',1200,x'0102',1)", [&chat]).unwrap();
        tx.execute("INSERT INTO wrapper_demonstrations(id,bot_id,conversation_id,identity,status,started_at,message_id) VALUES ('demo-1','grokbot',?1,'moi','done',1,'m2')", [&chat]).unwrap();
        tx.execute("INSERT INTO wrapper_run_sources(run_id,n,key,kind,title) VALUES (?1,1,'k1','web','Une source')", [&run1]).unwrap();
        tx.execute("INSERT INTO engine_response_usage(run_id,response,input_tokens,output_tokens,cached_input_tokens) VALUES (?1,1,10,5,2),(?1,2,7,3,0)", [&run2]).unwrap();
        tx.execute(
            "UPDATE wrapper_engine_runs SET notice='À noter' WHERE run_id=?1",
            [&run2],
        )
        .unwrap();
        // A meeting summary is not shown as a message of its own.
        tx.execute("INSERT INTO wrapper_meetings(id,conversation_id,title,status,created_at,message_id) VALUES ('meeting-1',?1,'Réunion','ready',1,'m4')", [&chat]).unwrap();
        tx.commit().unwrap();
    }
    let state = app.api("GET", "/api/state", b"").unwrap();
    for id in [&chat, &other] {
        let alone = serde_json::to_value(app.conversation(id).unwrap().unwrap()).unwrap();
        let listed = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == *id)
            .unwrap();
        assert_eq!(listed, &alone);
    }
    // The rows above do show: the comparison is not between two empty projections.
    let messages = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == chat)
        .unwrap()["messages"]
        .as_array()
        .unwrap();
    let find = |id: &str| messages.iter().find(|m| m["id"] == id);
    assert_eq!(find("m1").unwrap()["attachments"][0]["id"], "image-a");
    assert_eq!(
        find("m2").unwrap()["payload"]["mentions"],
        json!(["grokbot"])
    );
    assert_eq!(find("m2").unwrap()["payload"]["demonstration_id"], "demo-1");
    assert_eq!(find("m2").unwrap()["attachments"][0]["id"], "memo-1");
    assert_eq!(
        find("reply-m1").unwrap()["payload"]["sources"][0]["titre"],
        "Une source"
    );
    assert_eq!(
        find("reply-m2").unwrap()["payload"]["usage"],
        json!({"input_tokens": 17, "output_tokens": 8, "cached_input_tokens": 2})
    );
    assert_eq!(find("reply-m2").unwrap()["notice"], "À noter");
    assert!(find("m4").is_none() && find("reply-m4").is_none());
    assert!(find("m3").unwrap()["attachments"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(find("m3").unwrap()["payload"]["mentions"], json!([]));
    assert!(find("reply-m3").unwrap()["payload"]["usage"].is_null());
}
