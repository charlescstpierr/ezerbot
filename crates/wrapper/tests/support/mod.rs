//! Shared OAuth/Responses fixture. Queue raw replies to exercise tool calls, failures or vision
//! without coupling feature tests to the default text-only conversation.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::Endpoints;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};

// Disposable test key, not a credential.
const SIGNING_KEY: &[u8] = include_bytes!("../fixtures/test-signing-key.pem");
const JWKS: &str = include_str!("../fixtures/test-jwks.json");
pub const CLIENT: &str = "oaiapp_test";

/// Session file the production binary reads. The access value is an unsigned
/// test marker (header `{}`), never a live ChatGPT token.
pub fn write_codex_session(path: &std::path::Path, account_id: &str) {
    use base64::Engine;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = format!(
        "e30.{}.e30",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id": account_id}}).to_string()
        )
    );
    std::fs::write(
        path,
        json!({
            "host_id": "test-host",
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "account_id": account_id,
            "access_token": token,
            "refresh_token": "refresh-test",
            "expires_at": now + 3600,
            "welcomed": true
        })
        .to_string(),
    )
    .unwrap();
}

#[derive(Default)]
pub struct Seen {
    pub nonce: String,
    pub forms: Vec<HashMap<String, String>>,
    pub headers: Vec<HashMap<String, String>>,
    pub refresh_error: bool,
    /// HTTP 503 from the token endpoint: the refresh token is still good.
    pub refresh_transient: bool,
    /// Nombre de refus explicites avant un renouvellement réussi.
    pub refresh_refusals: u32,
    /// Replaces the scope string on a successful refresh. `None` keeps the fixture scope.
    pub refresh_scope: Option<String>,
    pub invalid_codex_account: bool,
    pub token_gate: Option<Arc<AtomicBool>>,
    pub bearer: Vec<String>,
    /// Distinct issued tokens let tests detect reuse of a stale bearer after refresh.
    pub access_tokens: Vec<String>,
    pub bodies: Vec<Value>,
    /// Connection probes are not chat turns: they must not consume scripted replies.
    pub probe_bodies: Vec<Value>,
    pub probe_reply: Option<ScriptedResponse>,
    pub codex_catalog: Option<Value>,
    pub codex_model_headers: Vec<HashMap<String, String>>,
    pub fail_probe: bool,
    pub probe_gate: Option<Arc<AtomicBool>>,
    pub revoked: bool,
    /// Header supplied by the fake Responses server, never an event/body identifier.
    pub request_id: Option<String>,
    /// FIFO replies for /v1/responses. An empty queue uses the original chat fixture.
    pub responses: VecDeque<ScriptedResponse>,
    /// Automatic memory runs independently; never consume a chat's scripted reply.
    pub memory_responses: VecDeque<ScriptedResponse>,
    /// Résumés de réunions Recall : un appel ponctuel, jamais une réponse de conversation.
    pub summary_responses: VecDeque<ScriptedResponse>,
    pub summary_requests: Vec<Value>,
    pub memory_model: bool,
    pub memory_delay: Duration,
    /// Optional provider telemetry for default chat replies (titles remain independent).
    pub usage: Option<Value>,
    /// GET WHAM usage is independent of model responses and their telemetry.
    pub quota_reply: Option<ScriptedResponse>,
    pub quota_headers: Vec<HashMap<String, String>>,
    pub quota_gate: Option<Arc<AtomicBool>>,
    /// Hold a catalogue or a numbered response without locking observations.
    pub models_gate: Option<Arc<AtomicBool>>,
    pub model_requests: usize,
    pub response_gate: Option<(usize, Arc<AtomicBool>)>,
    /// Test-only delay before sending Responses headers (without holding Seen).
    pub response_delay: Option<Duration>,
    /// Image generations are a separate Codex responses call from chat.
    pub image_calls: Vec<Value>,
    pub image_authorization: Vec<String>,
    pub image_headers: Vec<HashMap<String, String>>,
    pub image_urls: Vec<String>,
    pub image_reply: VecDeque<ScriptedResponse>,
    /// Held without the Seen lock so a test can observe the in-flight call.
    pub image_gate: Option<Arc<AtomicBool>>,
    /// Optional answer derived from the request. `None` keeps the scripted queue.
    pub reply_for: Option<ReplyFor>,
}

/// Test hook: inspect the request and optionally replace the scripted reply.
pub type ReplyFor = Arc<dyn Fn(&Value) -> Option<ScriptedResponse> + Send + Sync>;

pub struct ScriptedResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

impl ScriptedResponse {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json".into(),
            body: body.to_string(),
        }
    }

    /// Responses-shaped transport: repeated response objects/instructions and
    /// one metadata-rich delta per four Unicode characters (roughly one token).
    /// Deliberately not the tiny, one-delta convenience fixture.
    pub fn openai_text(text: &str, instructions: &str) -> Self {
        let item_id = format!("msg_{}", "a".repeat(48));
        let mut response = json!({"id":format!("resp_{}","b".repeat(48)),"object":"response","status":"in_progress","created_at":1800000000,"instructions":instructions,"model":"gpt-test-mini","reasoning":{"effort":"low"},"text":{"format":{"type":"text"}},"tools":[],"usage":null,"metadata":{},"output":[]});
        let mut events = vec![
            json!({"type":"response.created","response":response}),
            json!({"type":"response.in_progress","response":response}),
        ];
        let reasoning =
            json!({"id":format!("rs_{}","c".repeat(48)),"type":"reasoning","summary":[]});
        events.push(json!({"type":"response.output_item.added","output_index":0,"item":reasoning}));
        events.push(json!({"type":"response.output_item.done","output_index":0,"item":reasoning}));
        let mut message = json!({"id":item_id,"type":"message","role":"assistant","status":"in_progress","content":[]});
        events.push(json!({"type":"response.output_item.added","output_index":1,"item":message}));
        events.push(json!({"type":"response.content_part.added","item_id":item_id,"output_index":1,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
        for chunk in text.chars().collect::<Vec<_>>().chunks(4) {
            events.push(json!({"type":"response.output_text.delta","item_id":item_id,"output_index":1,"content_index":0,"delta":chunk.iter().collect::<String>(),"logprobs":[],"obfuscation":"0123456789abcdef0123456789abcdef"}));
        }
        let part = json!({"type":"output_text","text":text,"annotations":[],"logprobs":[]});
        events.push(json!({"type":"response.output_text.done","item_id":item_id,"output_index":1,"content_index":0,"text":text}));
        events.push(json!({"type":"response.content_part.done","item_id":item_id,"output_index":1,"content_index":0,"part":part}));
        message["content"] = json!([part]);
        message["status"] = json!("completed");
        events.push(json!({"type":"response.output_item.done","output_index":1,"item":message}));
        response["output"] = json!([reasoning, message]);
        response["status"] = json!("completed");
        response["usage"] = json!({"input_tokens":500,"output_tokens":400,"input_tokens_details":{"cached_tokens":0}});
        events.push(json!({"type":"response.completed","response":response}));
        for (sequence, event) in events.iter_mut().enumerate() {
            event["sequence_number"] = json!(sequence);
        }
        Self {
            status: 200,
            content_type: "text/event-stream".into(),
            body: events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect(),
        }
    }

    pub fn events(events: &[Value]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream".into(),
            body: events
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect(),
        }
    }
}

pub fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_owned(), unescape(value)))
        .collect()
}

fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            out.push(u8::from_str_radix(&value[index + 1..index + 3], 16).unwrap());
            index += 3;
        } else {
            out.push(if bytes[index] == b'+' {
                b' '
            } else {
                bytes[index]
            });
            index += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn id_token(issuer: &str, nonce: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key".into());
    let claims = json!({
        "iss": issuer, "aud": CLIENT, "sub": "user-1", "email": "vous@exemple.com",
        "nonce": nonce, "iat": now, "exp": now + 600,
    });
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(SIGNING_KEY).unwrap(),
    )
    .unwrap()
}

/// The first access token expires immediately, forcing renewal before the first API call.
/// Dropping the fixture stops its thread instead of leaving listeners behind between tests.
pub struct FakeOpenAi {
    pub base: String,
    pub seen: Arc<Mutex<Seen>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeOpenAi {
    pub fn new(seen: Arc<Mutex<Seen>>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!(
            "http://127.0.0.1:{}",
            server.server_addr().to_ip().unwrap().port()
        );
        let issuer = base.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let observations = seen.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let bearer = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("Authorization"))
                    .map(|header| header.value.to_string())
                    .unwrap_or_default();
                let mut seen = observations.lock().unwrap();
                let response_delay = if matches!(
                    request.url(),
                    "/v1/responses" | "/backend-api/codex/responses"
                ) {
                    seen.response_delay
                } else {
                    None
                };
                let mut delay = Duration::ZERO;
                let mut is_probe = false;
                let mut is_image = false;
                let mut probe_gate = None;
                let (status, content_type, payload) = match request.url() {
                    "/jwks" => (200, "application/json".to_owned(), JWKS.to_owned()),
                    "/token" => {
                        let fields = form(&body);
                        let refresh = fields["grant_type"] == "refresh_token";
                        let codex = fields["client_id"] == "app_EMoamEEZ73f0CkXaXp7hrann";
                        seen.forms.push(fields);
                        let sequence = seen.access_tokens.len() + 1;
                        let access = if codex {
                            let auth = if seen.invalid_codex_account {
                                json!({})
                            } else {
                                json!({"chatgpt_account_id":"account-test", "chatgpt_plan_type":"plus", "chatgpt_data_residency":"eu"})
                            };
                            encode(&Header::new(Algorithm::RS256), &json!({
                                "https://api.openai.com/auth":auth, "email":"codex@exemple.com", "sequence":sequence,
                            }), &EncodingKey::from_rsa_pem(SIGNING_KEY).unwrap()).unwrap()
                        } else {
                            format!("access-{sequence}")
                        };
                        seen.access_tokens.push(access.clone());
                        let default_scope = "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke";
                        let scope = if refresh {
                            seen.refresh_scope
                                .clone()
                                .unwrap_or_else(|| default_scope.into())
                        } else {
                            default_scope.into()
                        };
                        let reply = json!({
                            "access_token": access,
                            "refresh_token": format!("refresh-{sequence}"),
                            "id_token": if codex { access.clone() } else { id_token(&issuer, &seen.nonce) },
                            "token_type": "Bearer", "expires_in": if refresh { 3600 } else { 1 },
                            "scope": scope,
                        });
                        if refresh && seen.refresh_transient {
                            (
                                503,
                                "application/json".into(),
                                json!({"error":{"code":"temporarily_unavailable"}}).to_string(),
                            )
                        } else if refresh && seen.refresh_error {
                            (
                                400,
                                "application/json".into(),
                                json!({"error":{"code":"invalid_grant"}}).to_string(),
                            )
                        } else if refresh && seen.refresh_refusals > 0 {
                            seen.refresh_refusals -= 1;
                            (
                                400,
                                "application/json".into(),
                                json!({"error":{"code":"invalid_grant"}}).to_string(),
                            )
                        } else {
                            (200, "application/json".into(), reply.to_string())
                        }
                    }
                    "/revoke" => {
                        seen.revoked = true;
                        (200, "application/json".into(), String::new())
                    }
                    "/backend-api/wham/usage" => {
                        seen.quota_headers.push(
                            request
                                .headers()
                                .iter()
                                .map(|h| {
                                    (
                                        h.field.to_string().to_ascii_lowercase(),
                                        h.value.to_string(),
                                    )
                                })
                                .collect(),
                        );
                        let reply = seen.quota_reply.take().unwrap_or_else(|| ScriptedResponse::json(200, json!({"plan_type":"plus","rate_limit":{"primary_window":{"used_percent":35,"limit_window_seconds":18000,"reset_at":4102444800u64},"secondary_window":{"used_percent":82,"limit_window_seconds":604800,"reset_at":4102531200u64}}})));
                        (reply.status, reply.content_type, reply.body)
                    }
                    "/backend-api/codex/models?client_version=99.0.0" => {
                        seen.codex_model_headers.push(
                            request
                                .headers()
                                .iter()
                                .map(|h| {
                                    (
                                        h.field.to_string().to_ascii_lowercase(),
                                        h.value.to_string(),
                                    )
                                })
                                .collect(),
                        );
                        match &seen.codex_catalog {
                            Some(catalog) => (200, "application/json".into(), catalog.to_string()),
                            None => (404, "application/json".into(), "{}".into()),
                        }
                    }
                    "/v1/models" => {
                        seen.model_requests += 1;
                        seen.bearer.push(bearer);
                        let mut models = json!({"models": [
                            {"slug": "gpt-test", "display_name": "GPT Test", "description": "Modèle de test", "visibility": "list",
                             "default_reasoning_level": "medium", "default_reasoning_summary": "none",
                             "supported_reasoning_levels": [{"effort": "low", "description": "Léger"}, {"effort": "medium", "description": "Équilibré"}, {"effort": "high", "description": "Profond"}],
                             "service_tiers": [{"id": "priority", "name": "Fast", "description": "2x speed"}]},
                            {"slug": "internal", "display_name": "Internal", "visibility": "hide"},
                        ]});
                        if seen.memory_model {
                            models["models"].as_array_mut().unwrap().push(json!({"slug":"gpt-test-mini","display_name":"GPT Test Mini","visibility":"list","supported_reasoning_levels":[{"effort":"low","description":"Léger"}]}));
                        }
                        (200, "application/json".into(), models.to_string())
                    }
                    "/v1/responses" | "/backend-api/codex/responses" => {
                        let request_body: Value =
                            serde_json::from_str(&body).unwrap_or(Value::Null);
                        if request_body["tools"].as_array().is_some_and(|tools| {
                            tools.iter().any(|tool| tool["type"] == "image_generation")
                        }) {
                            is_image = true;
                            seen.image_authorization.push(bearer.clone());
                            seen.image_headers.push(
                                request
                                    .headers()
                                    .iter()
                                    .map(|header| {
                                        (
                                            header.field.to_string().to_ascii_lowercase(),
                                            header.value.to_string(),
                                        )
                                    })
                                    .collect(),
                            );
                            seen.image_urls.push(request.url().to_owned());
                            seen.image_calls.push(request_body);
                            let reply = seen.image_reply.pop_front().unwrap_or_else(|| {
                                ScriptedResponse::json(
                                    200,
                                    json!({"output":[{"type":"image_generation_call","status":"completed","result":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII="}]}),
                                )
                            });
                            (reply.status, reply.content_type, reply.body)
                        } else if request_body["instructions"]
                            .as_str()
                            .unwrap_or_default()
                            .starts_with("Test de connexion.")
                        {
                            is_probe = true;
                            probe_gate = seen.probe_gate.clone();
                            seen.probe_bodies.push(request_body);
                            if let Some(reply) = seen.probe_reply.take() {
                                (reply.status, reply.content_type, reply.body)
                            } else if seen.fail_probe {
                                (
                                    401,
                                    "application/json".into(),
                                    json!({"error":{"message":"probe refused"}}).to_string(),
                                )
                            } else {
                                let ok = ScriptedResponse::events(&[
                                    json!({"type":"response.output_text.delta","delta":"OK"}),
                                    json!({"type":"response.completed"}),
                                ]);
                                (ok.status, ok.content_type, ok.body)
                            }
                        } else {
                            seen.headers.push(
                                request
                                    .headers()
                                    .iter()
                                    .map(|h| {
                                        (
                                            h.field.to_string().to_ascii_lowercase(),
                                            h.value.to_string(),
                                        )
                                    })
                                    .collect(),
                            );
                            seen.bearer.push(bearer);
                            seen.bodies.push(request_body.clone());
                            if request.url() == "/backend-api/codex/responses"
                                && matches!(
                                    request_body["model"].as_str(),
                                    Some("gpt-5.5" | "gpt-5.3-codex-spark")
                                )
                            {
                                // Real-account refusals observed on 2026-10-20. Keep
                                // fresh-install tests from succeeding with retired defaults.
                                (400, "application/json".into(), json!({"detail":format!(
                                "The '{}' model is not supported when using Codex with a ChatGPT account.",
                                request_body["model"].as_str().unwrap()
                            )}).to_string())
                            } else if let Some(key) = request_body["prompt_cache_key"]
                                .as_str()
                                .filter(|key| key.chars().count() > 64)
                            {
                                // Mirror the Responses constraint before consuming scripted replies.
                                let error = json!({"error": {
                                    "type":"invalid_request_error", "code":"string_above_max_length",
                                    "param":"prompt_cache_key",
                                    "message":format!("Invalid 'prompt_cache_key': string too long. Expected a string with maximum length 64, but got a string with length {} instead.", key.chars().count()),
                                }});
                                (400, "application/json".into(), error.to_string())
                            } else if request_body["instructions"]
                                .as_str()
                                .unwrap_or_default()
                                .starts_with("Extraction de mémoire automatique.")
                            {
                                let reply = seen.memory_responses.pop_front().unwrap_or_else(|| ScriptedResponse::events(&[
                                json!({"type":"response.output_text.delta","delta":"{\"facts\":[]}"}),
                                json!({"type":"response.completed"}),
                            ]));
                                delay = seen.memory_delay;
                                (reply.status, reply.content_type, reply.body)
                            } else if request_body["instructions"]
                                .as_str()
                                .unwrap_or_default()
                                .starts_with("Résumé de réunion Recall.")
                            {
                                seen.summary_requests.push(request_body.clone());
                                let reply = seen.summary_responses.pop_front().unwrap_or_else(|| ScriptedResponse::events(&[
                                json!({"type":"response.output_text.delta","delta":"Décisions : aucune."}),
                                json!({"type":"response.completed"}),
                            ]));
                                (reply.status, reply.content_type, reply.body)
                            } else if let Some(reply) = seen
                                .reply_for
                                .clone()
                                .and_then(|reply| reply(&request_body))
                            {
                                (reply.status, reply.content_type, reply.body)
                            } else if let Some(reply) = seen.responses.pop_front() {
                                (reply.status, reply.content_type, reply.body)
                            } else if request_body["instructions"]
                                .as_str()
                                .unwrap_or_default()
                                .contains("titre")
                            {
                                let title = json!({"type": "response.output_text.delta", "delta": "« Salutations en français. »"});
                                (
                                    200,
                                    "text/event-stream".into(),
                                    format!(
                                        "data: {title}\n\ndata: {}\n\n",
                                        json!({"type": "response.completed"})
                                    ),
                                )
                            } else if request_body.get("service_tier").is_some() {
                                let refusal = json!({"error": {"code": "subscription_sharing_unsupported_capability", "message": "unsupported", "param": "service_tier"}});
                                (400, "application/json".into(), refusal.to_string())
                            } else {
                                let events = [
                                    json!({"type": "response.created"}),
                                    json!({"type": "response.output_text.delta", "delta": "Bon"}),
                                    json!({"type": "response.output_text.delta", "delta": "jour **vous**"}),
                                    json!({"type": "response.completed", "response": {"usage": seen.usage}}),
                                ];
                                let stream = events
                                    .iter()
                                    .map(|event| {
                                        format!(
                                            "event: {}\ndata: {event}\n\n",
                                            event["type"].as_str().unwrap()
                                        )
                                    })
                                    .collect();
                                (200, "text/event-stream".into(), stream)
                            }
                        }
                    }
                    _ => (404, "text/plain".into(), String::new()),
                };
                let gate = if is_probe {
                    None
                } else if is_image {
                    seen.image_gate.clone()
                } else {
                    match request.url() {
                        "/v1/models" => seen.models_gate.clone(),
                        // Codex-session fixtures call the backend path: gate/delay it too.
                        "/v1/responses" | "/backend-api/codex/responses" => seen
                            .response_gate
                            .as_ref()
                            .filter(|(at, _)| *at == seen.bodies.len())
                            .map(|(_, gate)| gate.clone()),
                        _ => None,
                    }
                };
                let gate = if request.url() == "/token" {
                    seen.token_gate.clone()
                } else if request.url() == "/backend-api/wham/usage" {
                    seen.quota_gate.clone()
                } else {
                    gate
                };
                let request_id = seen.request_id.clone().filter(|_| {
                    matches!(
                        request.url(),
                        "/v1/responses" | "/backend-api/codex/responses"
                    )
                });
                drop(seen);
                let header = tiny_http::Header::from_bytes("Content-Type", content_type).unwrap();
                let mut response = tiny_http::Response::from_string(payload)
                    .with_status_code(status)
                    .with_header(header);
                if let Some(id) = request_id {
                    response = response
                        .with_header(tiny_http::Header::from_bytes("x-request-id", id).unwrap());
                }
                if let Some(gate) = probe_gate {
                    // A held probe must not occupy the accept loop. The real
                    // service accepts the user's question while the check runs.
                    let stopping = stopping.clone();
                    std::thread::spawn(move || {
                        while !gate.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        let _ = request.respond(response);
                    });
                    continue;
                }
                if let Some(gate) = gate {
                    while !gate.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                if let Some(delay) = response_delay {
                    std::thread::sleep(delay);
                }
                std::thread::sleep(delay);
                let _ = request.respond(response);
            }
        });
        Self {
            base,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    pub fn endpoints(&self) -> Endpoints {
        let base = &self.base;
        Endpoints {
            authorize: format!("{base}/authorize"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            jwks: format!("{base}/jwks"),
            issuer: base.clone(),
            api: format!("{base}/v1"),
        }
    }
}

impl Drop for FakeOpenAi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

/// Ces tests décrivent le mode « Demander avant d'agir » : le Mode YOLO, activé par
/// défaut, lève les cartes d'approbation qu'ils vérifient. `tests/yolo.rs` couvre
/// le mode YOLO lui-même.
pub fn ask_before_acting(app: &mut grokbot_wrapper::App) {
    app.api("POST", "/api/yolo/mode", br#"{"mode":"ask"}"#)
        .expect("retour au mode Demander avant d'agir");
}

pub fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "délai dépassé : {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
