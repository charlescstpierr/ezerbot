//! Real host termination: watchdog cleanup must work when Rust Drop cannot run.
#[path = "support/launch_identity.rs"]
mod launch_identity;
mod support;
use launch_identity::LaunchIdentity;
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};

struct Host {
    child: Child,
    identity: LaunchIdentity,
    root: PathBuf,
    url: String,
    server: FakeOpenAi,
    _dir: grokbot_test_support::TestDir,
}
impl Host {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-local-shutdown");
        let root = guard.to_path_buf();
        support::write_codex_session(&root.join("workspace.codex-auth.json"), "local-shutdown");
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let (child, url, identity) = Self::launch(&root, &server);
        let host = Self {
            child,
            identity,
            root,
            url,
            server,
            _dir: guard,
        };
        host.ready();
        host.post("/api/yolo/mode", json!({"mode": "ask"}));
        host
    }
    fn launch(root: &std::path::Path, server: &FakeOpenAi) -> (Child, String, LaunchIdentity) {
        let identity = LaunchIdentity::new();
        let mut child = identity.spawn(
            Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
                .args(["--port", "0", "--db"])
                .arg(root.join("workspace.sqlite"))
                .current_dir(root)
                .env("HOME", root)
                .env("CFFIXED_USER_HOME", root)
                .env("GROKBOT_FAKE_OPENAI", &server.base)
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        );
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        (child, value["url"].as_str().unwrap().into(), identity)
    }
    fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .set("X-Ezerbot-Launch", &self.identity.value())
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn ready(&self) {
        wait("host models", || {
            self.state()["provider"]["models_ready"] == true
        });
    }
    fn post(&self, path: &str, body: Value) -> Value {
        ureq::post(&format!("{}{path}", self.url))
            .set("Origin", &self.url)
            .set("X-Grokbot-Request", "local")
            .set("X-Ezerbot-Launch", &self.identity.value())
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn start_job(&self) {
        self.post(
            "/api/local/bot",
            json!({"bot_id":"grokbot","shell":true,"files":false}),
        );
        let chat = self.post(
            "/api/conversations",
            json!({"title":"Arrêt du processus hôte"}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let args = json!({"command":"touch started; (sleep 1; touch orphan-sentinel) & wait","cwd":self.root,"timeout_ms":5000,"output_limit":4096});
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":"fc-shutdown","type":"function_call","status":"completed","call_id":"shutdown","name":"run_shell","arguments":args.to_string()}}),json!({"type":"response.completed"})]));
        self.post("/api/send",json!({"conversation_id":chat,"message_id":"shutdown-message","body":"Démarrer la commande de test jetable"}));
        let mut request = Value::Null;
        wait("approval", || {
            request = self.state()["forms"]["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["status"]["status"] == "pending")
                .cloned()
                .unwrap_or(Value::Null);
            !request.is_null()
        });
        self.post("/api/approvals/resolve",json!({"entry_id":request["entry_id"],"submission_id":"allow","conversation_id":request["route"]["target_conversation_id"],"decision":"allow_once"}));
        wait("command started", || {
            self.state();
            self.root.join("started").exists()
        });
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn host_sigterm_sigint_and_sigkill_leave_no_ordinary_process_group_running() {
    for signal in ["-TERM", "-INT", "-KILL"] {
        let mut host = Host::new();
        host.start_job();
        assert!(Command::new("/bin/kill")
            .args([signal, &host.child.id().to_string()])
            .current_dir(&host.root)
            .status()
            .unwrap()
            .success());
        host.child.wait().unwrap();
        std::thread::sleep(Duration::from_millis(1300));
        assert!(
            !host.root.join("orphan-sentinel").exists(),
            "descendant survived {signal}"
        );
        if signal == "-TERM" {
            let (child, url, identity) = Host::launch(&host.root, &host.server);
            host.child = child;
            host.url = url;
            host.identity = identity;
            host.ready();
            wait("unknown result after restart", || {
                host.state()["conversations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|c| c["messages"].as_array().unwrap())
                    .any(|m| {
                        m["payload"]["outcome"]["reason"]
                            .as_str()
                            .is_some_and(|s| s.starts_with("Résultat inconnu"))
                    })
            });
        }
    }
}
