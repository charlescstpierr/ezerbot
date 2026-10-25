//! Répondre en fil : le sous-fil cite le message parent, et le fil principal le garde.
mod support;

use grokbot_engine::contracts::ContextSource;
use grokbot_wrapper::{ApiError, App, Provider};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const QUESTION: &str = "Quelle est la capitale du Canada ?";
const PARENT_REPLY: &str = "La capitale précise est Ottawa.";

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    dir: std::path::PathBuf,
    _dir: grokbot_test_support::TestDir,
}

impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("threads-test");
        let dir = guard.to_path_buf();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider =
            Provider::with_endpoints(&dir.join("workspace.sqlite"), server.endpoints(), false);
        let app = App::open_with(&dir.join("workspace.sqlite"), provider.clone()).unwrap();
        Self {
            app,
            provider,
            server,
            dir,
            _dir: guard,
        }
    }

    fn post(&mut self, path: &str, body: Value) -> Value {
        self.post_result(path, body).unwrap()
    }

    fn post_result(&mut self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
    }

    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }

    fn connect(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            self.dir.join("workspace.chatgpt-auth.json"),
            json!({
                "host_id": "test-host",
                "client_id": CLIENT,
                "access_token": "access-2",
                "refresh_token": "refresh-2",
                "expires_at": now + 3600,
                "scopes": "chatgpt.tokens.use.direct",
            })
            .to_string(),
        )
        .unwrap();
        self.provider.refresh_models();
        wait("catalogue", || {
            self.provider.status()["models_ready"] == true
        });
        assert!(self.provider.connected());
    }

    fn reopen(&mut self) {
        self.app =
            App::open_with(&self.dir.join("workspace.sqlite"), self.provider.clone()).unwrap();
    }

    /// Le bot ne tient compte du parent que si la requête le contient.
    fn answer_with_parent(&self) {
        self.server.seen.lock().unwrap().reply_for = Some(Arc::new(|body: &Value| {
            let instructions = body["instructions"].as_str().unwrap_or("");
            if instructions.starts_with("Extraction de mémoire automatique.")
                || instructions.contains("Tu écris le titre")
            {
                return None;
            }
            let blob = body.to_string();
            let text = if blob.contains("Et en un mot ?") && blob.contains("Ottawa") {
                "En tenant compte du message parent : Ottawa."
            } else if blob.contains("Et en un mot ?") {
                "Je ne vois pas de message parent."
            } else {
                PARENT_REPLY
            };
            Some(ScriptedResponse::events(&[
                json!({"type": "response.created"}),
                json!({"type": "response.output_text.delta", "delta": text}),
                json!({"type": "response.completed"}),
            ]))
        }));
    }

    fn conversation(&mut self) -> String {
        self.post("/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn ask(&mut self, conversation: &str, message: &str, body: &str) {
        self.post(
            "/api/send",
            json!({"conversation_id": conversation, "message_id": message, "body": body}),
        );
    }

    fn reply(&mut self, conversation: &str, message: &str) -> String {
        let reply_id = format!("reply-{message}");
        let mut body = String::new();
        wait(&format!("réponse {reply_id}"), || {
            let state = self.state();
            let Some(found) = state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == conversation)
            else {
                return false;
            };
            let Some(message) = found["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"] == reply_id && item["state"] == "echoed")
            else {
                return false;
            };
            body = message["body"].as_str().unwrap_or("").to_owned();
            true
        });
        body
    }

    fn messages(&mut self, conversation: &str) -> Vec<Value> {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == conversation)
            .map(|item| item["messages"].as_array().unwrap().clone())
            .unwrap_or_default()
    }

    fn parent_prompt(&self, conversation: &str) -> String {
        self.app
            .context_preview(conversation)
            .unwrap()
            .into_iter()
            .filter(|block| block.source == ContextSource::DelegatedContext)
            .map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[test]
fn le_sous_fil_cite_le_parent_et_le_fil_principal_garde_le_message() {
    let mut f = Fixture::new();
    f.connect();
    f.answer_with_parent();
    let main = f.conversation();
    f.ask(&main, "question", QUESTION);
    assert_eq!(f.reply(&main, "question"), PARENT_REPLY);
    let before = f.messages(&main);

    let opened = f.post(
        "/api/messages/thread",
        json!({"conversation_id": main, "message_id": "reply-question"}),
    );
    let thread = opened["conversation"]["id"].as_str().unwrap().to_owned();
    assert_ne!(thread, main);
    assert_eq!(opened["thread"]["parent_conversation_id"], main);
    assert_eq!(opened["thread"]["parent_message_id"], "reply-question");
    assert_eq!(opened["thread"]["parent_role"], "assistant");
    assert!(opened["thread"]["parent_excerpt"]
        .as_str()
        .unwrap()
        .contains("Ottawa"));
    assert!(opened["conversation"]["messages"]
        .as_array()
        .unwrap()
        .is_empty());

    // Le bot reçoit le message parent avant même la première question du sous-fil.
    assert!(
        f.parent_prompt(&thread).contains("Ottawa"),
        "le sous-fil doit citer le parent : {}",
        f.parent_prompt(&thread)
    );
    assert!(!f.parent_prompt(&main).contains("message parent"));

    f.ask(&thread, "suite", "Et en un mot ?");
    assert_eq!(
        f.reply(&thread, "suite"),
        "En tenant compte du message parent : Ottawa."
    );

    // Le fil principal montre toujours ce message et ne reçoit rien du sous-fil.
    let after = f.messages(&main);
    assert_eq!(after.len(), before.len());
    let parent = after
        .iter()
        .find(|message| message["id"] == "reply-question")
        .expect("le message parent reste dans le fil principal");
    assert_eq!(parent["body"], PARENT_REPLY);
    assert!(!after
        .iter()
        .any(|message| message["body"] == "Et en un mot ?"));

    // Rouvrir « Répondre en fil » sur le même message rend le même sous-fil.
    let again = f.post(
        "/api/messages/thread",
        json!({"conversation_id": main, "message_id": "reply-question"}),
    );
    assert_eq!(again["conversation"]["id"], thread);
    assert_eq!(
        again["conversation"]["messages"].as_array().unwrap().len(),
        2
    );

    // Le lien survit au redémarrage.
    f.reopen();
    let state = f.state();
    assert_eq!(
        state["threads"][&thread]["parent_message_id"],
        "reply-question"
    );
    assert_eq!(state["threads"][&thread]["parent_conversation_id"], main);
}

#[test]
fn un_message_ou_une_conversation_absente_n_ouvre_pas_de_fil() {
    let mut f = Fixture::new();
    let main = f.conversation();
    for (conversation, message) in [(main.as_str(), "absent"), ("inconnue", "absent")] {
        let error = f
            .post_result(
                "/api/messages/thread",
                json!({"conversation_id": conversation, "message_id": message}),
            )
            .unwrap_err();
        assert_eq!(error.status, 404);
    }
    assert!(f.state()["threads"].as_object().unwrap().is_empty());
}

#[test]
fn r6_thread_excerpt_is_masked_on_reopen_and_state() {
    let mut f = Fixture::new();
    f.connect();
    f.answer_with_parent();
    let main = f.conversation();
    f.ask(&main, "question", QUESTION);
    f.reply(&main, "question");
    let input = json!({"conversation_id": main, "message_id": "reply-question"});
    let opened = f.post("/api/messages/thread", input.clone());
    let thread = opened["conversation"]["id"].as_str().unwrap().to_owned();
    assert!(opened["thread"]["parent_excerpt"]
        .as_str()
        .unwrap()
        .contains(PARENT_REPLY));
    let vault = grokbot_forms::vault::Vault::open(&f.dir.join("workspace.secrets.json")).unwrap();
    vault
        .set("R6_PARENT", "Fixture synthétique", PARENT_REPLY)
        .unwrap();
    assert!(
        !f.parent_prompt(&thread).contains(PARENT_REPLY),
        "le contexte masque le parent"
    );
    for restarted in [false, true] {
        if restarted {
            f.reopen();
        }
        let state = f.state();
        assert!(
            !state.to_string().contains(PARENT_REPLY),
            "le poll masque le parent"
        );
        assert!(state["threads"][&thread]["parent_excerpt"]
            .as_str()
            .unwrap()
            .contains("[...]"));
        let reopened = f.post("/api/messages/thread", input.clone());
        assert!(
            !reopened.to_string().contains(PARENT_REPLY),
            "la réouverture masque le parent"
        );
        assert_eq!(reopened["conversation"]["id"], thread);
        assert!(reopened["thread"]["parent_excerpt"]
            .as_str()
            .unwrap()
            .contains("[...]"));
    }
    vault.remove("R6_PARENT").unwrap();
    f.reopen();
    let reopened = f.post("/api/messages/thread", input);
    assert!(
        !reopened.to_string().contains(PARENT_REPLY),
        "le retrait du coffre ne réexpose pas le parent"
    );
    assert!(
        !f.state().to_string().contains(PARENT_REPLY),
        "le poll garde le masque après retrait"
    );
}
