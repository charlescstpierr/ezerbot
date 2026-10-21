//! Quota diagnostics and the durable UI path, only against the fake OpenAI.
mod support;

use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};

#[derive(Debug)]
struct TestFailure {
    message: String,
    kind: &'static str,
}

struct Fixture {
    directory: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "ezerbot-quota-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen {
            request_id: Some("req_quota_test".into()),
            ..Seen::default()
        })));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(directory.join("workspace.chatgpt-auth.json"), json!({
            "host_id":"test-host", "client_id":"oaiapp_test", "email":"quota@example.test",
            "access_token":"private-access", "refresh_token":"private-refresh", "expires_at":now+3600,
            "scopes":"chatgpt.tokens.use.direct", "welcomed":true
        }).to_string()).unwrap();
        let provider = Provider::with_endpoints(
            &directory.join("workspace.sqlite"),
            server.endpoints(),
            false,
        );
        Self {
            directory,
            server,
            provider,
        }
    }
    fn reply(&self, response: ScriptedResponse) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(response);
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.directory.join("logs/engine.log")).unwrap_or_default()
    }
    fn raw(&self) -> Result<ureq::Response, TestFailure> {
        self.provider
            .responses(json!({"model":"gpt-test", "stream":true,"input":[]}))
            .map_err(|error| TestFailure {
                message: error.message,
                kind: error.kind,
            })
    }
    fn app(&self) -> App {
        App::open_with(
            &self.directory.join("workspace.sqlite"),
            self.provider.clone(),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}
fn state(app: &mut App) -> Value {
    app.api("GET", "/api/state", b"").unwrap()
}
fn reply(app: &mut App) -> Value {
    state(app)["conversations"][0]["messages"]
        .as_array()
        .and_then(|messages| messages.iter().find(|m| m["role"] == "assistant"))
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn each_http_mapping_logs_only_metadata_once_without_retrying_limits() {
    for (status, code, expected_kind) in [
        (429, "subscription_sharing_usage_limit_exceeded", "limit"),
        (403, "subscription_sharing_user_not_eligible", "other"),
        (503, "subscription_sharing_usage_unavailable", "other"),
        (503, "subscription_sharing_user_unavailable", "other"),
        (403, "subscription_sharing_invalid_user", "signin"),
        (429, "usage_limit_reached", "limit"),
        (403, "usage_not_included", "other"),
        (429, "rate_limit_exceeded", "other"),
        (429, "insufficient_quota", "other"),
        (400, "subscription_sharing_unsupported_capability", "other"),
        (503, "unknown", "other"),
    ] {
        let fixture = Fixture::new();
        fixture.reply(ScriptedResponse::json(status,json!({"error":{"code":code,"message":"private prompt quota@example.test private-access","param":"prompt_cache_key"}})));
        let error = fixture.raw().unwrap_err();
        assert_eq!(error.kind, expected_kind, "{code}");
        assert_eq!(fixture.server.seen.lock().unwrap().bodies.len(), 1);
        let log = fixture.log();
        assert_eq!(log.lines().count(), 1);
        assert!(log.ends_with(&format!("OpenAI refus : statut={status} code={code} param=prompt_cache_key request_id=req_quota_test\n")),"{log}");
        assert!(!log.contains("private") && !log.contains("@"));
        if expected_kind == "limit" {
            if code == "subscription_sharing_usage_limit_exceeded" {
                assert!(error.message.contains("Ezerbot a atteint la limite d’utilisation que lui accorde votre abonnement ChatGPT"));
                assert!(error.message.contains("quota Codex peut rester disponible"));
                assert!(error.message.contains("ChatGPT → Réglages → Utilisation"));
            } else {
                assert!(error.message.contains("limite d’utilisation Codex"));
                assert!(error.message.contains("Codex → Réglages → Utilisation"));
                assert!(!error.message.contains("abonnement"));
            }
            assert!(!error.message.contains("Réinitialisation"));
        }
    }
}

#[test]
fn successful_calls_do_not_log_and_bad_metadata_cannot_inject_lines() {
    let fixture = Fixture::new();
    fixture.reply(ScriptedResponse::json(200, json!({"id":"ok"})));
    fixture.raw().unwrap();
    assert!(fixture.log().is_empty());
    fixture.server.seen.lock().unwrap().request_id = Some("req_bad@example.test".into());
    fixture.reply(ScriptedResponse::json(400,json!({"error":{"code":"bad\nprivate-access","param":"quota@example.test","message":"private-refresh"}})));
    fixture.raw().unwrap_err();
    assert!(fixture
        .log()
        .ends_with("statut=400 code=- param=- request_id=-\n"));
    assert_eq!(fixture.log().lines().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(fixture.directory.join("logs/engine.log"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

fn local_reset_time() -> String {
    rusqlite::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT strftime('%d/%m à %H h %M', '2026-10-20T18:00:00Z', 'localtime')",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn http_reset_is_explicit_validated_and_limited_to_the_usage_card() {
    let fixture = Fixture::new();
    for reset in [
        json!(1792519200),
        json!("1792519200"),
        json!("2026-10-20T18:00:00Z"),
        json!("2026-10-20T14:00:00-04:00"),
    ] {
        fixture.reply(ScriptedResponse::json(
            429,
            json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","reset_at":reset}}),
        ));
        let message = fixture.raw().unwrap_err().message;
        assert!(
            message.ends_with(&format!(
                "Réinitialisation prévue par OpenAI : le {}.",
                local_reset_time()
            )),
            "{message}"
        );
        assert!(!message.contains("UTC") && !message.contains(":00:00"));
    }
    for reset in [
        Value::Null,
        json!(-1),
        json!("not a date private-access"),
        json!("now"),
        json!({"secret":"private-access"}),
    ] {
        fixture.reply(ScriptedResponse::json(
            429,
            json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","resets_at":reset}}),
        ));
        assert!(!fixture
            .raw()
            .unwrap_err()
            .message
            .contains("Réinitialisation"));
    }
    fixture.reply(ScriptedResponse::json(
        429,
        json!({"error":{"code":"rate_limit_exceeded","reset_at":1792519200}}),
    ));
    assert!(!fixture
        .raw()
        .unwrap_err()
        .message
        .contains("Réinitialisation"));
    assert!(!fixture.log().contains("UTC"));
}

#[test]
fn streamed_mappings_keep_http_request_id_and_durable_limit_action_after_restart() {
    for shape in ["response.failed", "error"] {
        for (code, kind) in [
            ("subscription_sharing_usage_limit_exceeded", "limit"),
            ("subscription_sharing_user_not_eligible", "other"),
            ("subscription_sharing_usage_unavailable", "other"),
            ("subscription_sharing_user_unavailable", "other"),
            ("subscription_sharing_invalid_user", "signin"),
            ("rate_limit_exceeded", "other"),
            ("usage_limit_reached", "limit"),
            ("usage_not_included", "other"),
            ("insufficient_quota", "other"),
            ("subscription_sharing_unsupported_capability", "other"),
        ] {
            let fixture = Fixture::new();
            let error = json!({"code":code,"param":"input","message":"private-access","reset_at":"2026-10-20T18:00:00Z"});
            // The flat error event is a real Responses SSE shape.
            let event = if shape == "response.failed" {
                json!({"type":shape,"response":{"error":error}})
            } else {
                json!({"type":shape,"code":code,"param":"input","reset_at":"2026-10-20T18:00:00Z"})
            };
            fixture.reply(ScriptedResponse::events(&[event]));
            let mut app = fixture.app();
            wait("catalogue", || {
                fixture.provider.status()["models_ready"] == true
            });
            let id = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
                .as_str()
                .unwrap()
                .to_owned();
            post(
                &mut app,
                "/api/send",
                json!({"conversation_id":id,"message_id":"m1","body":"prompt confidentiel"}),
            );
            wait("erreur durable", || durable_limit(&reply(&mut app), kind));
            let error = reply(&mut app);
            assert_eq!(
                fixture
                    .log()
                    .lines()
                    .filter(|line| line.contains("OpenAI refus"))
                    .count(),
                1
            );
            assert!(fixture.log().contains(&format!(
                "statut=200 code={code} param=input request_id=req_quota_test\n"
            )));
            assert!(!fixture.log().contains("private") && !fixture.log().contains("confidentiel"));
            if kind == "limit" {
                assert!(error["error"]
                    .as_str()
                    .unwrap()
                    .contains(&local_reset_time()));
                // The SSE classification must not depend on the old French title.
                assert!(!error["error"]
                    .as_str()
                    .unwrap()
                    .contains("Limite d’utilisation"));
            }
            drop(app);
            let restarted = Provider::with_endpoints(
                &fixture.directory.join("workspace.sqlite"),
                fixture.server.endpoints(),
                false,
            );
            let mut app =
                App::open_with(&fixture.directory.join("workspace.sqlite"), restarted).unwrap();
            assert_eq!(reply(&mut app)["error"], error["error"]);
            assert_eq!(reply(&mut app)["error_kind"], kind);
            let provider = state(&mut app)["provider"].clone();
            assert_eq!(provider["email"], "quota@example.test");
            assert!(!provider.to_string().contains("private-access"));
        }
    }
}

#[test]
fn each_unauthorized_attempt_is_logged_including_a_recovered_failure() {
    let fixture = Fixture::new();
    fixture.reply(ScriptedResponse::json(
        401,
        json!({"error":{"code":"expired"}}),
    ));
    fixture.reply(ScriptedResponse::json(200, json!({"id":"ok"})));
    fixture.raw().unwrap();
    assert_eq!(fixture.log().lines().count(), 1);
    for _ in 0..2 {
        fixture.reply(ScriptedResponse::json(
            401,
            json!({"error":{"code":"expired"}}),
        ));
    }
    assert_eq!(fixture.raw().unwrap_err().kind, "signin");
    assert_eq!(fixture.log().lines().count(), 3);
    assert!(fixture
        .log()
        .lines()
        .all(|line| line.ends_with("statut=401 code=expired param=- request_id=req_quota_test")));
}

#[test]
#[cfg(unix)]
fn native_stderr_log_handle_does_not_duplicate_or_overwrite_diagnostics() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let fixture = Fixture::new();
    fixture.reply(ScriptedResponse::json(
        429,
        json!({"error":{"code":"subscription_sharing_usage_limit_exceeded", "reset_at":"2026-10-20T18:00:00Z"}}),
    ));
    std::fs::create_dir(fixture.directory.join("logs")).unwrap();
    let mut log = std::fs::File::create(fixture.directory.join("logs/engine.log")).unwrap();
    writeln!(log, "native startup").unwrap();
    let mut child = Child(
        Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--port", "0", "--db"])
            .arg(fixture.directory.join("workspace.sqlite"))
            .env("HOME", &fixture.directory)
            .env("CFFIXED_USER_HOME", &fixture.directory)
            .env("TZ", "America/Toronto")
            .env("GROKBOT_FAKE_OPENAI", &fixture.server.base)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(log.try_clone().unwrap())
            .spawn()
            .unwrap(),
    );
    let mut first = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let first: Value = serde_json::from_str(&first).unwrap();
    let base = first["url"].as_str().unwrap();
    let post = |path: &str, body: Value| -> Value {
        ureq::post(&format!("{base}{path}"))
            .set("X-Grokbot-Request", "local")
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    };
    wait("child catalogue", || {
        let state: Value = ureq::get(&format!("{base}/api/state"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        state["provider"]["models_ready"] == true
    });
    let conversation = post("/api/conversations", json!({}))["conversation"]["id"].clone();
    post(
        "/api/send",
        json!({"conversation_id":conversation, "message_id":"m1", "body":"private prompt"}),
    );
    wait("native-style diagnostic", || {
        fixture.log().contains("OpenAI refus")
    });
    wait("local reset card", || {
        let state: Value = ureq::get(&format!("{base}/api/state"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        state["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["error"].as_str().is_some_and(|text| {
                    text.ends_with("Réinitialisation prévue par OpenAI : le 20/10 à 14 h 00.")
                })
            })
    });
    // Same open file description as the native shell. A direct O_APPEND write
    // from another handle would leave this offset stale and overwrite the log.
    writeln!(log, "native shutdown").unwrap();
    let contents = fixture.log();
    assert_eq!(
        contents
            .lines()
            .filter(|line| !line.contains("upstream turn="))
            .count(),
        3
    );
    assert!(contents.contains("code=started") && contents.contains("code=failed"));
    assert!(contents.starts_with("native startup\n"));
    assert!(contents.contains("statut=429 code=subscription_sharing_usage_limit_exceeded param=- request_id=req_quota_test\n"));
    assert!(contents.ends_with("native shutdown\n"));
}

#[test]
fn http_limit_card_and_account_survive_restart_then_signout_hides_identity() {
    let fixture = Fixture::new();
    fixture.reply(ScriptedResponse::json(429,json!({"error":{"code":"subscription_sharing_usage_limit_exceeded","resets_at":"2026-10-20T18:00:00Z"}})));
    let mut app = fixture.app();
    wait("catalogue", || {
        fixture.provider.status()["models_ready"] == true
    });
    let id = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id":id,"message_id":"m1","body":"Bonjour"}),
    );
    wait("limite", || durable_limit(&reply(&mut app), "limit"));
    let error = reply(&mut app)["error"].clone();
    drop(app);
    let mut app = fixture.app();
    assert_eq!(reply(&mut app)["error"], error);
    assert_eq!(reply(&mut app)["error_kind"], "limit");
    assert_eq!(state(&mut app)["provider"]["email"], "quota@example.test");
    post(&mut app, "/api/provider/signout", json!({}));
    assert!(state(&mut app)["provider"]["email"].is_null());
}

fn durable_limit(reply: &Value, kind: &str) -> bool {
    reply["state"] == "failed" && reply["error_kind"] == kind
}
