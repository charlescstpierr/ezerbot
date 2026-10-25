//! Visible image generation against the loop provider. The scripted chat asks
//! for `generate_image`; the image host is a separate call on the same loop.
mod support;

use base64::{engine::general_purpose::STANDARD, Engine};
use grokbot_forms::vault::Vault;
use grokbot_wrapper::{features::attachments, ApiResponse, App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const SECRET: &str = "ezerbot-bot-secret-9f3c1a7e-loop";
const ACCOUNT: &str = "account-ezerbot-image";
const INDICATOR: &str = "Génération d’une image…";
const RESTRICTED: &str = "Ce modèle d’image est restreint pour ce compte.";
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    root: PathBuf,
    chat: String,
    token: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl Fixture {
    fn open() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-image-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("data/workspace.sqlite");
        let token = session_token(ACCOUNT);
        std::fs::write(
            path.with_extension("chatgpt-auth.json"),
            json!({
                "host_id": "test-host",
                "client_id": CLIENT,
                "account_id": ACCOUNT,
                "access_token": token,
                "refresh_token": "fake-refresh",
                "expires_at": now() / 1000 + 3600,
                "scopes": "chatgpt.tokens.use.direct"
            })
            .to_string(),
        )
        .unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        let vault = Vault::open(&path.with_extension("secrets.json")).unwrap();
        vault.set("BOT_PHARE", "Secret d’un bot", SECRET).unwrap();
        vault
            .set(
                "OPENAI_API_KEY",
                "Secret nommé comme une ancienne clé",
                "ezerbot-named-secret-9f3c1a7e-loop",
            )
            .unwrap();
        let chat = app
            .api(
                "POST",
                "/api/conversations",
                &serde_json::to_vec(&json!({"title":"Images générées"})).unwrap(),
            )
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            app,
            provider,
            server,
            path,
            root,
            chat,
            token,
        }
    }

    fn db(&self) -> Connection {
        let connection = Connection::open(&self.path).unwrap();
        connection.busy_timeout(Duration::from_secs(3)).unwrap();
        connection
    }

    fn messages(&mut self) -> Vec<Value> {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["id"] == self.chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn hold_image(&self) -> Arc<AtomicBool> {
        let gate = Arc::new(AtomicBool::new(false));
        self.server.seen.lock().unwrap().image_gate = Some(gate.clone());
        gate
    }

    fn script_image(&self, status: u16, body: Value) {
        self.server
            .seen
            .lock()
            .unwrap()
            .image_reply
            .push_back(ScriptedResponse::json(status, body));
    }

    fn ask(&mut self, id: &str, arguments: Value, body: &str) {
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":"generate_image","arguments":arguments.to_string()}}),
            json!({"type":"response.completed"}),
        ]));
        self.app
            .api(
                "POST",
                "/api/send",
                serde_json::to_vec(
                    &json!({"conversation_id":self.chat,"message_id":id,"body":body}),
                )
                .unwrap()
                .as_slice(),
            )
            .unwrap();
    }

    fn image_calls(&self) -> usize {
        self.server.seen.lock().unwrap().image_calls.len()
    }

    fn until(&mut self, label: &'static str, mut ready: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            self.app.tick(now());
            if ready(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{label}");
    }

    fn tool(&mut self) -> Value {
        self.messages()
            .into_iter()
            .find(|message| message["payload"]["call"]["name"] == "generate_image")
            .unwrap_or(Value::Null)
    }
}

fn session_token(account: &str) -> String {
    format!(
        "e30.{}.e30",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":account,"chatgpt_data_residency":"eu"}}).to_string()
        )
    )
}

fn assert_image_contract(fixture: &Fixture, prompt: &str, size: &str) {
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(seen.image_calls.len(), 1);
    assert_eq!(seen.image_urls[0], "/backend-api/codex/responses");
    let call = &seen.image_calls[0];
    assert_eq!(call["model"], "gpt-6.1-sol");
    assert_eq!(call["input"], json!([{"role": "user", "content": prompt}]));
    assert_eq!(call["stream"], true);
    assert!(call["instructions"]
        .as_str()
        .is_some_and(|text| !text.trim().is_empty()));
    assert!(call.get("aspect_ratio").is_none());
    assert!(call.get("prompt").is_none());
    assert!(call.get("n").is_none());
    let tools = call["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["type"], "image_generation");
    assert_eq!(tools[0]["model"], "gpt-image-1.5");
    assert_eq!(tools[0]["size"], size);
    assert!(tools[0].get("aspect_ratio").is_none());
    assert_eq!(call["tool_choice"]["type"], "image_generation");
    let headers = &seen.image_headers[0];
    let authorization = format!("Bearer {}", fixture.token);
    assert_eq!(
        headers.get("authorization").map(String::as_str),
        Some(authorization.as_str())
    );
    assert_eq!(
        headers.get("chatgpt-account-id").map(String::as_str),
        Some(ACCOUNT)
    );
    assert_eq!(
        headers
            .get("x-openai-internal-codex-residency")
            .map(String::as_str),
        Some("eu")
    );
    assert_eq!(
        headers.get("originator").map(String::as_str),
        Some("ezerbot")
    );
    assert!(!call.to_string().contains("api.openai.com"));
    assert!(!call.to_string().contains("images/generations"));
}

fn png() -> Vec<u8> {
    STANDARD.decode(PNG_B64).unwrap()
}

fn assert_key_hidden(fixture: &mut Fixture) {
    let state = fixture
        .app
        .api("GET", "/api/state", b"")
        .unwrap()
        .to_string();
    assert!(
        !state.contains(SECRET),
        "secret présent dans l’état exporté"
    );
    let bodies = fixture.server.seen.lock().unwrap().bodies.clone();
    for body in bodies {
        assert!(
            !body.to_string().contains(SECRET),
            "secret présent dans le contexte du modèle"
        );
    }
    let calls = fixture.server.seen.lock().unwrap().image_calls.clone();
    for call in calls {
        assert!(
            !call.to_string().contains(SECRET),
            "secret présent dans le corps d’image"
        );
    }
    scan_tree(&fixture.root);
    let log = fixture.root.join("data/logs/engine.log");
    if log.is_file() {
        let text = std::fs::read(&log).unwrap();
        assert!(
            !String::from_utf8_lossy(&text).contains(SECRET),
            "secret présent dans le journal"
        );
    }
}

fn scan_tree(root: &Path) {
    for entry in walkdir_files(root) {
        if entry
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".secrets.json"))
        {
            continue;
        }
        let bytes = std::fs::read(&entry).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&bytes).contains(SECRET),
            "secret présent hors du coffre"
        );
    }
}

fn walkdir_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

#[test]
fn cancelled_generation_does_not_store_or_link_the_late_image() {
    let mut fixture = Fixture::open();
    let gate = fixture.hold_image();
    fixture.script_image(
        200,
        json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
    );
    fixture.ask(
        "annuler",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("appel image absent", |fixture| fixture.image_calls() == 1);
    fixture
        .app
        .api(
            "POST",
            "/api/stop",
            &serde_json::to_vec(&json!({"conversation_id":fixture.chat,"message_id":"annuler"}))
                .unwrap(),
        )
        .unwrap();
    gate.store(true, Ordering::SeqCst);
    wait("outil terminé", || {
        fixture
            .db()
            .query_row(
                "SELECT COUNT(*) FROM engine_external_tools WHERE outcome IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 0
    });
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT COUNT(*) FROM attachments_staged", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .db()
            .query_row(
                "SELECT COUNT(*) FROM attachments_message_links",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn a_drawing_that_lands_after_its_thread_is_deleted_is_not_stored() {
    let mut fixture = Fixture::open();
    let gate = fixture.hold_image();
    fixture.script_image(
        200,
        json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
    );
    fixture.ask(
        "efface",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("appel image absent", |fixture| fixture.image_calls() == 1);
    let chat = fixture.chat.clone();
    fixture
        .app
        .api(
            "POST",
            "/api/conversations/delete",
            &serde_json::to_vec(&json!({"id": chat})).unwrap(),
        )
        .unwrap();
    gate.store(true, Ordering::SeqCst);
    wait("outil terminé", || {
        fixture
            .db()
            .query_row(
                "SELECT COUNT(*) FROM engine_external_tools WHERE outcome IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 0
    });
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT COUNT(*) FROM attachments_staged", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn deleting_a_thread_erases_the_drawing_it_holds() {
    let mut fixture = Fixture::open();
    fixture.script_image(
        200,
        json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
    );
    fixture.ask(
        "garde",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("réponse non terminée", |fixture| {
        fixture.provider.status()["titling"] == false
            && fixture
                .db()
                .query_row(
                    "SELECT COUNT(*) FROM engine_runs WHERE status NOT IN ('finished','failed','cancelled')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                == 0
    });
    let url = fixture.tool()["attachments"][0]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(fixture.app.api_response("GET", &url, b"").is_ok());
    let chat = fixture.chat.clone();
    fixture
        .app
        .api(
            "POST",
            "/api/conversations/delete",
            &serde_json::to_vec(&json!({"id": chat})).unwrap(),
        )
        .unwrap();
    assert_eq!(
        fixture
            .app
            .api_response("GET", &url, b"")
            .unwrap_err()
            .status,
        404
    );
}

#[test]
fn drawing_replaces_the_indicator_with_one_image() {
    let mut fixture = Fixture::open();
    let gate = fixture.hold_image();
    fixture.script_image(
        200,
        json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
    );
    fixture.ask(
        "phare",
        json!({"description":"un phare au crépuscule"}),
        "Dessine un phare au crépuscule",
    );
    fixture.until("indicateur absent ou appel image manquant", |fixture| {
        fixture.tool()["body"] == INDICATOR && fixture.image_calls() == 1
    });
    let pending = fixture.tool();
    assert!(pending["attachments"].as_array().unwrap().is_empty());
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT COUNT(*) FROM forms_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(fixture
        .messages()
        .iter()
        .all(|message| message["kind"] != "approval"));
    gate.store(true, Ordering::SeqCst);
    fixture.until("l’image ne remplace pas l’indicateur", |fixture| {
        let tool = fixture.tool();
        !tool["body"].as_str().unwrap_or("").contains("Génération")
            && tool["attachments"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["mime_type"] == "image/png"))
    });
    assert_eq!(fixture.image_calls(), 1);
    let tool = fixture.tool();
    let url = tool["attachments"][0]["url"].as_str().unwrap();
    match fixture.app.api_response("GET", url, b"").unwrap() {
        ApiResponse::Bytes { body, content_type } => {
            assert_eq!(body, png());
            assert_eq!(content_type, "image/png");
        }
        ApiResponse::Json(_) => panic!("image non binaire"),
    }
    assert_eq!(fixture.image_calls(), 1);
    assert_image_contract(&fixture, "un phare au crépuscule", "1024x1024");
    fixture.until("réponse non terminée", |fixture| {
        fixture.provider.status()["titling"] == false
            && fixture
                .db()
                .query_row(
                    "SELECT COUNT(*) FROM engine_runs WHERE status NOT IN ('finished','failed','cancelled')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                == 0
    });
    assert_key_hidden(&mut fixture);
}

#[test]
fn safety_refusal_is_one_french_sentence() {
    let mut fixture = Fixture::open();
    fixture.script_image(
        400,
        json!({"error":{"code":"content_safety_blocked","message":"blocked"}}),
    );
    fixture.ask(
        "refus",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("refus de sécurité absent", |fixture| {
        fixture.tool()["body"]
            .as_str()
            .unwrap_or("")
            .contains("sécurité")
            && fixture.image_calls() == 1
    });
    let tool = fixture.tool();
    assert!(tool["attachments"].as_array().unwrap().is_empty());
    assert!(!tool["body"].as_str().unwrap_or("").contains("blocked"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fixture.image_calls(), 1);
    assert_key_hidden(&mut fixture);
}

#[test]
fn streamed_safety_refusal_with_http_success_is_shown() {
    let mut fixture = Fixture::open();
    fixture.server.seen.lock().unwrap().image_reply.push_back(ScriptedResponse::events(&[
        json!({"type":"response.failed","response":{"error":{"code":"content_safety_blocked","message":"blocked"}}}),
    ]));
    fixture.ask(
        "refus-stream",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("refus de sécurité streamé absent", |fixture| {
        fixture.tool()["body"]
            == "La génération de cette image a été refusée pour des raisons de sécurité."
    });
    assert!(fixture.tool()["attachments"].as_array().unwrap().is_empty());
    assert_eq!(fixture.image_calls(), 1);
}

#[test]
fn repeated_streamed_image_preserves_the_decoded_size_limit() {
    for (size, accepted) in [(20 * 1024 * 1024, true), (20 * 1024 * 1024 + 1, false)] {
        let mut fixture = Fixture::open();
        let mut bytes = png();
        bytes.resize(size, 0);
        let item = json!({"type":"image_generation_call","status":"completed","result":STANDARD.encode(&bytes)});
        fixture
            .server
            .seen
            .lock()
            .unwrap()
            .image_reply
            .push_back(ScriptedResponse::events(&[
                json!({"type":"response.output_item.done","item":item}),
                json!({"type":"response.completed","response":{"output":[item]}}),
            ]));
        fixture.ask(
            "large-stream",
            json!({"description":"un phare"}),
            "Dessine un phare",
        );
        fixture.until("génération streamée non terminée", |fixture| {
            let tool = fixture.tool();
            !tool.is_null() && tool["body"] != INDICATOR
        });
        let tool = fixture.tool();
        if accepted {
            let attachments = tool["attachments"].as_array().unwrap();
            assert_eq!(attachments.len(), 1, "{tool}");
            match fixture
                .app
                .api_response("GET", attachments[0]["url"].as_str().unwrap(), b"")
                .unwrap()
            {
                ApiResponse::Bytes { body, .. } => assert_eq!(body, bytes),
                ApiResponse::Json(_) => panic!("image non binaire"),
            }
        } else {
            assert_eq!(
                tool["body"],
                "L’image générée dépasse 20 Mo et n’a pas été conservée."
            );
            assert!(tool["attachments"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn restricted_model_is_a_different_french_sentence() {
    let mut fixture = Fixture::open();
    fixture.script_image(
        400,
        json!({"error":{"code":"model_restricted","message":"restricted"}}),
    );
    fixture.ask(
        "restreint",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("modèle restreint absent", |fixture| {
        let tool = fixture.tool();
        let body = tool["body"].as_str().unwrap_or("");
        body.contains("restreint") && !body.contains("sécurité") && fixture.image_calls() == 1
    });
    assert!(fixture.tool()["attachments"].as_array().unwrap().is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fixture.image_calls(), 1);
}

#[test]
fn wide_frame_is_sent_as_a_supported_size() {
    let mut fixture = Fixture::open();
    fixture.ask(
        "large",
        json!({"description":"un phare large","aspect_ratio":"16:9"}),
        "Dessine un phare en large",
    );
    fixture.until("appel image large absent", |fixture| {
        fixture.image_calls() == 1
    });
    assert_image_contract(&fixture, "un phare large", "1536x1024");
    fixture.until("réponse large non terminée", |fixture| {
        fixture.provider.status()["titling"] == false
            && fixture
                .db()
                .query_row(
                    "SELECT COUNT(*) FROM engine_runs WHERE status NOT IN ('finished','failed','cancelled')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap()
                == 0
    });
}

#[test]
fn host_refusal_is_one_restricted_sentence() {
    let mut fixture = Fixture::open();
    fixture.script_image(
        403,
        json!({"error":{"code":"insufficient_quota","message":"nope"}}),
    );
    fixture.ask(
        "refus-hote",
        json!({"description":"un phare"}),
        "Dessine un phare",
    );
    fixture.until("refus d’hôte absent", |fixture| {
        fixture.tool()["body"] == RESTRICTED && fixture.image_calls() == 1
    });
    assert!(fixture.tool()["attachments"].as_array().unwrap().is_empty());
    assert!(!fixture.tool()["body"]
        .as_str()
        .unwrap_or("")
        .contains("nope"));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fixture.image_calls(), 1);
}

#[test]
fn signout_removes_only_the_session() {
    let mut fixture = Fixture::open();
    let path = fixture.path.with_extension("secrets.json");
    let before = std::fs::read(&path).unwrap();
    fixture
        .app
        .api("POST", "/api/provider/signout", b"{}")
        .unwrap();
    assert!(!fixture.provider.connected());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let vault = Vault::open(&path).unwrap();
    assert_eq!(vault.get("BOT_PHARE").unwrap().as_deref(), Some(SECRET));
    assert_eq!(
        vault.get("OPENAI_API_KEY").unwrap().as_deref(),
        Some("ezerbot-named-secret-9f3c1a7e-loop")
    );
}

#[cfg(unix)]
#[test]
fn signout_leaves_a_symlinked_vault_untouched() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::open();
    let path = fixture.path.with_extension("secrets.json");
    let stash = fixture.root.join("data/vault-stash.json");
    let before = std::fs::read(&path).unwrap();
    assert!(String::from_utf8_lossy(&before).contains(SECRET));
    std::fs::rename(&path, &stash).unwrap();
    symlink(&stash, &path).unwrap();
    fixture
        .app
        .api("POST", "/api/provider/signout", b"{}")
        .unwrap();
    assert!(!fixture.provider.connected());
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink(),
        "le coffre a été réécrit"
    );
    assert_eq!(std::fs::read(&stash).unwrap(), before);
}

/// Attaches the sample image to the thread by sending a described message,
/// the way Charles joins a photo. Returns the attachment id.
fn attach_described_image(fixture: &mut Fixture) -> String {
    let bytes = png();
    let image = fixture.app.api("POST", "/api/attachments", &bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture
        .server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"Un carré bleu."}),
            json!({"type":"response.completed"}),
        ]));
    fixture
        .app
        .api(
            "POST",
            "/api/attachments/precaption",
            serde_json::to_vec(&json!({
                "conversation_id": fixture.chat,
                "attachment_id": image,
                "draft": "Décris cette image"
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    wait("légende prête", || {
        let value = fixture
            .app
            .api(
                "POST",
                "/api/attachments/precaption",
                serde_json::to_vec(&json!({
                    "conversation_id": fixture.chat,
                    "attachment_id": image,
                    "draft": "Décris cette image"
                }))
                .unwrap()
                .as_slice(),
            )
            .unwrap();
        value["caption"]["status"] == "ready"
    });
    fixture
        .server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"C’est un carré bleu."}),
            json!({"type":"response.completed"}),
        ]));
    fixture
        .app
        .api(
            "POST",
            "/api/send",
            serde_json::to_vec(&json!({
                "conversation_id": fixture.chat,
                "message_id": "joindre",
                "body": "Décris cette image",
                "attachments": [image]
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    fixture.until("description de l’image jointe absente", |fixture| {
        fixture.messages().iter().any(|message| {
            message["id"] == "reply-joindre"
                && message["body"]
                    .as_str()
                    .unwrap_or("")
                    .contains("carré bleu")
        })
    });
    image
}

#[test]
fn describing_an_attached_image_does_not_generate_one() {
    let mut fixture = Fixture::open();
    attach_described_image(&mut fixture);
    let user = fixture
        .messages()
        .into_iter()
        .find(|message| message["id"] == "joindre")
        .unwrap();
    assert!(user["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["mime_type"] == "image/png"));
    assert_eq!(fixture.image_calls(), 0);
    wait("travaux d’image terminés", || {
        fixture.provider.status()["titling"] == false
            && attachments::state(&fixture.app.feature_context())["network_workers"] == 0
    });
    assert_eq!(fixture.image_calls(), 0);
}

const FRAME_REFUSED: &str =
    "Ce cadre n’est pas pris en charge. Choisissez 1:1, 16:9, 9:16, 4:3 ou 3:4.";
const BMP_REFUSED: &str =
    "Une image BMP ne peut pas servir de référence. Utilisez un PNG, un JPEG, un GIF ou un WebP.";
const REFERENCE_REFUSED: &str = "Une image de référence doit être jointe à cette conversation.";

impl Fixture {
    fn settle(&mut self) {
        self.until("réponse non terminée", |fixture| {
            fixture.provider.status()["titling"] == false
                && fixture
                    .db()
                    .query_row(
                        "SELECT COUNT(*) FROM engine_runs WHERE status NOT IN ('finished','failed','cancelled')",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap()
                    == 0
        });
    }
}

#[test]
fn an_attached_photo_is_sent_as_the_reference_in_the_chosen_frame() {
    let mut fixture = Fixture::open();
    let photo = attach_described_image(&mut fixture);
    fixture.settle();
    fixture.script_image(
        200,
        json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
    );
    fixture.ask(
        "style",
        json!({
            "description": "le même style, en large",
            "aspect_ratio": "16:9",
            "reference_image_paths": [photo]
        }),
        "Le même style, en 16:9",
    );
    fixture.until("appel image avec référence absent", |fixture| {
        fixture.image_calls() == 1
    });
    {
        let seen = fixture.server.seen.lock().unwrap();
        let content = seen.image_calls[0]["input"][0]["content"]
            .as_array()
            .expect("la demande doit porter des parties de contenu");
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "input_text");
        assert_eq!(content[0]["text"], "le même style, en large");
        assert_eq!(content[1]["type"], "input_image");
        assert_eq!(
            content[1]["image_url"],
            format!("data:image/png;base64,{PNG_B64}")
        );
        assert_eq!(seen.image_calls[0]["tools"][0]["size"], "1536x1024");
        assert!(!seen.image_calls[0].to_string().contains(&photo));
    }
    fixture.until("l’image large n’apparaît pas", |fixture| {
        fixture.tool()["attachments"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["mime_type"] == "image/png"))
    });
    assert_eq!(
        fixture.tool()["payload"]["outcome"]["output"]["aspect_ratio"],
        "16:9"
    );
    assert_eq!(fixture.image_calls(), 1);
    fixture.settle();
    assert_key_hidden(&mut fixture);
}

#[test]
fn every_listed_frame_is_accepted_and_sent_as_its_size() {
    for (frame, size) in [
        ("1:1", "1024x1024"),
        ("16:9", "1536x1024"),
        ("9:16", "1024x1536"),
        ("4:3", "1536x1024"),
        ("3:4", "1024x1536"),
    ] {
        let mut fixture = Fixture::open();
        fixture.ask(
            "cadre",
            json!({"description":"un phare","aspect_ratio":frame}),
            "Dessine un phare",
        );
        fixture.until("appel image absent", |fixture| fixture.image_calls() == 1);
        assert_eq!(
            fixture.server.seen.lock().unwrap().image_calls[0]["tools"][0]["size"],
            size,
            "{frame}"
        );
        fixture.settle();
    }
}

#[test]
fn an_unknown_frame_is_a_french_sentence_and_no_call() {
    let mut fixture = Fixture::open();
    fixture.ask(
        "deux-un",
        json!({"description":"un phare","aspect_ratio":"2:1"}),
        "Dessine un phare en 2:1",
    );
    fixture.until("phrase de refus absente", |fixture| {
        fixture.tool()["body"] == FRAME_REFUSED
    });
    assert!(fixture.tool()["attachments"].as_array().unwrap().is_empty());
    fixture.settle();
    assert_eq!(fixture.image_calls(), 0);
}

#[test]
fn a_reference_that_is_not_in_the_thread_is_refused_without_a_call() {
    let mut fixture = Fixture::open();
    let elsewhere = fixture.app.api("POST", "/api/attachments", &png()).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (id, reference) in [
        ("inconnue", json!("image-qui-nexiste-pas")),
        ("non-jointe", json!(elsewhere)),
    ] {
        fixture.ask(
            id,
            json!({"description":"un phare","reference_image_paths":[reference]}),
            "Dessine un phare dans ce style",
        );
        fixture.until("phrase de refus absente", |fixture| {
            fixture
                .messages()
                .iter()
                .any(|message| message["body"] == REFERENCE_REFUSED)
        });
        fixture.settle();
    }
    assert_eq!(fixture.image_calls(), 0);
}

#[test]
fn a_bmp_reference_is_refused_without_a_call() {
    let mut fixture = Fixture::open();
    let mut bmp = vec![0u8; 30];
    bmp[..2].copy_from_slice(b"BM");
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    let id = fixture.app.api("POST", "/api/attachments", &bmp).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    fixture
        .server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"Image reçue."}),
            json!({"type":"response.completed"}),
        ]));
    fixture
        .app
        .api(
            "POST",
            "/api/send",
            serde_json::to_vec(&json!({
                "conversation_id": fixture.chat,
                "message_id": "bitmap",
                "body": "Voici un BMP",
                "attachments": [id]
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    fixture.until("réponse au BMP absente", |fixture| {
        fixture
            .messages()
            .iter()
            .any(|message| message["id"] == "reply-bitmap")
    });
    fixture.settle();
    fixture.ask(
        "style-bmp",
        json!({"description":"un phare","reference_image_paths":[id]}),
        "Dessine un phare dans ce style",
    );
    fixture.until("phrase de refus absente", |fixture| {
        fixture
            .messages()
            .iter()
            .any(|message| message["body"] == BMP_REFUSED)
    });
    fixture.settle();
    assert_eq!(fixture.image_calls(), 0);
}
