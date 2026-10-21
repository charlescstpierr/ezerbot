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
                json!({"model":"gpt-6.1-sol", "instructions":"Consignes", "input":[], "stream":true}),
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
        model: "gpt-6.1-sol".into(),
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
    assert_eq!(query["originator"], "ezerbot");
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
    let models = status["models"].as_array().unwrap();
    assert_eq!(models.len(), 7);
    assert_eq!(
        models
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "gpt-6.1-sol",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-luna",
            "gpt-5.6-terra"
        ]
    );
    assert!(models.iter().all(|model| !matches!(
        model["slug"].as_str(),
        Some("gpt-5.5" | "gpt-5.3-codex-spark")
    )));
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
    assert!(f.provider.select_backend("sharing").is_err());
    let restored = Fixture::provider(&f.db, &f.server);
    assert_eq!(restored.status()["backend"], "codex");
    assert!(restored.status().get("connections").is_none());
    restored.select_backend("codex").unwrap();
    assert_eq!(restored.status()["signed_in"], true);
    wait("service Codex", || restored.status()["connected"] == true);
    assert!(restored.signout().unwrap());
    assert!(!f.server.seen.lock().unwrap().revoked); // no undocumented revoke endpoint
    let restarted = Fixture::provider(&f.db, &f.server);
    assert_eq!(restarted.status()["connected"], false);
    let saved = std::fs::read_to_string(f.db.with_extension("codex-auth.json")).unwrap();
    assert!(!saved.contains("refresh-2"));
}

#[test]
fn usage_reads_wham_with_ezerbot_oauth_refresh_and_no_model_call() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    assert_eq!(f.provider.codex_usage(false)["status"], "loading");
    wait("quota Codex", || {
        f.provider.codex_usage(false)["status"] == "ready"
    });
    let quota = f.provider.codex_usage(false);
    assert_eq!(quota["quota"]["secondary"]["remaining_percent"], 18.0);
    assert_eq!(quota["quota"]["secondary"]["reset_at"], 4102531200u64);
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.forms.len(), 2); // expired Ezerbot session was refreshed
    assert_eq!(seen.forms[1]["refresh_token"], "refresh-1");
    assert_eq!(seen.quota_headers.len(), 1); // state reads coalesce
    assert_eq!(seen.quota_headers[0]["chatgpt-account-id"], "account-test");
    assert_eq!(
        seen.quota_headers[0]["authorization"],
        format!("Bearer {}", seen.access_tokens[1])
    );
    assert!(seen.bodies.is_empty());
    drop(seen);
    assert!(!quota.to_string().contains("refresh-") && !quota.to_string().contains("account-test"));
    assert!(f.provider.select_backend("sharing").is_err());
    assert_eq!(f.provider.codex_usage(false)["status"], "ready");
    f.provider.select_backend("codex").unwrap();
    f.provider.signout().unwrap();
    assert_eq!(f.provider.codex_usage(false)["status"], "signin");
}

#[test]
fn usage_retries_one_unauthorized_read_with_the_renewed_ezerbot_session() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    f.server.seen.lock().unwrap().quota_reply = Some(ScriptedResponse::json(
        401,
        json!({"error":"expired-private-token"}),
    ));
    f.provider.codex_usage(false);
    wait("quota après renouvellement", || {
        f.provider.codex_usage(false)["status"] == "ready"
    });
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.quota_headers.len(), 2);
    assert_eq!(seen.forms.len(), 3);
    assert_ne!(
        seen.quota_headers[0]["authorization"],
        seen.quota_headers[1]["authorization"]
    );
    assert_eq!(
        seen.quota_headers[1]["authorization"],
        format!("Bearer {}", seen.access_tokens[2])
    );
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
    assert_eq!(headers["originator"], "ezerbot");
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
    assert!(body.get("text").is_none());
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
fn codex_vision_caption_and_reply_use_supported_models() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    post(&mut app, "/api/preferences", json!({"model":"gpt-6-sol"}));
    let c = post(
        &mut app,
        "/api/conversations",
        json!({"title":"Image avec Codex"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=").unwrap();
    let image = app.api("POST", "/api/attachments", &png).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"Un pixel blanc."}),
        json!({"type":"response.completed"}),
    ]));
    let prepare = json!({"conversation_id":c,"attachment_id":image,"draft":"Décris ce pixel"});
    post(&mut app, "/api/attachments/precaption", prepare.clone());
    wait("description vision Codex", || {
        let result = post(&mut app, "/api/attachments/precaption", prepare.clone());
        assert_eq!(result["model"], "gpt-6-sol");
        result["caption"]["status"] == "ready"
    });
    drop(app);
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"La description montre un pixel blanc."}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"codex-image","body":"Décris ce pixel","attachments":[image]}),
    );
    wait("réponse Codex avec image", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["id"] == "reply-codex-image" && message["state"] == "echoed")
    });
    let seen = f.server.seen.lock().unwrap();
    assert!(seen.bodies.len() >= 2);
    for body in &seen.bodies[..2] {
        assert_eq!(body["model"], "gpt-6-sol");
        assert!(body["input"].to_string().contains("input_image"));
    }
}

#[test]
fn every_advertised_codex_effort_is_selected_persisted_and_sent_unchanged() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let models = app.api("GET", "/api/state", b"").unwrap()["provider"]["models"]
        .as_array()
        .unwrap()
        .clone();
    let c = post(
        &mut app,
        "/api/conversations",
        json!({"title":"Efforts Codex"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    for model in models {
        let slug = model["slug"].as_str().unwrap();
        let levels: Vec<_> = model["efforts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level["effort"].as_str().unwrap())
            .collect();
        // pi 1.3.0: off=null on Astra / 6.1 Sol, off=none elsewhere;
        // minimal maps to low, xhigh and max are explicitly supported.
        let expected = if matches!(slug, "gpt-6-astra" | "gpt-6.1-sol") {
            vec!["low", "medium", "high", "xhigh", "max"]
        } else {
            vec!["none", "low", "medium", "high", "xhigh", "max"]
        };
        assert_eq!(levels, expected, "{slug}");
        for effort in levels {
            let selected = post(
                &mut app,
                "/api/preferences",
                json!({"model":slug,"effort":effort}),
            );
            assert_eq!(selected["preferences"]["model"], slug);
            assert_eq!(selected["preferences"]["effort"], effort);
            f.reply(ScriptedResponse::events(&[
                json!({"type":"response.output_text.delta","delta":"Réponse avec effort choisi."}),
                json!({"type":"response.completed"}),
            ]));
            let message = format!("effort-{slug}-{effort}");
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id":c,"message_id":message,"body":"Réponds en texte."}),
            );
            wait("réponse avec effort choisi", || {
                app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| {
                        item["id"] == format!("reply-{message}") && item["state"] == "echoed"
                    })
            });
            let seen = f.server.seen.lock().unwrap();
            let body = seen.bodies.last().unwrap();
            assert_eq!(body["model"], slug);
            assert_eq!(body["reasoning"], json!({"effort":effort,"summary":"auto"}));
            let saved: String = conn
                .query_row(
                    "SELECT effort FROM wrapper_engine_runs ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(saved, effort);
        }
    }
}

#[test]
fn first_simple_question_uses_supported_default_and_persists_answer_text() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let c = post(&mut app, "/api/conversations", json!({"title":"Calcul"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"17 × 23 = **391**."}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"simple","body":"Combien font 17 × 23 ?"}),
    );
    wait("réponse texte au premier message", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["id"] == "reply-simple"
                    && message["state"] == "echoed"
                    && message["body"] == "17 × 23 = **391**."
            })
    });
    let seen = f.server.seen.lock().unwrap();
    let body = &seen.bodies[0];
    assert_eq!(body["model"], "gpt-6.1-sol");
    assert!(body["input"].to_string().contains("Combien font 17 × 23 ?"));
    // The emitted Responses instructions are the owned prompt contract, not
    // evidence that a scripted backend can interpret natural language.
    let instructions = body["instructions"].as_str().unwrap();
    assert!(instructions.contains(
        "For a simple question or a request to write text, answer directly in your response text"
    ));
    assert!(instructions.contains("only for genuine multi-step work requiring tools"));
    assert!(instructions.contains("A complete_task summary never replaces the answer"));
    drop(seen);
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let record: String = conn
        .query_row(
            "SELECT record FROM engine_runs WHERE conversation_id=?1",
            [&c],
            |row| row.get(0),
        )
        .unwrap();
    let run: Value = serde_json::from_str(&record).unwrap();
    assert_eq!(run["kind"], "chat");
    assert!(run["task_status"].is_null());
    let tool_calls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_run_events WHERE kind='tool_call'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tool_calls, 0);
}

#[test]
fn retired_preferences_migrate_durably_and_cannot_select_another_subscription() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    post(
        &mut app,
        "/api/preferences",
        json!({"model":"gpt-6-sol","effort":"high","fast":true}),
    );
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    for retired in ["gpt-5.5", "gpt-5.3-codex-spark"] {
        conn.execute("UPDATE wrapper_preferences SET model=?1", [retired])
            .unwrap();
        drop(app);
        let restarted = Fixture::provider(&f.db, &f.server);
        assert!(restarted.models().is_empty());
        app = App::open_with(&f.db, restarted).unwrap();
        app.api("GET", "/api/state", b"").unwrap();
        // Migration is durable even while the restarted catalogue is loading.
        let saved: String = conn
            .query_row("SELECT model FROM wrapper_preferences", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(saved, "gpt-6.1-sol");
        wait("catalogue après redémarrage", || {
            let state = app.api("GET", "/api/state", b"").unwrap();
            assert_eq!(state["preferences"]["fast"], true);
            state["preferences"]["model"] == "gpt-6.1-sol"
                && state["preferences"]["effort"] == "high"
        });
    }
    post(&mut app, "/api/preferences", json!({"model":"gpt-6-sol"}));
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["model"],
        "gpt-6-sol"
    );
    assert!(app
        .api("POST", "/api/provider/select", br#"{"backend":"sharing"}"#)
        .is_err());
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["preferences"]["model"],
        "gpt-6-sol"
    );
}

#[test]
fn unsupported_model_refusal_proposes_switching_without_retry_or_signout() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    for retired in ["gpt-5.5", "gpt-5.3-codex-spark"] {
        let before = f.server.seen.lock().unwrap().bodies.len();
        let failure = f
            .provider
            .responses(json!({"model":retired,"input":[]}))
            .unwrap_err();
        assert!(failure
            .message
            .contains("bouton « Modèle » sous la zone de message"));
        assert!(failure.message.contains(retired));
        assert_eq!(failure.kind, "other");
        assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before + 1);
        assert!(f.provider.connected());
    }
    for body in [
        json!({"detail":"The 'gpt-5.5' model is not supported when using Codex with a ChatGPT account."}),
        json!({"error":{"type":"invalid_request_error","message":"The 'gpt-5.5' model is not supported when using Codex with a ChatGPT account."}}),
        json!({"detail":"This model is not supported."}),
    ] {
        let before = f.server.seen.lock().unwrap().bodies.len();
        let identifies_model = body.to_string().contains("gpt-5.5");
        f.reply(ScriptedResponse::json(400, body));
        let message = f.raw().unwrap_err();
        assert!(
            message.contains("bouton « Modèle » sous la zone de message"),
            "{message}"
        );
        assert!(message.contains("puis relancez la réponse ou envoyez un nouveau message"));
        assert!(!message.contains("cliquez sur Relancer"));
        assert!(!message.contains("en haut"));
        assert_eq!(message.contains("gpt-5.5"), identifies_model);
        assert!(!message.contains("Reconnectez-vous"));
        assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before + 1);
        assert!(f.provider.connected());
    }
}

#[test]
fn plan_without_codex_is_not_a_temporary_quota_limit() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    f.reply(ScriptedResponse::json(
        403,
        json!({"error":{"type":"usage_not_included", "resets_at":1900000000}}),
    ));
    let failure = f
        .provider
        .responses(json!({"model":"gpt-6.1-sol","stream":true,"input":[]}))
        .unwrap_err();
    assert_eq!(failure.kind, "other");
    assert!(failure
        .message
        .contains("Votre forfait ChatGPT n’inclut pas Codex"));
    assert!(failure.message.contains("Sign in with ChatGPT"));
    assert!(!failure.message.contains("Attendez") && !failure.message.contains("Réinitialisation"));
    assert!(!failure.message.contains("abonnement") && !failure.message.contains("partage"));
}

#[test]
fn old_sharing_selection_is_ignored_without_touching_legacy_credentials() {
    let f = Fixture::new();
    assert_eq!(f.provider.status()["backend"], "codex");
    std::fs::write(f.db.with_extension("chatgpt-auth.json"), json!({"host_id":"fake", "client_id":"fake", "access_token":"fake", "refresh_token":"fake", "expires_at":4102444800u64, "scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    std::fs::write(f.db.with_extension("provider.json"), "\"sharing\"").unwrap();
    let upgraded = Fixture::provider(&f.db, &f.server);
    assert_eq!(upgraded.status()["backend"], "codex");
    assert_eq!(upgraded.status()["signed_in"], false);
    assert!(upgraded.select_backend("sharing").is_err());
    upgraded.select_backend("codex").unwrap();
    assert!(upgraded.status().get("connections").is_none());
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
        "gpt-6.1-sol"
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
    let complete_result = "Voici le paragraphe relu en entier : le texte demandé reste disponible pour le bot parent.";
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":complete_result}),
        call(
            "finish",
            "complete_task",
            json!({"summary":complete_result}),
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
    let refused = app
        .api("POST", "/api/provider/select", br#"{"backend":"sharing"}"#)
        .unwrap_err();
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
        call(
            "routine-finish",
            "complete_task",
            json!({"summary":"Bilan Codex."}),
        ),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/routines/run-now",
        json!({"bot_id":"grokbot","routine_id":routine["routine_id"],"nonce":"codex-manual"}),
    );
    wait("routine Codex terminée", || {
        tick(&mut app);
        let history = post(
            &mut app,
            "/api/routines/history",
            json!({"bot_id":"grokbot","routine_id":routine["routine_id"]}),
        );
        history["occurrences"][0]["status"]["status"] == "done"
    });
    assert!(app.api("GET", "/api/state", b"").unwrap()["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|chat| {
            chat["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["body"] == "Bilan Codex.")
        }));
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
    assert_eq!(status["result"], complete_result);
    let seen = f.server.seen.lock().unwrap();
    let task_requests: Vec<_> = seen
        .bodies
        .iter()
        .filter(|body| {
            body["instructions"]
                .as_str()
                .is_some_and(|text| text.contains("You are executing a Task"))
        })
        .collect();
    assert_eq!(
        task_requests.len(),
        2,
        "one delegated task and one routine occurrence"
    );
    for body in task_requests {
        let instructions = body["instructions"].as_str().unwrap();
        assert!(instructions.contains("then call complete_task(summary) when the task is finished"));
        assert!(instructions
            .contains("for a delegated task the summary must contain the complete result"));
        assert!(!instructions.contains("without begin_task or complete_task"));
        assert!(!instructions.contains("A complete_task summary never replaces the answer"));
    }
    assert!(seen
        .bodies
        .iter()
        .all(|body| body["model"].as_str().unwrap().starts_with("gpt-")));
    assert!(seen
        .headers
        .iter()
        .all(|headers| headers["chatgpt-account-id"] == "account-test"));
}

#[test]
fn connected_requires_a_real_openai_call_and_a_failed_probe_still_sends() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().probe_gate = Some(gate.clone());
    f.login();
    let first = f.provider.status();
    assert_eq!(first["signed_in"], true);
    assert_eq!(first["connected"], false);
    assert_eq!(first["connection_test"]["state"], "running");
    assert!(first.get("connections").is_none());
    gate.store(true, Ordering::SeqCst);
    wait("appel OpenAI confirmé", || {
        f.provider.status()["connected"] == true
    });
    {
        let seen = f.server.seen.lock().unwrap();
        assert!(
            seen.bodies.is_empty(),
            "la sonde ne doit pas compter comme un tour de chat"
        );
        let probe = &seen.probe_bodies[0];
        assert!(probe["instructions"]
            .as_str()
            .unwrap()
            .starts_with("Test de connexion."));
        assert!(probe["input"].to_string().contains("Réponds seulement OK."));
        assert_ne!(probe["model"], "gpt-5.5");
        assert!(probe.get("tool_choice").is_none(), "{probe}");
        assert!(probe.get("parallel_tool_calls").is_none(), "{probe}");
    }
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let conversation = post(
        &mut app,
        "/api/conversations",
        json!({"title":"Vérification"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"391"}),
        json!({"type":"response.completed"}),
    ]));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"q1","body":"Combien font 17 × 23 ?"}),
    );
    wait("réponse du modèle", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["id"] == "reply-q1" && message["body"] == "391")
    });
    let question = f.server.seen.lock().unwrap().bodies[0].clone();
    assert!(question["input"]
        .to_string()
        .contains("Combien font 17 × 23 ?"));
    assert!(!question["instructions"]
        .as_str()
        .unwrap_or_default()
        .starts_with("Test de connexion."));
    drop(app);
    drop(f);

    let denied = Fixture::new();
    denied.server.seen.lock().unwrap().fail_probe = true;
    denied.login();
    wait("sonde refusée", || {
        denied.provider.status()["connection_test"]["state"] == "failed"
    });
    assert_eq!(denied.provider.status()["signed_in"], true);
    assert_eq!(denied.provider.status()["connected"], false);
    let mut app = App::open_with(&denied.db, denied.provider.clone()).unwrap();
    let conversation = post(&mut app, "/api/conversations", json!({"title":"Refus"}))
        ["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    denied.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"bonjour quand même"}),
        json!({"type":"response.completed"}),
    ]));
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"q2","body":"Dis bonjour"}),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    wait("réponse malgré la sonde refusée", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["id"] == "reply-q2" && message["body"] == "bonjour quand même")
    });
    let sent = app.api("GET", "/api/state", b"").unwrap();
    let reply = sent["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == "reply-q2")
        .unwrap();
    assert_eq!(reply["state"], "echoed");
    assert!(reply["error"].is_null());
    assert!(denied.server.seen.lock().unwrap().bodies[0]["input"]
        .to_string()
        .contains("Dis bonjour"));
    assert_eq!(denied.provider.status()["connected"], false);
}

#[test]
fn a_pending_probe_lets_the_message_leave_without_freezing_the_interface() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().probe_gate = Some(gate.clone());
    f.login();
    wait("sonde encore en cours", || {
        f.provider.status()["connection_test"]["state"] == "running"
    });
    assert_eq!(f.provider.status()["connected"], false);
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let conversation = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"parti tout de suite"}),
        json!({"type":"response.completed"}),
    ]));
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"now","body":"Question immédiate"}),
    );
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "l'envoi a attendu la sonde"
    );
    let start = std::time::Instant::now();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "l'état a gelé pendant la vérification"
    );
    assert_eq!(state["provider"]["connection_test"]["state"], "running");
    assert_eq!(state["provider"]["connected"], false);
    wait("réponse pendant que la sonde est bloquée", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["id"] == "reply-now"
                    && message["body"] == "parti tout de suite"
                    && message["state"] == "echoed"
            })
    });
    {
        let seen = f.server.seen.lock().unwrap();
        assert!(seen.bodies[0]["input"]
            .to_string()
            .contains("Question immédiate"));
        assert!(!seen.bodies[0]["instructions"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Test de connexion."));
    }
    assert_eq!(f.provider.status()["connection_test"]["state"], "running");
    assert_eq!(f.provider.status()["connected"], false);
    assert!(
        !gate.load(Ordering::SeqCst),
        "la réponse ne doit pas dépendre de la fin de la sonde"
    );
    gate.store(true, Ordering::SeqCst);
}

#[test]
fn sending_during_verification_never_holds_the_app_mutex_and_stop_fences_the_reply() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().probe_gate = Some(gate.clone());
    f.login();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let c = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"pending","body":"Question conservée"}),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    let start = std::time::Instant::now();
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert_eq!(state["provider"]["connection_test"]["state"], "running");
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/stop",
        json!({"conversation_id":c,"message_id":"pending"}),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    assert!(
        !gate.load(Ordering::SeqCst),
        "Arrêter a attendu la fin de la sonde"
    );
    gate.store(true, Ordering::SeqCst);
    wait("arrêt pendant vérification", || {
        app.api("GET", "/api/state", b"").unwrap()["provider"]["titling"] == false
    });
    let bodies = f.server.seen.lock().unwrap().bodies.clone();
    if let Some(body) = bodies.first() {
        assert!(body["input"].to_string().contains("Question conservée"));
    }
}

#[test]
fn live_catalog_uses_account_residency_and_only_advertises_account_efforts() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.server.seen.lock().unwrap().codex_catalog = Some(json!({"models":[
        {"slug":"gpt-6-sol","display_name":"GPT-6 Sol","visibility":"list","supported_reasoning_levels":[{"effort":"low"}]},
        {"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list","default_reasoning_level":"high","supported_reasoning_levels":[{"effort":"high"}]},
        {"slug":"hidden","visibility":"hide"}
    ]}));
    f.login();
    let models = f.provider.models();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].slug, "gpt-6.1-sol");
    assert_eq!(models[0].default_effort.as_deref(), Some("high"));
    assert_eq!(models[0].efforts.len(), 1);
    assert_eq!(models[0].efforts[0].effort, "high");
    let seen = f.server.seen.lock().unwrap();
    let headers = &seen.codex_model_headers[0];
    assert_eq!(headers["chatgpt-account-id"], "account-test");
    assert_eq!(headers["x-openai-internal-codex-residency"], "eu");
    assert_eq!(headers["originator"], "ezerbot");
}

#[test]
fn refresh_test_rotates_the_same_ezerbot_grant_off_the_http_thread() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.login();
    wait("connexion", || f.provider.status()["connected"] == true);
    let before = f.server.seen.lock().unwrap().forms.len();
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let start = std::time::Instant::now();
    post(&mut app, "/api/provider/test", json!({"refresh":true}));
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    wait("rotation et nouvelle sonde", || {
        f.provider.status()["connected"] == true
    });
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.forms.len(), before + 1);
    assert_eq!(seen.forms.last().unwrap()["grant_type"], "refresh_token");
    assert_eq!(seen.forms.last().unwrap()["client_id"], CLIENT);
    let log = std::fs::read_to_string(f.db.parent().unwrap().join("logs/engine.log")).unwrap();
    assert!(log.contains("status=200 code=token_refresh_saved"));
    assert!(!log.contains("refresh-"));
}

#[test]
fn an_incomplete_or_empty_probe_cannot_claim_chatgpt_is_connected() {
    let _serial = OAUTH.lock().unwrap();
    for event in [
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
        json!({"type":"response.completed"}),
    ] {
        let f = Fixture::new();
        f.server.seen.lock().unwrap().probe_reply = Some(ScriptedResponse::events(&[event]));
        f.login();
        wait("sonde sans réponse complète", || {
            f.provider.status()["connection_test"]["state"] == "failed"
        });
        assert_eq!(f.provider.status()["connected"], false);
    }
}

#[test]
fn retry_relaunches_the_probe_then_returns_the_complete_default_model_reply() {
    let _serial = OAUTH.lock().unwrap();
    let f = Fixture::new();
    f.server.seen.lock().unwrap().fail_probe = true;
    f.login();
    wait("sonde refusée", || {
        f.provider.status()["connection_test"]["state"] == "failed"
    });
    let mut app = App::open_with(&f.db, f.provider.clone()).unwrap();
    let c = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    post(
        &mut app,
        "/api/preferences",
        json!({"model":"gpt-6.1-sol","effort":"high"}),
    );
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"premier envoi"}),
        json!({"type":"response.completed"}),
    ]));
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":c,"message_id":"retry","body":"Une réponse complète"}),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    wait("premier envoi malgré la sonde refusée", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == "premier envoi" && m["state"] == "echoed")
    });
    let before = f.server.seen.lock().unwrap().probe_bodies.len();
    f.server.seen.lock().unwrap().fail_probe = false;
    let complete = "Voici la réponse complète.\n\n1. Première explication détaillée.\n2. Deuxième explication détaillée.\n3. Conclusion demandée.";
    f.reply(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":complete}),
        json!({"type":"response.completed"}),
    ]));
    let start = std::time::Instant::now();
    post(
        &mut app,
        "/api/regenerate",
        json!({"conversation_id":c,"message_id":"retry"}),
    );
    assert!(start.elapsed() < std::time::Duration::from_millis(500));
    wait("réponse après relance", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["body"] == complete && m["state"] == "echoed")
    });
    let seen = f.server.seen.lock().unwrap();
    assert!(seen.probe_bodies.len() > before);
    assert_eq!(seen.bodies[0]["model"], "gpt-6.1-sol");
    assert_eq!(seen.bodies[0]["reasoning"]["effort"], "high");
    assert!(seen.bodies[0].get("text").is_none());
}
