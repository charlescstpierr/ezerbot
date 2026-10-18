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

#[derive(Default)]
pub struct Seen {
    pub nonce: String,
    pub forms: Vec<HashMap<String, String>>,
    pub bearer: Vec<String>,
    /// Distinct issued tokens let tests detect reuse of a stale bearer after refresh.
    pub access_tokens: Vec<String>,
    pub bodies: Vec<Value>,
    pub revoked: bool,
    /// FIFO replies for /v1/responses. An empty queue uses the original chat fixture.
    pub responses: VecDeque<ScriptedResponse>,
    /// Automatic memory runs independently; never consume a chat's scripted reply.
    pub memory_responses: VecDeque<ScriptedResponse>,
    pub memory_model: bool,
    pub memory_delay: Duration,
    /// Optional provider telemetry for default chat replies (titles remain independent).
    pub usage: Option<Value>,
    /// Hold a catalogue or a numbered response without locking observations.
    pub models_gate: Option<Arc<AtomicBool>>,
    pub model_requests: usize,
    pub response_gate: Option<(usize, Arc<AtomicBool>)>,
    /// Test-only delay before sending Responses headers (without holding Seen).
    pub response_delay: Option<Duration>,
}

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
                let response_delay = if request.url() == "/v1/responses" {
                    seen.response_delay
                } else {
                    None
                };
                let mut delay = Duration::ZERO;
                let (status, content_type, payload) = match request.url() {
                    "/jwks" => (200, "application/json".to_owned(), JWKS.to_owned()),
                    "/token" => {
                        let fields = form(&body);
                        let refresh = fields["grant_type"] == "refresh_token";
                        seen.forms.push(fields);
                        let sequence = seen.access_tokens.len() + 1;
                        let access = format!("access-{sequence}");
                        seen.access_tokens.push(access.clone());
                        let reply = json!({
                            "access_token": access,
                            "refresh_token": format!("refresh-{sequence}"),
                            "id_token": id_token(&issuer, &seen.nonce),
                            "token_type": "Bearer", "expires_in": if refresh { 3600 } else { 1 },
                            "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke",
                        });
                        (200, "application/json".into(), reply.to_string())
                    }
                    "/revoke" => {
                        seen.revoked = true;
                        (200, "application/json".into(), String::new())
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
                    "/v1/responses" => {
                        seen.bearer.push(bearer);
                        let request_body: Value = serde_json::from_str(&body).unwrap();
                        seen.bodies.push(request_body.clone());
                        if let Some(key) = request_body["prompt_cache_key"]
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
                    _ => (404, "text/plain".into(), String::new()),
                };
                let gate = match request.url() {
                    "/v1/models" => seen.models_gate.clone(),
                    "/v1/responses" => seen
                        .response_gate
                        .as_ref()
                        .filter(|(at, _)| *at == seen.bodies.len())
                        .map(|(_, gate)| gate.clone()),
                    _ => None,
                };
                drop(seen);
                if let Some(gate) = gate {
                    while !gate.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                if let Some(delay) = response_delay {
                    std::thread::sleep(delay);
                }
                std::thread::sleep(delay);
                let header = tiny_http::Header::from_bytes("Content-Type", content_type).unwrap();
                let _ = request.respond(
                    tiny_http::Response::from_string(payload)
                        .with_status_code(status)
                        .with_header(header),
                );
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

pub fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "délai dépassé : {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
