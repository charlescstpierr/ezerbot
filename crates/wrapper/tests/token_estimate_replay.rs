//! Offline replay driver for tools/token_estimate.py: the fake server answers with a counted
//! `input_tokens`, the report compares it with what the compaction trigger measured.
mod support;

use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const SECRET: &str = "PRIVATE-ESTIMATE-VAULT-sk-0123456789abcdef";
const FRENCH: &str = "Le vent léger traverse les champs de lavande près d'Aix-en-Provence et les enfants rentrent du marché avec des paniers pleins de fruits mûrs. ";

struct ReplayDir(grokbot_test_support::TestDir);
impl ReplayDir {
    fn new() -> Self {
        Self(grokbot_test_support::TestDir::new(
            "ezerbot-estimate-replay",
        ))
    }
}

type Row = (i64, i64, Option<i64>, Option<i64>, Option<i64>);

struct Counter {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Counter {
    fn spawn(command: &str) -> Self {
        let mut words = command.split_whitespace();
        let mut child = Command::new(words.next().expect("EZERBOT_TOKEN_COUNTER is empty"))
            .args(words)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("token counter starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn count(&mut self, body: &Value) -> u64 {
        writeln!(self.stdin, "{body}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        line.trim().parse().expect("one integer per request")
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}

fn is_main_request(body: &Value) -> bool {
    let instructions = body["instructions"].as_str().unwrap_or_default();
    body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty())
        && !instructions.starts_with("Extraction de mémoire automatique.")
        && !instructions.starts_with("Summarize the supplied conversation evidence")
        && !instructions.contains("Tu écris le titre")
}

fn long_message(conversation: usize, turn: usize) -> String {
    let marker = format!("PRIVATE-ESTIMATE-LONG-{conversation}-{turn} ");
    let mut text = marker;
    while text.chars().count() < 12_000 {
        text.push_str(FRENCH);
    }
    text
}

#[test]
fn varied_conversations_report_the_trigger_estimate_against_counted_tokens() {
    let dir = ReplayDir::new();
    let db = dir.0.join("app.sqlite");
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    std::fs::write(
        dir.0.join("app.chatgpt-auth.json"),
        json!({
            "host_id":"test-host", "client_id":CLIENT, "access_token":"fake-access",
            "refresh_token":"fake-refresh", "expires_at":expires, "scopes":"chatgpt.tokens.use.direct"
        })
        .to_string(),
    )
    .unwrap();
    grokbot_forms::vault::Vault::open(&db.with_extension("secrets.json"))
        .unwrap()
        .set("TOKEN", "test", SECRET)
        .unwrap();

    let counter = std::env::var("EZERBOT_TOKEN_COUNTER")
        .ok()
        .map(|command| Mutex::new(Counter::spawn(&command)));
    let returned = Arc::new(Mutex::new(Vec::<u64>::new()));
    let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
    {
        let returned = returned.clone();
        let counter = counter.map(Arc::new);
        server.seen.lock().unwrap().reply_for = Some(Arc::new(move |body: &Value| {
            if !is_main_request(body) {
                return None;
            }
            // Without a counter the test owns a fixed formula, so every assertion is exact.
            let input_tokens = match &counter {
                Some(counter) => counter.lock().unwrap().count(body),
                None => body.to_string().chars().count() as u64 / 5 + 17,
            };
            returned.lock().unwrap().push(input_tokens);
            Some(ScriptedResponse::events(&[
                json!({"type":"response.created"}),
                json!({"type":"response.output_text.delta","delta":"Bien reçu, je continue."}),
                json!({"type":"response.completed","response":{"usage":{
                    "input_tokens":input_tokens,"output_tokens":20,
                    "input_tokens_details":{"cached_tokens":0}}}}),
            ]))
        }));
    }
    let provider = Provider::with_endpoints(&db, server.endpoints(), false);
    let mut app = App::open_with(&db, provider.clone()).unwrap();
    wait("fake models", || provider.status()["models_ready"] == true);
    let conn = Connection::open(&db).unwrap();

    const LONG: usize = 4;
    const GROUP: usize = 5;
    let turns = [1usize, 2, 4, 6, 8, 1];
    let mut sent = Vec::new();
    let mut conversation_ids = Vec::new();
    let mut answered = 0usize;
    for (index, count) in turns.iter().enumerate() {
        let long = index == LONG;
        let group = index == GROUP;
        if long {
            post(
                &mut app,
                "/api/engine/config",
                json!({"estimated_tokens":12000}),
            );
        }
        let conversation = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        conversation_ids.push(conversation.clone());
        if group {
            post(
                &mut app,
                "/api/bots",
                json!({"id":"pair","name":"Pair","color":"cyan"}),
            );
            post(
                &mut app,
                "/api/conversations/members/add",
                json!({"conversation_id":conversation,"bot_id":"pair","command_id":"membre"}),
            );
        }
        for turn in 0..*count {
            let id = format!("estimate-{index}-{turn}");
            let message = if long {
                long_message(index, turn)
            } else {
                format!(
                    "PRIVATE-ESTIMATE-SHORT-{index}-{turn} Bonjour, peux-tu continuer ? {FRENCH}"
                )
            };
            sent.push(message.clone());
            let mentions = if group {
                json!([{"id":"pair","label":"Pair"}])
            } else {
                json!([])
            };
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id":conversation, "message_id":id, "body":message,
                    "mentions":mentions}),
            );
            answered += 1;
            wait("completed fake turn and durable telemetry", || {
                if group {
                    app.tick(0);
                }
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
                let finished = finished
                    || group
                        && conn
                            .query_row(
                                "SELECT COUNT(*) FROM wrapper_group_turns
                                WHERE conversation_id=?1 AND status='completed'",
                                [&conversation],
                                |r| r.get::<_, i64>(0),
                            )
                            .unwrap()
                            > 0;
                let rows: usize = conn
                    .query_row("SELECT COUNT(*) FROM engine_response_usage", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                finished && rows == answered
            });
            wait("fake title worker drained", || {
                provider.status()["titling"] == false
            });
        }
    }
    let total: usize = turns.iter().sum();
    assert_eq!(answered, total);
    let returned = returned.lock().unwrap().clone();
    assert_eq!(
        returned.len(),
        total,
        "one counted request per main response"
    );

    let rows: Vec<Row> = conn
        .prepare(
            "SELECT r.rowid, u.input_tokens, u.context_characters, u.characters_per_token,
                u.tool_characters
            FROM engine_response_usage u JOIN engine_runs r ON r.run_id = u.run_id
            ORDER BY r.rowid, u.response",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), total);
    assert_eq!(
        rows.iter().map(|r| r.1 as u64).collect::<Vec<_>>(),
        returned
    );
    for (_, _, characters, per_token, tools) in &rows {
        assert!(
            characters.is_some_and(|c| c > 0),
            "context_characters recorded"
        );
        assert_eq!(*per_token, Some(4));
        assert!(
            tools.is_some_and(|t| t > 0),
            "tool definitions are sent and measured"
        );
    }
    for column in [
        "context_characters",
        "characters_per_token",
        "tool_characters",
    ] {
        let types: Vec<String> = conn
            .prepare(&format!(
                "SELECT DISTINCT typeof({column}) FROM engine_response_usage"
            ))
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(types, ["integer"], "{column}");
    }

    let long_conversation = &conversation_ids[LONG];
    let summaries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_summaries WHERE conversation_id=?1 AND record IS NOT NULL",
            [long_conversation],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        summaries > 0,
        "the minimum budget compacts the long conversation"
    );
    let long_characters: Vec<i64> = rows[turns[..LONG].iter().sum::<usize>()..][..turns[LONG]]
        .iter()
        .map(|r| r.2.unwrap())
        .collect();
    let managed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM engine_response_usage u
            JOIN wrapper_group_turns g ON g.run_id = u.run_id
            WHERE u.context_characters IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        managed, 1,
        "a managed group send is estimated although it never compacts"
    );
    let threshold = 12_000 * 4 * 70 / 100;
    assert!(
        long_characters
            .iter()
            .all(|characters| *characters <= threshold),
        "sends after compaction stay under the trigger threshold: {long_characters:?}"
    );
    assert!(
        (long_characters[long_characters.len() - 1] as usize) < 8 * 12_000,
        "the last send carries a summary, not the eight raw messages"
    );

    let output = std::env::var_os("EZERBOT_TOKEN_ESTIMATE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.0.join("results.json"));
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/token_estimate.py");
    let result = Command::new("python3")
        .arg(script)
        .args(["report", "--synthetic", "--db"])
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
    assert_eq!(report["tool_definitions_counted"], false);
    assert_eq!(report["unmeasured_responses"], 0);
    assert_eq!(report["global"]["signed_error"]["n"], total);
    assert_eq!(report["global"]["signed_error_with_tools"]["n"], total);
    let conversations = report["conversations"].as_array().unwrap();
    assert_eq!(conversations.len(), turns.len());
    for (conversation, expected) in conversations.iter().zip(turns) {
        assert_eq!(conversation["signed_error"]["n"], expected);
        assert_eq!(
            conversation["responses"].as_array().unwrap().len(),
            expected
        );
    }
    assert!(conversations[LONG]["compaction_count"].as_u64().unwrap() > 0);
    let reported: Vec<&Value> = conversations
        .iter()
        .flat_map(|c| c["responses"].as_array().unwrap())
        .collect();
    for (reported, (_, input_tokens, characters, per_token, tools)) in reported.iter().zip(&rows) {
        let estimated = characters.unwrap() as f64 / per_token.unwrap() as f64;
        let with_tools = (characters.unwrap() + tools.unwrap()) as f64 / per_token.unwrap() as f64;
        let actual = *input_tokens as f64;
        let near = |value: &Value, expected: f64| (value.as_f64().unwrap() - expected).abs() < 1e-5;
        assert_eq!(reported["input_tokens"], *input_tokens);
        assert!(near(
            &reported["signed_error"],
            (estimated - actual) / actual
        ));
        assert!(near(
            &reported["signed_error_with_tools"],
            (with_tools - actual) / actual
        ));
    }

    for message in &sent {
        assert!(!text.contains(message.as_str()));
    }
    for conversation in &conversation_ids {
        assert!(!text.contains(conversation.as_str()));
    }
    for private in [
        "PRIVATE-ESTIMATE",
        SECRET,
        "sk-0123456789",
        "lavande",
        "Bonjour",
        "fake-access",
        "fake-refresh",
        "test-host",
    ] {
        assert!(!text.contains(private), "{private}");
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
            "test_token_estimate.py",
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
