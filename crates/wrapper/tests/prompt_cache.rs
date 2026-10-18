//! Full host/Responses boundary: stable conversation prefix and durable provider telemetry.
mod support;

use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, Seen, CLIENT};

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    dir: PathBuf,
    conversation: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("app.sqlite");
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        std::fs::write(dir.join("app.chatgpt-auth.json"), json!({
            "host_id":"test-host", "client_id":CLIENT, "access_token":"fake-access",
            "refresh_token":"fake-refresh", "expires_at":expires, "scopes":"chatgpt.tokens.use.direct"
        }).to_string()).unwrap();
        let seen = Seen {
            usage: Some(json!({"input_tokens":12340,"output_tokens":100,
            "input_tokens_details":{"cached_tokens":11008}})),
            ..Seen::default()
        };
        let server = FakeOpenAi::new(Arc::new(Mutex::new(seen)));
        let provider = Provider::with_endpoints(&db, server.endpoints(), false);
        let mut app = App::open_with(&db, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        let conversation = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        // Deliberately reverse insertion order: index and tool ordering must be deterministic.
        for name in ["zebra", "alpha"] {
            post(
                &mut app,
                "/api/skills",
                json!({"bot_id":"grokbot", "name":name, "description":"Un skill", "body":"Un corps"}),
            );
        }
        Self {
            app,
            provider,
            server,
            dir,
            conversation,
        }
    }
    fn remember(&self, text: &str) {
        self.app
            .invoke_tool(
                "grokbot",
                "remember",
                json!({"text":text,"source":"user_stated"}),
            )
            .unwrap();
    }
    fn send(&mut self, id: &str, text: &str) -> Value {
        post(
            &mut self.app,
            "/api/send",
            json!({"conversation_id":self.conversation,"message_id":id,"body":text}),
        );
        wait("completed reply and usage", || {
            let state = self.app.api("GET", "/api/state", b"").unwrap();
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == self.conversation)
                .unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| {
                    m["id"] == format!("reply-{id}")
                        && m["state"] == "echoed"
                        && m["payload"]["usage"]["cached_input_tokens"] == 11008
                })
        });
        wait("title worker drained", || {
            self.provider.status()["titling"] == false
        });
        self.server
            .seen
            .lock()
            .unwrap()
            .bodies
            .iter()
            .rev()
            .find(|b| b.get("prompt_cache_key").is_some())
            .unwrap()
            .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}
fn without_memory(instructions: &Value) -> String {
    let text = instructions.as_str().unwrap();
    let (prefix, rest) = text
        .split_once("<external_data source=\"memory\">")
        .expect("memory block");
    let (_, suffix) = rest.split_once("</external_data>").unwrap();
    format!("{prefix}<memory>{suffix}")
}
fn assert_prefix(first: &Value, second: &Value) {
    assert_eq!(first["prompt_cache_key"], second["prompt_cache_key"]);
    assert_eq!(first["store"], false);
    assert_eq!(second["store"], false);
    assert_eq!(bytes(&first["tools"]), bytes(&second["tools"]));
    let prior = first["input"].as_array().unwrap();
    let next = second["input"].as_array().unwrap();
    assert!(next.len() > prior.len());
    assert_eq!(bytes(&json!(prior)), bytes(&json!(&next[..prior.len()])));
}

#[test]
fn consecutive_turns_keep_serialized_prefix_and_usage_after_restart() {
    let mut f = Fixture::new();
    f.remember("Je préfère le thé.");
    f.remember("Je parle français.");
    let first = f.send("one", "Bonjour");
    // Retitling, new run/message ids and wall-clock changes must not enter the prompt prefix.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let second = f.send("two", "Et ensuite ?");
    assert_prefix(&first, &second);
    assert_eq!(
        bytes(&first["instructions"]),
        bytes(&second["instructions"])
    );
    let key = second["prompt_cache_key"].as_str().unwrap();
    assert!(key.starts_with("ezerbot-") && key.len() == 72);
    assert!(!key.contains("test-host") && !key.contains("fake-access"));

    let conn = Connection::open(f.dir.join("app.sqlite")).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM engine_response_usage", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let provider = Provider::with_endpoints(&f.dir.join("app.sqlite"), f.server.endpoints(), false);
    let mut reopened = App::open_with(&f.dir.join("app.sqlite"), provider.clone()).unwrap();
    let state = reopened.api("GET", "/api/state", b"").unwrap();
    let messages = state["conversations"][0]["messages"].as_array().unwrap();
    let replies = messages
        .iter()
        .filter(|m| m["role"] == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 2);
    for reply in replies {
        assert_eq!(
            reply["payload"]["usage"],
            json!({"input_tokens":12340,"output_tokens":100,"cached_input_tokens":11008})
        );
    }
    wait("reopened models", || {
        provider.status()["models_ready"] == true
    });
    f.app = reopened;
    f.provider = provider;
    let third = f.send("three", "Toujours là ?");
    assert_eq!(second["prompt_cache_key"], third["prompt_cache_key"]);
    let other = post(&mut f.app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.conversation = other;
    let other = f.send("other", "Bonjour ailleurs");
    assert_ne!(third["prompt_cache_key"], other["prompt_cache_key"]);
}

#[test]
fn memory_change_only_rewrites_its_instruction_block() {
    let mut f = Fixture::new();
    f.remember("Je préfère le thé.");
    let first = f.send("one", "Bonjour");
    f.remember("Je préfère aussi les pommes.");
    let second = f.send("two", "Continue");
    assert_prefix(&first, &second);
    assert_ne!(first["instructions"], second["instructions"]);
    assert_eq!(
        without_memory(&first["instructions"]),
        without_memory(&second["instructions"])
    );
    assert!(!first["instructions"].as_str().unwrap().contains("pommes"));
    assert!(second["instructions"].as_str().unwrap().contains("pommes"));
}
