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
    let mut transport = super::LocalTransport {
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
            bot["is_default"].clone(),
            bot["template"].clone(),
            bot["customized"].clone(),
        ),
        (
            json!("grokbot"),
            json!("Grokbot"),
            json!("citron"),
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
    assert_eq!(bot["name"], "Grokbot");
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
            "id": "bot-1", "name": "Juriste", "color": "violet",
            "description": "Relit vos contrats.", "instructions": "Cite toujours l'article.",
            "is_default": false, "template": null, "customized": false, "deleted": false,
        })
    );
    assert_eq!(bot_ids(&mut app), ["grokbot", "bot-1"]);
    let mut renamed = request;
    renamed["name"] = json!("Notaire");
    assert_eq!(post(&mut app, "/api/bots", renamed), Err(409));
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
fn composed_instructions_put_identity_role_and_private_rules_after_the_app_contract() {
    let bot = super::Bot {
        id: "bot-1".into(),
        name: "Juriste".into(),
        color: "violet".into(),
        description: "Relit vos contrats.".into(),
        instructions: "Cite toujours l'article.".into(),
        is_default: false,
        template: None,
        customized: false,
        deleted: false,
    };
    assert_eq!(
        super::compose_instructions(&bot),
        "Tu es un assistant dans une application de bureau. Réponds dans la langue de l'utilisateur. Tu peux utiliser le Markdown, des blocs de code avec le nom du langage, des diagrammes dans des blocs ```mermaid et des formules LaTeX entre $...$ ou $$...$$.\n\nTu t'appelles Juriste.\n\nTon rôle, tel que les autres le voient : Relit vos contrats.\n\nConsignes privées sur ta façon de te comporter :\nCite toujours l'article."
    );
    let bare = super::Bot {
        description: String::new(),
        instructions: String::new(),
        ..bot
    };
    assert!(super::compose_instructions(&bare).ends_with("$$...$$.\n\nTu t'appelles Juriste."));
}
