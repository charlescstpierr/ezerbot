//! Codex OAuth + Responses + durable app path. Only ephemeral local servers/accounts.
mod support;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use grokbot_engine::contracts::{
    HistoryItem, ImageInput, ModelEvent, ModelGateway, ModelRequest, Role, ToolSpec,
};
use grokbot_wrapper::{App, Endpoints, Provider};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use support::{form, wait, FakeOpenAi, ScriptedResponse, Seen};
const CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
static OAUTH: Mutex<()> = Mutex::new(());

struct Fixture {
    db: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-codex-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("app.sqlite");
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Self::provider(&db, &server);
        Self {
            db,
            server,
            provider,
        }
    }
    fn provider(db: &std::path::Path, server: &FakeOpenAi) -> Provider {
        Provider::with_backends(
            db,
            server.endpoints(),
            Endpoints {
                api: format!("{}/backend-api/codex", server.base),
                ..server.endpoints()
            },
            false,
        )
    }
    fn begin(&self) -> HashMap<String, String> {
        let url = self.provider.start_signin().unwrap();
        assert!(url.starts_with(&self.server.base));
        form(url.split_once('?').unwrap().1)
    }
    fn callback(query: &HashMap<String, String>, result: &str) -> String {
        ureq::get(&format!(
            "{}?state={}&{result}",
            query["redirect_uri"], query["state"]
        ))
        .call()
        .unwrap()
        .into_string()
        .unwrap()
    }
    fn login(&self) {
        let query = self.begin();
        assert!(Self::callback(&query, "code=fake-code").contains("Connexion réussie"));
        wait("connexion Codex et modèles", || {
            self.provider.status()["models_ready"] == true
        });
    }
    fn raw(&self) -> Result<ureq::Response, String> {
        self.provider
            .responses(
                json!({"model":"gpt-5.5", "instructions":"Consignes", "input":[], "stream":true}),
            )
            .map_err(|f| f.message)
    }
    fn reply(&self, reply: ScriptedResponse) {
        self.server.seen.lock().unwrap().responses.push_back(reply);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.provider.cancel_signin();
        let _ = std::fs::remove_dir_all(self.db.parent().unwrap());
    }
}
fn post(app: &mut App, path: &str, value: Value) -> Value {
    app.api("POST", path, value.to_string().as_bytes()).unwrap()
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "gpt-5.5".into(),
        instructions: "Consignes locales".into(),
        input: vec![HistoryItem {
            entry_id: "user".into(),
            role: Role::User,
            text: "Regarde".into(),
            tool_items: vec![],
            images: vec![ImageInput {
                attachment_id: "image-test".into(),
                url: Some("data:image/png;base64,AA==".into()),
                caption: None,
            }],
        }],
        tools: vec![ToolSpec {
            name: "remember".into(),
            description: "Mémoire".into(),
            schema: json!({"type":"object"}),
            requires_approval: false,
        }],
        effort: Some("high".into()),
        service_tier: None,
        prompt_cache_key: Some("ezerbot-test".into()),
    }
}

#[test]
fn oauth_matches_pi_pkce_refresh_storage_selection_and_local_signout() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    assert_eq!(f.provider.status()["backend"], "codex");
    let query = f.begin();
    assert_eq!(query["client_id"], CLIENT);
    assert_eq!(query["scope"], "openid profile email offline_access");
    assert_eq!(query["redirect_uri"], "http://localhost:1455/auth/callback");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["originator"], "pi");
    assert_eq!(query["id_token_add_organizations"], "true");
    assert_eq!(query["codex_cli_simplified_flow"], "true");
    assert!(!query.contains_key("resource") && !query.contains_key("ext_agent_host_id"));
    assert!(
        ureq::get(&format!("{}?code=wrong&state=wrong", query["redirect_uri"]))
            .call()
            .is_err()
    );
    assert_eq!(f.provider.status()["signin"], "pending");
    assert!(Fixture::callback(&query, "code=fake-code").contains("Connexion réussie"));
    wait("models", || f.provider.status()["models_ready"] == true);
    f.raw().unwrap(); // immediately expiring initial token -> serialized refresh
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.forms.len(), 2);
    assert_eq!(seen.forms[0]["client_id"], CLIENT);
    assert_eq!(seen.forms[0]["code"], "fake-code");
    assert!(!seen.forms[0].contains_key("resource"));
    use sha2::{Digest, Sha256};
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(seen.forms[0]["code_verifier"].as_bytes())),
        query["code_challenge"]
    );
    assert_eq!(seen.forms[1]["grant_type"], "refresh_token");
    assert_eq!(seen.forms[1]["refresh_token"], "refresh-1");
    assert!(!seen.forms[1].contains_key("resource"));
    assert_eq!(seen.model_requests, 0); // pi's static Codex catalogue, never /v1/models
    drop(seen);
    let status = f.provider.status();
    assert_eq!(status["email"], "codex@exemple.com");
    assert_eq!(status["plan"], "plus");
    assert_eq!(status["models"].as_array().unwrap().len(), 9);
    assert!(!status.to_string().contains("refresh-"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.db.with_extension("codex-auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    f.provider.select_backend("sharing").unwrap();
    let restored = Fixture::provider(&f.db, &f.server);
    assert_eq!(restored.status()["backend"], "sharing");
    assert_eq!(restored.status()["connections"]["codex"], true);
    restored.select_backend("codex").unwrap();
    assert_eq!(restored.status()["connected"], true);
    assert!(restored.signout().unwrap());
    assert!(!f.server.seen.lock().unwrap().revoked); // no undocumented revoke endpoint
    let restarted = Fixture::provider(&f.db, &f.server);
    assert_eq!(restarted.status()["connected"], false);
    let saved = std::fs::read_to_string(f.db.with_extension("codex-auth.json")).unwrap();
    assert!(!saved.contains("refresh-2"));
}

#[test]
fn denial_cancel_missing_account_and_late_token_never_connect() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    let query = f.begin();
    assert!(Fixture::callback(&query, "error=access_denied").contains("Vous avez refusé"));
    wait("refus", || f.provider.status()["signin"] == "failed");
    assert!(!f.provider.connected());
    assert!(f.server.seen.lock().unwrap().forms.is_empty());
    let query = f.begin();
    f.server.seen.lock().unwrap().invalid_codex_account = true;
    assert!(Fixture::callback(&query, "code=no-account").contains("identifiant du compte Codex"));
    wait("compte refusé", || {
        f.provider.status()["signin"] == "failed"
    });
    assert!(!f.provider.connected());
    f.server.seen.lock().unwrap().invalid_codex_account = false;
    let query = f.begin();
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().token_gate = Some(gate.clone());
    let pending = std::thread::spawn(move || Fixture::callback(&query, "code=late"));
    wait("échange suspendu", || {
        f.server.seen.lock().unwrap().forms.len() == 2
    });
    f.provider.cancel_signin();
    gate.store(true, Ordering::SeqCst);
    assert!(pending.join().unwrap().contains("annulée"));
    assert!(!f.provider.connected());
    assert_eq!(f.provider.status()["signin"], "idle");
    f.server.seen.lock().unwrap().token_gate = None;
    f.login(); // immediate retry after cancellation must free localhost:1455
}

#[test]
fn headers_stream_tools_images_and_cache_use_only_codex_backend() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta", "delta":"Une réponse"}),
        json!({"type":"response.output_item.done", "item":{"type":"function_call","id":"fc","call_id":"call","name":"remember","arguments":"{\"text\":\"thé\"}"}}),
        json!({"type":"response.completed"}),
    ]));
    let mut events = vec![];
    f.provider
        .stream(&request(), &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::TextDelta { text } if text == "Une réponse")));
    assert!(events
        .iter()
        .any(|e| matches!(e, ModelEvent::ToolCall { name, .. } if name == "remember")));
    let seen = f.server.seen.lock().unwrap();
    let headers = &seen.headers[0];
    assert_eq!(headers["chatgpt-account-id"], "account-test");
    assert_eq!(headers["originator"], "pi");
    assert_eq!(headers["openai-beta"], "responses=experimental");
    assert_eq!(headers["session-id"], "ezerbot-test");
    assert_eq!(headers["x-client-request-id"], "ezerbot-test");
    assert!(headers["authorization"].starts_with("Bearer ey"));
    let body = &seen.bodies[0];
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "Consignes locales");
    assert_eq!(
        body["reasoning"],
        json!({"effort":"high", "summary":"auto"})
    );
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(body["text"]["verbosity"], "low");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(body["tools"][0]["name"], "remember");
}

#[test]
fn quota_reset_http_sse_and_expired_refresh_are_french_and_not_retried() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let error =
        json!({"error":{"type":"usage_limit_reached", "plan_type":"plus", "resets_at":1900000000}});
    f.reply(ScriptedResponse::json(429, error.clone()));
    let message = f.raw().err().unwrap();
    assert!(message.contains("Codex") && message.contains("Réinitialisation prévue par OpenAI"));
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 1);
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.failed", "response":error}),
    ]));
    let mut events = vec![];
    f.provider
        .stream(&request(), &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
    assert!(events.iter().any(|event| matches!(event, ModelEvent::Failed { reason } if reason.contains("Réinitialisation prévue"))));
    f.reply(ScriptedResponse::json(
        401,
        json!({"error":{"code":"invalid_token"}}),
    ));
    f.server.seen.lock().unwrap().refresh_error = true;
    assert!(f.raw().err().unwrap().contains("Reconnectez-vous"));
    assert!(!f.provider.connected());
}

#[test]
fn existing_sharing_session_is_preserved_and_new_install_prefers_codex() {
    let f = Fixture::new();
    assert_eq!(f.provider.status()["backend"], "codex");
    std::fs::write(f.db.with_extension("chatgpt-auth.json"), json!({"host_id":"fake", "client_id":"fake", "access_token":"fake", "refresh_token":"fake", "expires_at":4102444800u64, "scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    let upgraded = Fixture::provider(&f.db, &f.server);
    assert_eq!(upgraded.status()["backend"], "sharing");
    upgraded.select_backend("codex").unwrap();
    assert_eq!(upgraded.status()["connections"]["sharing"], true);
    assert!(upgraded.select_backend("inconnu").is_err());
    // Neither migration nor switching modifies the separate sharing token file.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(f.db.with_extension("chatgpt-auth.json")).unwrap())
            .unwrap();
    assert_eq!(saved["refresh_token"], "fake");
}

#[test]
fn app_tools_memory_cache_quota_card_and_restart_are_durable() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["model"],
        "gpt-5.5"
    );
    let c = post(&mut app, "/api/conversations", json!({"title":"Codex"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_item.done", "item":{"type":"function_call", "id":"fc-mem", "call_id":"mem", "name":"remember", "arguments":"{\"text\":\"Je préfère le thé\"}"}}),
        json!({"type":"response.completed"}),
    ]));
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta", "delta":"C’est noté."}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"m1","body":"Retiens que je préfère le thé"}),
    );
    wait("réponse avec outil", || {
        let state = app.api("GET", "/api/state", b"").unwrap();
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "reply-m1" && m["state"] == "echoed")
    });
    let seen = f.server.seen.lock().unwrap();
    assert!(seen.bodies.len() >= 2);
    assert_eq!(
        seen.bodies[0]["prompt_cache_key"],
        seen.bodies[1]["prompt_cache_key"]
    );
    assert!(seen.bodies[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "function_call_output"));
    drop(seen);
    wait("extraction automatique via Codex", || {
        tick(&mut app);
        f.server.seen.lock().unwrap().bodies.iter().any(|body| {
            body["model"] == "gpt-6-luna"
                && body.get("max_output_tokens").is_none()
                && body["stream"] == true
                && body["include"] == json!(["reasoning.encrypted_content"])
        })
    });
    f.reply(ScriptedResponse::json(
        429,
        json!({"error":{"type":"usage_limit_reached","resets_at":1900000000}}),
    ));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"m2","body":"Encore"}),
    );
    wait("carte quota", || {
        let state = app.api("GET", "/api/state", b"").unwrap();
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "reply-m2" && m["error_kind"] == "limit")
    });
    drop(app);
    let mut reopened = App::open_with(&f.db, Fixture::provider(&f.db, &f.server)).unwrap();
    let state = reopened.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["provider"]["email"], "codex@exemple.com");
    assert!(state["conversations"][0]["messages"]
        .to_string()
        .contains("C’est noté."));
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let facts: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_facts", [], |r| r.get(0))
        .unwrap();
    assert!(facts >= 1);
    let signed_out = post(&mut reopened, "/api/provider/signout", json!({}));
    assert_eq!(signed_out["provider"]["connected"], false);
}

#[test]
fn failed_local_token_erasure_freezes_requests_until_cleanup_succeeds() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let temporary = f.db.with_extension("codex-auth.json.tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert!(f.provider.signout().is_err());
    assert_eq!(f.provider.status()["retained"], true);
    assert!(!f.provider.connected());
    assert!(f.raw().err().unwrap().contains("Reconnectez-vous"));
    assert!(f.server.seen.lock().unwrap().bodies.is_empty());
    assert!(f.provider.start_signin().is_err());
    std::fs::remove_dir(&temporary).unwrap();
    assert!(f.provider.signout().unwrap());
    assert_eq!(f.provider.status()["retained"], false);
    assert!(!Fixture::provider(&f.db, &f.server).connected());
}

fn tick(app: &mut App) {
    app.tick(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    );
}

#[test]
fn delegation_and_manual_routine_keep_the_codex_transport() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    post(
        &mut app,
        "/api/bots",
        json!({"id":"codex-child","name":"Relecteur","color":"citron","description":"Relecture","instructions":""}),
    );
    let c = post(
        &mut app,
        "/api/conversations",
        json!({"title":"Délégation Codex"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let call = |id: &str, name: &str, args: Value| json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":args.to_string()}});
    f.reply(ScriptedResponse::events(&[
        call("delegate", "delegate", json!({"child_bot":"codex-child", "objective":"Relis ce texte", "selected_context":{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}})),
        json!({"type":"response.completed"}),
    ]));
    f.reply(ScriptedResponse::events(&[
        call(
            "finish",
            "complete_task",
            json!({"summary":"Relecture Codex terminée"}),
        ),
        json!({"type":"response.completed"}),
    ]));
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"Le relecteur a terminé."}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"delegate-message","body":"Délègue la relecture"}),
    );
    let refused = app.api("POST", "/api/provider/select", br#"{"backend":"sharing"}"#).unwrap_err();
    assert_eq!(refused.status, 409);
    assert!(refused.message.contains("avant de changer de connexion"));
    assert_eq!(f.provider.status()["backend"], "codex");
    wait("délégation Codex terminée", || {
        tick(&mut app);
        let state = app.api("GET", "/api/state", b"").unwrap();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|chat| chat["id"] == c)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["id"] == "reply-delegate-message"
                    && message["body"] == "Le relecteur a terminé."
            })
    });
    let routine = post(&mut app, "/api/routines", json!({"bot_id":"grokbot","name":"Routine Codex","prompt":"Fais un bilan","schedule":"@every 1m"}))["routine"].clone();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"Bilan Codex."}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/routines/run-now",
        json!({"bot_id":"grokbot","routine_id":routine["routine_id"],"nonce":"codex-manual"}),
    );
    wait("routine Codex terminée", || {
        tick(&mut app);
        app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|chat| {
                chat["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["body"] == "Bilan Codex.")
            })
    });
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM delegation_graph WHERE child_bot='codex-child'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let status: Value = serde_json::from_str(&status).unwrap();
    assert_eq!(status["status"], "done", "{status}");
    assert!(status["result"]
        .as_str()
        .unwrap()
        .contains("Relecture Codex terminée"));
    let seen = f.server.seen.lock().unwrap();
    assert!(seen
        .bodies
        .iter()
        .all(|body| body["model"].as_str().unwrap().starts_with("gpt-")));
    assert!(seen
        .headers
        .iter()
        .all(|headers| headers["chatgpt-account-id"] == "account-test"));
}
