use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use grokbot_domain::{ConversationId, MessageId};
use grokbot_session::SubmissionTransport;
use serde_json::json;

use super::{validate_headers, App};

static NEXT: AtomicUsize = AtomicUsize::new(0);

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
        "system"
    );
    app.api("POST", "/api/preferences", br#"{"theme":"dark"}"#)
        .unwrap();
    drop(app);
    let mut app = App::open(&db.0).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["theme"],
        "dark"
    );
    assert_eq!(
        app.api("POST", "/api/preferences", br#"{"theme":"invalid"}"#)
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["theme"],
        "dark"
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
    let mut transport = super::LocalTransport {
        connection: &app.connection,
        lose_reply: false,
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
        let _ = std::fs::remove_file(&self.0);
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
