//! A keyring locked across a restart: the host holds no copy of the secrets, so nothing can be
//! masked. Turns and tool outputs stop at the masking boundary, reads keep working.
#![cfg(all(target_os = "linux", debug_assertions))]
mod support;
use grokbot_engine::{contracts::RunEventKind, model::RunStatus, store};
use grokbot_forms::{redact, requests, vault::Vault};
use grokbot_linux_desktop::testing::Keyring;
use grokbot_wrapper::{features::iphone, ApiResponse, App, Provider};
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
/// Attend les fils de fond du fournisseur avant que le faux serveur et le dossier ne partent,
/// y compris pendant l'unwinding d'une assertion qui échoue (CODING_STANDARDS.md).
impl Drop for Fixture {
    fn drop(&mut self) {
        match self.provider.wait_for_background_work() {
            Ok(()) => {}
            Err(message) if std::thread::panicking() => eprintln!("{message}"),
            Err(message) => panic!("{message}"),
        }
    }
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
        3,
        |phase, keyring| match phase {
            1 => {
                keep_only_in_the_keyring(keyring);
                keyring.lock();
            }
            2 => keyring.unlock(),
            _ => {}
        },
    ) else {
        return;
    };
    let mut f = Fixture::new();
    let saved = f.path.with_extension("chat");
    if phase == 0 {
        let chat = f.chat();
        std::fs::write(&saved, chat).unwrap();
        return;
    }
    let chat = std::fs::read_to_string(&saved).unwrap();
    if phase == 1 {
        let state = f.state();
        assert_eq!(state["forms"]["storage"]["backend"], "file_fallback");
        assert_eq!(state["vault"]["locked"], true);
        // La réponse elle-même ne sert rien de l'historique : seul compte ce qui part.
        let _ = f.app.api(
            "POST",
            "/api/send",
            json!({"conversation_id":chat,"message_id":"m","body":format!("Voici la clé : {SECRET}")}).to_string().as_bytes(),
        );
        wait("fin du tour refusé", || {
            f.db()
                .query_row(
                    "SELECT count(*) FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')='m'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
                > 0
                && !matches!(f.run_of("m").status, RunStatus::Running | RunStatus::Queued)
        });
        assert!(
            !f.sent().contains(SECRET),
            "the secret reached the provider"
        );
        assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 0);
    } else {
        assert_eq!(f.state()["forms"]["storage"]["backend"], "keychain");
        let refused = f
            .messages(&chat)
            .into_iter()
            .find(|m| m["id"] == "reply-m")
            .unwrap();
        assert!(
            refused["error"].as_str().unwrap().contains(MESSAGE),
            "{refused}"
        );
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
fn every_masking_refuses_a_partial_vault() {
    let Some(_) =
        support::keyring_phases("every_masking_refuses_a_partial_vault", 1, |_, keyring| {
            keep_only_in_the_keyring(keyring);
            keyring.lock();
        })
    else {
        return;
    };
    let file = PathBuf::from(std::env::var_os("HOME").unwrap()).join("workspace.secrets.json");
    let vault = Vault::open_app(&file).unwrap();
    assert!(vault.is_partial());
    let refused = |error: grokbot_engine::contracts::Error| {
        assert!(error.to_string().contains(MESSAGE), "{error}");
    };
    refused(redact::redact(&vault, SECRET).unwrap_err());
    refused(redact::redact_value(&vault, &json!(SECRET)).unwrap_err());
    refused(redact::redact_live(&vault, SECRET).unwrap_err());
    refused(redact::app_patterns(&file).err().unwrap());
    refused(
        redact::SnapshotRedactor::default()
            .redact(&vault, &json!({}), false)
            .unwrap_err(),
    );
    refused(vault.values_for_redaction().unwrap_err());
}

#[test]
fn deleting_the_last_secret_leaves_nothing_for_a_locked_keyring_to_hide() {
    if support::keyring_phases(
        "deleting_the_last_secret_leaves_nothing_for_a_locked_keyring_to_hide",
        1,
        |_, keyring| {
            keep_only_in_the_keyring(keyring);
            let file = keyring.home().path().join("workspace.secrets.json");
            Vault::open_test_keychain(&file, keyring.address())
                .unwrap()
                .remove("PORCELAINE")
                .unwrap();
            keyring.lock();
        },
    )
    .is_none()
    {
        return;
    }
    let file = PathBuf::from(std::env::var_os("HOME").unwrap()).join("workspace.secrets.json");
    let vault = Vault::open_app(&file).unwrap();
    assert_eq!(vault.storage_status().backend, "file_fallback");
    assert!(!vault.is_partial());
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

/// Un ancien message contient une valeur enregistrée ensuite comme secret. Trousseau verrouillé
/// après un redémarrage : rien de l'historique n'est servi (ni ici ni à l'iPhone) ni recopié
/// dans un fil. Une fois déverrouillé, le masquage ordinaire revient et rien n'a été stocké brut.
#[test]
fn a_locked_keyring_serves_and_copies_nothing_from_history() {
    let Some(phase) = support::keyring_phases(
        "a_locked_keyring_serves_and_copies_nothing_from_history",
        3,
        |phase, keyring| match phase {
            1 => {
                keep_only_in_the_keyring(keyring);
                keyring.lock();
            }
            2 => keyring.unlock(),
            _ => {}
        },
    ) else {
        return;
    };
    let mut f = Fixture::new();
    let remote = |app: &mut App, method: &str, path: &str, body: Value, token: Option<&str>| {
        let headers = token
            .map(|token| vec![("Cookie".into(), format!("ezerbot_device={token}"))])
            .unwrap_or_default();
        iphone::remote(app, method, path, body.to_string().as_bytes(), &headers)
    };
    let json_of = |response: ApiResponse| match response {
        ApiResponse::Json(value) => value,
        _ => panic!("JSON attendu"),
    };
    let saved = f.path.with_extension("phase0");
    if phase == 0 {
        let chat = f.post("/api/conversations", json!({"title": "Sujet"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":"vieux","body":format!("Note : {SECRET}")}),
        );
        wait("réponse", || {
            f.messages(&chat)
                .iter()
                .any(|m| m["id"] == "reply-vieux" && m["state"] == "echoed")
        });
        f.post("/api/iphone/enabled", json!({"enabled":true}));
        let code = f.post("/api/iphone/code", json!({}))["code"].clone();
        let receipt = json_of(
            remote(
                &mut f.app,
                "POST",
                "/api/iphone/claim",
                json!({"code":code,"name":"iPhone"}),
                None,
            )
            .unwrap(),
        )["receipt"]
            .clone();
        let status = f.app.api("GET", "/api/iphone/status", b"").unwrap();
        let id = status["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["status"] == "pending")
            .unwrap()["id"]
            .clone();
        f.post("/api/iphone/confirm", json!({"id":id,"allow":true}));
        let token = json_of(
            remote(
                &mut f.app,
                "POST",
                "/api/iphone/exchange",
                json!({"receipt":receipt}),
                None,
            )
            .unwrap(),
        )["credential"]
            .as_str()
            .unwrap()
            .to_owned();
        let served =
            json_of(remote(&mut f.app, "GET", "/api/state", json!({}), Some(&token)).unwrap());
        assert!(
            served.to_string().contains(SECRET),
            "témoin : visible avant le secret"
        );
        std::fs::write(&saved, json!({"token": token, "chat": chat}).to_string()).unwrap();
        return;
    }
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&saved).unwrap()).unwrap();
    let (token, chat) = (
        saved["token"].as_str().unwrap(),
        saved["chat"].as_str().unwrap(),
    );
    if phase == 1 {
        let local = f.state();
        assert!(
            !local.to_string().contains(SECRET),
            "le secret est servi localement"
        );
        assert_eq!(local["vault"]["locked"], true, "{}", local["vault"]);
        assert_eq!(
            local["vault"]["message"],
            MESSAGE.to_owned() + " pour continuer."
        );
        let listed = local["conversations"].as_array().unwrap();
        assert!(listed.iter().any(|c| c["id"] == chat));
        assert!(listed
            .iter()
            .all(|c| c["messages"].as_array().unwrap().is_empty()));
        let phone =
            json_of(remote(&mut f.app, "GET", "/api/state", Value::Null, Some(token)).unwrap());
        assert!(
            !phone.to_string().contains(SECRET),
            "le secret a atteint l'iPhone"
        );
        assert_eq!(phone["vault"]["locked"], true);
        let thread = f.app.api(
            "POST",
            "/api/messages/thread",
            json!({"conversation_id":chat,"message_id":"reply-vieux"})
                .to_string()
                .as_bytes(),
        );
        let error = thread.expect_err("un fil copié depuis l'historique verrouillé");
        assert!(error.to_string().contains(MESSAGE), "{error}");
    } else {
        let local = f.state();
        assert_eq!(local["vault"]["locked"], false);
        let text = local.to_string();
        // Ce que l'utilisateur a tapé lui est rendu tel quel (comme avant ce correctif) ;
        // tout le reste passe par le masquage ordinaire.
        let mut others = local.clone();
        for c in others["conversations"].as_array_mut().unwrap() {
            c["messages"]
                .as_array_mut()
                .unwrap()
                .retain(|m| m["role"] != "user");
        }
        assert!(others.to_string().contains("reply-vieux"));
        assert!(
            !others.to_string().contains(SECRET),
            "le masquage ordinaire est revenu"
        );
        assert!(
            text.contains("Note : "),
            "l'historique est de nouveau lisible"
        );
        let raw: i64 = f
            .db()
            .query_row(
                "SELECT (SELECT count(*) FROM wrapper_threads) + (SELECT count(*) FROM wrapper_conversations WHERE title LIKE 'Fil :%')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(raw, 0, "un fil a été recopié pendant le verrouillage");
    }
}
