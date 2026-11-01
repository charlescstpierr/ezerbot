//! A keyring locked across a restart: the host holds no copy of the secrets, so nothing can be
//! masked. Turns and tool outputs stop at the masking boundary, reads keep working.
#![cfg(all(target_os = "linux", debug_assertions))]
mod support;
use grokbot_engine::{contracts::RunEventKind, model::RunStatus, store};
use grokbot_forms::{requests, vault::Vault};
use grokbot_linux_desktop::testing::Keyring;
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const SECRET: &str = "porcelaine-ambree-synthetique-9931";
const MESSAGE: &str = "Déverrouillez votre trousseau";

/// Stored and migrated while the keyring is open: from then on only the keyring knows it.
fn keep_only_in_the_keyring(keyring: &Keyring) {
    let file = keyring.home().path().join("workspace.secrets.json");
    Vault::open(&file)
        .unwrap()
        .set("PORCELAINE", "Piège synthétique", SECRET)
        .unwrap();
    Vault::open_test_keychain(&file, keyring.address()).unwrap();
    assert!(!std::fs::read_to_string(&file).unwrap().contains(SECRET));
}

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(std::env::var_os("HOME").unwrap()).join("workspace.sqlite");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        Self {
            app,
            provider,
            server,
            path,
        }
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn chat(&mut self) -> String {
        self.post("/api/conversations", json!({"title":"Trousseau"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn messages(&mut self, chat: &str) -> Vec<Value> {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn sent(&self) -> String {
        serde_json::to_string(&self.server.seen.lock().unwrap().bodies).unwrap()
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn run_of(&self, message: &str) -> grokbot_engine::model::Run {
        let record:String=self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[message],|r|r.get(0)).unwrap();
        serde_json::from_str(&record).unwrap()
    }
}

#[test]
fn a_turn_waits_for_the_keyring_and_never_leaves_unmasked() {
    let Some(phase) = support::keyring_phases(
        "a_turn_waits_for_the_keyring_and_never_leaves_unmasked",
        2,
        |phase, keyring| match phase {
            0 => {
                keep_only_in_the_keyring(keyring);
                keyring.lock();
            }
            _ => keyring.unlock(),
        },
    ) else {
        return;
    };
    let mut f = Fixture::new();
    if phase == 0 {
        let state = f.state();
        assert_eq!(state["forms"]["storage"]["backend"], "file_fallback");
        let chat = f.chat();
        f.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":"m","body":format!("Voici la clé : {SECRET}")}),
        );
        wait("refus affiché ou envoi", || {
            f.sent().contains("Voici la clé")
                || f.messages(&chat)
                    .iter()
                    .any(|m| m["id"] == "reply-m" && m["error"].as_str().is_some())
        });
        assert!(
            !f.sent().contains(SECRET),
            "the secret reached the provider"
        );
        let reply = f.messages(&chat).pop().unwrap();
        assert!(
            reply["error"].as_str().unwrap().contains(MESSAGE),
            "{reply}"
        );
        assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 0);
        std::fs::write(f.path.with_extension("chat"), chat).unwrap();
    } else {
        let chat = std::fs::read_to_string(f.path.with_extension("chat")).unwrap();
        assert_eq!(f.state()["forms"]["storage"]["backend"], "keychain");
        f.post(
            "/api/regenerate",
            json!({"conversation_id":chat,"message_id":"m"}),
        );
        wait("réponse", || {
            f.messages(&chat)
                .iter()
                .any(|m| m["id"] == "reply-m" && m["state"] == "echoed" && m["error"].is_null())
        });
        let sent = f.sent();
        assert!(sent.contains("Voici la clé"), "{sent}");
        assert!(!sent.contains(SECRET), "the secret reached the provider");
    }
}

#[test]
fn a_keyring_that_holds_nothing_of_this_vault_stops_no_turn() {
    let Some(phase) = support::keyring_phases(
        "a_keyring_that_holds_nothing_of_this_vault_stops_no_turn",
        2,
        |phase, keyring| match phase {
            0 => {
                let file = keyring.home().path().join("workspace.secrets.json");
                Vault::open(&file)
                    .unwrap()
                    .set("PORCELAINE", "Piège synthétique", SECRET)
                    .unwrap();
                keyring.lock();
            }
            _ => keyring.stop(),
        },
    ) else {
        return;
    };
    let mut f = Fixture::new();
    assert_eq!(f.state()["forms"]["storage"]["backend"], "file_fallback");
    let chat = f.chat();
    let id = format!("m{phase}");
    f.post(
        "/api/send",
        json!({"conversation_id":chat,"message_id":id,"body":format!("Voici la clé : {SECRET}")}),
    );
    wait("réponse", || {
        f.messages(&chat)
            .iter()
            .any(|m| m["id"] == format!("reply-{id}") && m["state"] == "echoed")
    });
    let sent = f.sent();
    assert!(sent.contains("Voici la clé"), "{sent}");
    assert!(!sent.contains(SECRET), "the file secret was not masked");
}

#[test]
fn a_tool_output_is_not_kept_or_sent_while_the_keyring_is_locked() {
    let Some(phase) = support::keyring_phases(
        "a_tool_output_is_not_kept_or_sent_while_the_keyring_is_locked",
        2,
        |phase, keyring| match phase {
            0 => keep_only_in_the_keyring(keyring),
            _ => keyring.lock(),
        },
    ) else {
        return;
    };
    let mut f = Fixture::new();
    let document = f.path.with_extension("document");
    if phase == 0 {
        support::ask_before_acting(&mut f.app);
        f.post(
            "/api/local/bot",
            json!({"bot_id":"grokbot","shell":false,"files":true}),
        );
        std::fs::write(&document, format!("clé : {SECRET}")).unwrap();
        let chat = f.chat();
        f.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":"fc-read","type":"function_call","status":"completed","call_id":"read","name":"read_file","arguments":json!({"path":document}).to_string()}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
        ]));
        f.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":"read","body":"Lis ce document"}),
        );
        wait("approbation", || {
            f.run_of("read").status == RunStatus::AwaitingApproval
                && f.provider.status()["titling"] == false
        });
    } else {
        let run = f.run_of("read");
        let request = requests::list_for_run(&f.db(), &run.run_id)
            .unwrap()
            .remove(0);
        let resolved = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":request.route.target_conversation_id,"decision":"allow_once"}).to_string().as_bytes(),
        );
        for _ in 0..300 {
            f.app.tick(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            );
            if f.run_of("read").status.is_terminal() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let kept = store::events(&f.db(), &run.run_id, 0)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == RunEventKind::ToolResult)
            .map(|e| e.payload.to_string())
            .collect::<String>();
        assert!(!kept.contains(SECRET), "stored unmasked: {kept}");
        assert!(
            !f.sent().contains(SECRET),
            "the secret reached the provider"
        );
        let state = f.state().to_string();
        assert!(
            resolved.is_err_and(|e| e.message.contains(MESSAGE)) || state.contains(MESSAGE),
            "{state}"
        );
    }
}
