use super::support::ScriptedResponse;
use grokbot_wrapper::Endpoints;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

pub type ReplyFor = Arc<dyn Fn(&Value) -> ScriptedResponse + Send + Sync>;

#[derive(Default)]
pub struct Seen {
    pub bodies: Vec<Value>,
    pub response_gate: Option<(usize, Arc<AtomicBool>)>,
    pub instruction_gate: Option<(String, Arc<AtomicBool>)>,
    pub reply_for: Option<ReplyFor>,
}

pub struct FakeOpenAi {
    base: String,
    pub seen: Arc<Mutex<Seen>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeOpenAi {
    pub fn new() -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!(
            "http://127.0.0.1:{}",
            server.server_addr().to_ip().unwrap().port()
        );
        let seen = Arc::new(Mutex::new(Seen::default()));
        let observations = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut held = Vec::new();
            while !stopping.load(Ordering::SeqCst) {
                let Some(mut request) = server.recv_timeout(Duration::from_millis(20)).unwrap()
                else {
                    continue;
                };
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let (reply, gate) = match request.url() {
                    "/v1/models" => (
                        ScriptedResponse::json(
                            200,
                            json!({"models":[{"slug":"gpt-test","display_name":"GPT Test",
                                "visibility":"list","default_reasoning_level":"medium",
                                "supported_reasoning_levels":[{"effort":"medium","description":"Test"}]}]}),
                        ),
                        None,
                    ),
                    "/v1/responses" | "/backend-api/codex/responses" => {
                        let request_body: Value = serde_json::from_str(&body).unwrap();
                        let mut seen = observations.lock().unwrap();
                        seen.bodies.push(request_body.clone());
                        let reply = seen
                            .reply_for
                            .as_ref()
                            .map(|reply| reply(&request_body))
                            .unwrap_or_else(|| text("Réponse de contrôle."));
                        let gate = seen
                            .response_gate
                            .as_ref()
                            .filter(|(at, _)| *at == seen.bodies.len())
                            .map(|(_, gate)| gate.clone())
                            .or_else(|| {
                                seen.instruction_gate
                                    .as_ref()
                                    .filter(|(text, _)| {
                                        request_body["instructions"]
                                            .as_str()
                                            .is_some_and(|instructions| instructions.contains(text))
                                    })
                                    .map(|(_, gate)| gate.clone())
                            });
                        (reply, gate)
                    }
                    "/backend-api/wham/usage" => (
                        ScriptedResponse::json(200, json!({"plan_type":"plus"})),
                        None,
                    ),
                    _ => (ScriptedResponse::json(404, json!({})), None),
                };
                let response = tiny_http::Response::from_string(reply.body)
                    .with_status_code(reply.status)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", reply.content_type).unwrap(),
                    );
                if let Some(gate) = gate {
                    let stopping = stopping.clone();
                    held.push(std::thread::spawn(move || {
                        while !gate.load(Ordering::SeqCst) && !stopping.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        let _ = request.respond(response);
                    }));
                } else {
                    let _ = request.respond(response);
                }
            }
            for thread in held {
                thread.join().unwrap();
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
        Endpoints {
            authorize: format!("{}/authorize", self.base),
            token: format!("{}/token", self.base),
            revoke: format!("{}/revoke", self.base),
            jwks: format!("{}/jwks", self.base),
            issuer: self.base.clone(),
            api: format!("{}/v1", self.base),
        }
    }
}

impl Drop for FakeOpenAi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

pub fn text(body: &str) -> ScriptedResponse {
    ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":body}),
        json!({"type":"response.completed"}),
    ])
}
