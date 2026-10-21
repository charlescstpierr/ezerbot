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

const KEY: &str = "ezerbot-image-key-9f3c1a7e-loop";
const INDICATOR: &str = "Génération d’une image…";
const CONNECT: &str = "Connecte ton compte OpenAI";
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    root: PathBuf,
    chat: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

impl Fixture {
    fn open(with_key: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-image-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("data/workspace.sqlite");
        std::fs::write(
            path.with_extension("chatgpt-auth.json"),
            json!({
                "host_id": "test-host",
                "client_id": CLIENT,
                "access_token": "fake-access",
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
        if with_key {
            Vault::open(&path.with_extension("secrets.json"))
                .unwrap()
                .set(
                    "OPENAI_API_KEY",
                    "Compte OpenAI pour générer des images",
                    KEY,
                )
                .unwrap();
        }
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

fn png() -> Vec<u8> {
    STANDARD.decode(PNG_B64).unwrap()
}

fn assert_key_hidden(fixture: &mut Fixture) {
    let state = fixture
        .app
        .api("GET", "/api/state", b"")
        .unwrap()
        .to_string();
    assert!(!state.contains(KEY), "clé présente dans l’état exporté");
    let bodies = fixture.server.seen.lock().unwrap().bodies.clone();
    for body in bodies {
        assert!(
            !body.to_string().contains(KEY),
            "clé présente dans le contexte du modèle"
        );
    }
    let calls = fixture.server.seen.lock().unwrap().image_calls.clone();
    for call in calls {
        assert!(
            !call.to_string().contains(KEY),
            "clé présente dans le corps d’image"
        );
    }
    scan_tree(&fixture.root);
    let log = fixture.root.join("data/logs/engine.log");
    if log.is_file() {
        let text = std::fs::read(&log).unwrap();
        assert!(
            !String::from_utf8_lossy(&text).contains(KEY),
            "clé présente dans le journal"
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
            !String::from_utf8_lossy(&bytes).contains(KEY),
            "clé présente hors du trousseau"
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
fn drawing_replaces_the_indicator_with_one_image() {
    let mut fixture = Fixture::open(true);
    let gate = fixture.hold_image();
    fixture.script_image(200, json!({"data":[{"b64_json":PNG_B64}]}));
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
    let call = fixture.server.seen.lock().unwrap().image_calls[0].clone();
    assert_eq!(call["model"], "gpt-image");
    assert_eq!(call["prompt"], "un phare au crépuscule");
    assert_eq!(call["aspect_ratio"], "1:1");
    assert_eq!(call["n"], 1);
    let authorization = fixture.server.seen.lock().unwrap().image_authorization[0].clone();
    assert!(
        authorization == format!("Bearer {KEY}"),
        "en-tête d’autorisation inattendu"
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
    assert_key_hidden(&mut fixture);
}

#[test]
fn safety_refusal_is_one_french_sentence() {
    let mut fixture = Fixture::open(true);
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
fn restricted_model_is_a_different_french_sentence() {
    let mut fixture = Fixture::open(true);
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
fn missing_key_shows_the_connect_card_without_a_call() {
    let mut fixture = Fixture::open(false);
    fixture.ask(
        "sans-cle",
        json!({"description":"un phare au crépuscule"}),
        "Dessine un phare au crépuscule",
    );
    fixture.until("carte de connexion absente", |fixture| {
        fixture.tool()["body"] == CONNECT
    });
    assert_eq!(fixture.image_calls(), 0);
    assert!(fixture.tool()["attachments"].as_array().unwrap().is_empty());
    assert!(fixture
        .messages()
        .iter()
        .all(|message| message["kind"] != "approval"));
    assert_eq!(
        fixture
            .db()
            .query_row("SELECT COUNT(*) FROM forms_requests", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fixture.image_calls(), 0);
}

#[test]
fn describing_an_attached_image_does_not_generate_one() {
    let mut fixture = Fixture::open(true);
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
