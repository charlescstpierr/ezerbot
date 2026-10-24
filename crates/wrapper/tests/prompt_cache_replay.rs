//! Offline replay driver for tools/prompt_cache.py. The existing prefix tests stay untouched.
mod support;

use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, Seen, CLIENT};

struct ReplayDir(PathBuf);
impl ReplayDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-cache-replay-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for ReplayDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}

#[test]
fn five_turn_fake_replay_writes_private_report() {
    let dir = ReplayDir::new();
    let db = dir.0.join("app.sqlite");
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    std::fs::write(dir.0.join("app.chatgpt-auth.json"), json!({
        "host_id":"test-host", "client_id":CLIENT, "access_token":"fake-access",
        "refresh_token":"fake-refresh", "expires_at":expires, "scopes":"chatgpt.tokens.use.direct"
    }).to_string()).unwrap();
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen {
        usage: Some(json!({"input_tokens":12340,"output_tokens":100,
            "input_tokens_details":{"cached_tokens":11008}})),
        ..Seen::default()
    })));
    let provider = Provider::with_endpoints(&db, server.endpoints(), false);
    let mut app = App::open_with(&db, provider.clone()).unwrap();
    wait("fake models", || provider.status()["models_ready"] == true);
    let conn = Connection::open(&db).unwrap();
    let mut messages = Vec::new();
    // Two representative scenarios: ordinary followups and a memory change mid-thread.
    for scenario in 0..2 {
        let conversation = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        for turn in 0..5 {
            if scenario == 1 && turn == 2 {
                app.invoke_tool(
                    "grokbot",
                    "remember",
                    json!({"text":"PRIVATE-REPLAY-MEMORY prefers tea", "source":"user_stated"}),
                )
                .unwrap();
            }
            let id = format!("replay-{scenario}-{turn}");
            let message = format!("PRIVATE-REPLAY-MESSAGE-{scenario}-{turn} Bonjour, continue.");
            messages.push(message.clone());
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id":conversation, "message_id":id, "body":message}),
            );
            wait("completed fake turn and durable telemetry", || {
                let state = app.api("GET", "/api/state", b"").unwrap();
                let finished = state["conversations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|c| c["id"] == conversation)
                    .unwrap()["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["id"] == format!("reply-{id}") && m["state"] == "echoed");
                let count: usize = conn
                    .query_row("SELECT COUNT(*) FROM engine_response_usage", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                finished && count == scenario * 5 + turn + 1
            });
            wait("fake title worker drained", || {
                provider.status()["titling"] == false
            });
        }
    }
    let output = std::env::var_os("EZERBOT_PROMPT_CACHE_REPLAY_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.0.join("results.json"));
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/prompt_cache.py");
    let result = Command::new("python3")
        .arg(script)
        .args(["report", "--db"])
        .arg(&db)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = std::fs::read_to_string(output).unwrap();
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["synthetic"], true);
    assert_eq!(report["compaction_count"], 0);
    assert_eq!(report["unmeasured_runs"], 0);
    assert_eq!(report["global"]["excluded_first_responses"], 2);
    assert_eq!(report["global"]["measured"]["responses"], 8);
    assert_eq!(report["global"]["measured"]["input_tokens"], 8 * 12340);
    assert_eq!(
        report["global"]["measured"]["cached_input_tokens"],
        8 * 11008
    );
    let conversations = report["conversations"].as_array().unwrap();
    assert_eq!(conversations.len(), 2);
    for conversation in conversations {
        let turns = conversation["turns"].as_array().unwrap();
        assert_eq!(turns.len(), 5);
        assert_eq!(conversation["measured"]["responses"], 4);
        for (index, turn) in turns.iter().enumerate() {
            assert_eq!(turn["turn"], index + 1);
            assert_eq!(turn["responses"][0]["excluded_first"], index == 0);
            assert_eq!(turn["responses"][0]["rate_percent"], 89.2);
            assert_eq!(turn["cacheable"]["responses"], usize::from(index != 0));
        }
    }
    for message in messages {
        assert!(!text.contains(&message));
    }
    for private in [
        "PRIVATE-REPLAY",
        "Bonjour",
        "fake-access",
        "fake-refresh",
        "test-host",
    ] {
        assert!(!text.contains(private));
    }
}

#[test]
fn report_arithmetic_and_privacy_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = Command::new("python3")
        .current_dir(root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "tools",
            "-p",
            "test_prompt_cache.py",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
