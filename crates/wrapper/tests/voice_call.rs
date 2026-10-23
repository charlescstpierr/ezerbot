//! L’appel vocal : la couture de session contre un fournisseur de voix en boucle.
//! Aucun microphone, aucun vrai fournisseur ; le test observe le transcript dans le fil,
//! l’absence de fichier audio et l’absence du secret éphémère.
mod support;
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use support::{FakeOpenAi, Seen};

const CALL: &str = "/api/voice/call/debug";
const MODE: &str = "/api/voice/session/debug";

struct Fixture {
    app: App,
    provider: Provider,
    _server: FakeOpenAi,
    dir: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "voice-call-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider =
            Provider::with_endpoints(&dir.join("workspace.sqlite"), server.endpoints(), false);
        let app = App::open_with(&dir.join("workspace.sqlite"), provider.clone()).unwrap();
        Self {
            app,
            provider,
            _server: server,
            dir,
        }
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap()
    }
    fn denied(&mut self, path: &str, body: Value) -> String {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
            .unwrap_err()
            .to_string()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn conversation(&mut self) -> String {
        self.post("/api/conversations", json!({}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn reopen(&mut self) {
        self.app =
            App::open_with(&self.dir.join("workspace.sqlite"), self.provider.clone()).unwrap();
    }
    /// Le début d’un appel : la ligne côté Mac, puis la session du fournisseur ouverte avec le secret.
    fn call(&mut self, conversation: &str) -> (String, String) {
        let started = self.post(
            "/api/voice/call/start",
            json!({"conversation_id":conversation}),
        );
        let id = started["call"]["id"].as_str().unwrap().to_owned();
        let secret = started["secret"].as_str().unwrap().to_owned();
        let opened = self.post(
            CALL,
            json!({"command":"start","token":id,"secret":secret,"microphone":"fake-usb","voice":"fake-fr-CA","speed":1.2,"language":"fr"}),
        );
        assert_eq!(opened["status"], "listening", "{opened}");
        (id, secret)
    }
    /// Ce que dit Charles, puis la réponse du fournisseur de boucle ; les lignes partent vers le fil.
    fn say(&mut self, id: &str, text: &str) -> Value {
        self.post(
            CALL,
            json!({"command":"feed","token":id,"text":text,"final_result":true}),
        )
    }
    fn record_lines(&mut self, id: &str, status: &Value) {
        for (seq, line) in status["lines"].as_array().unwrap().iter().enumerate() {
            self.post(
                "/api/voice/call/line",
                json!({"id":id,"seq":seq,"role":line["role"],"text":line["text"]}),
            );
        }
    }
    fn thread(&mut self, conversation: &str) -> Vec<Value> {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == conversation)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
    /// Tout ce que la base garde, en clair : tables, colonnes, blobs.
    fn everything_stored(&self) -> String {
        let db = Connection::open(self.dir.join("workspace.sqlite")).unwrap();
        let tables: Vec<String> = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut all = String::new();
        for table in tables {
            let columns: Vec<String> = db
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            if columns.is_empty() {
                continue;
            }
            for column in columns {
                let mut query = db
                    .prepare(&format!(
                        "SELECT CAST(\"{column}\" AS BLOB) FROM \"{table}\""
                    ))
                    .unwrap();
                for row in query
                    .query_map([], |r| r.get::<_, Option<Vec<u8>>>(0))
                    .unwrap()
                {
                    all.push_str(&String::from_utf8_lossy(&row.unwrap().unwrap_or_default()));
                    all.push('\n');
                }
            }
        }
        all
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_call_fills_a_transcript_that_stays_in_the_thread_marked_voice_call_without_any_audio_file() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    // Le transcript se remplit pendant l’appel.
    f.say(&id, "Bonjour Ezerbot.");
    let status = f.post(CALL, json!({"command":"status","token":id}));
    let lines = status["lines"].as_array().unwrap();
    assert_eq!(lines[0], json!({"role":"user","text":"Bonjour Ezerbot."}));
    assert_eq!(lines[1]["role"], "assistant");
    f.record_lines(&id, &status);
    assert_eq!(f.state()["voice_call"]["active"]["id"], id);
    // Pas de carte dans le fil tant que Charles n’a pas quitté.
    assert!(f
        .thread(&conversation)
        .iter()
        .all(|m| m["kind"] != "voice_call"));
    // Quitter arrête la session et range le transcript dans le fil.
    f.post(CALL, json!({"command":"cancel","token":id}));
    let ended = f.post("/api/voice/call/end", json!({"id":id}));
    assert_eq!(ended["status"], "ended");
    let card = f
        .thread(&conversation)
        .into_iter()
        .find(|m| m["kind"] == "voice_call")
        .expect("le transcript est dans le fil");
    assert_eq!(card["payload"]["label"], "Appel vocal");
    assert_eq!(card["payload"]["lines"][0]["text"], "Bonjour Ezerbot.");
    assert_eq!(card["payload"]["lines"][0]["role"], "user");
    assert_eq!(card["payload"]["lines"].as_array().unwrap().len(), 2);
    assert!(f.state()["voice_call"]["active"].is_null());
    // Il y reste après un redémarrage, et aucun audio n’est proposé ni gardé.
    f.reopen();
    let card = f
        .thread(&conversation)
        .into_iter()
        .find(|m| m["kind"] == "voice_call")
        .expect("le transcript reste dans le fil");
    assert_eq!(card["payload"]["lines"].as_array().unwrap().len(), 2);
    let text = card.to_string();
    assert!(
        !text.contains("audio_url") && !text.contains("data:audio"),
        "{text}"
    );
    // La base et le coffre de l’espace de travail, jamais un fichier d’appel.
    assert!(
        f.files().iter().all(|name| name.starts_with("workspace.")),
        "{:?}",
        f.files()
    );
}

#[test]
fn the_ephemeral_secret_opens_the_session_and_is_found_nowhere_else() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, secret) = f.call(&conversation);
    assert!(
        secret.len() >= 24,
        "un secret éphémère n’est pas un mot de passe court"
    );
    f.say(&id, "Ceci reste entre nous.");
    let status = f.post(CALL, json!({"command":"status","token":id}));
    assert!(!status.to_string().contains(&secret), "{status}");
    f.record_lines(&id, &status);
    f.post("/api/voice/call/end", json!({"id":id}));
    f.reopen();
    assert!(!f.state().to_string().contains(&secret));
    assert!(!f.everything_stored().contains(&secret));
    let bytes: Vec<u8> = std::fs::read_dir(&f.dir)
        .unwrap()
        .flat_map(|e| std::fs::read(e.unwrap().path()).unwrap())
        .collect();
    assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()));
    // Un secret refusé par le fournisseur n’ouvre pas la session.
    let refused = f.post(CALL, json!({"command":"start","token":"autre","secret":""}));
    assert_eq!(refused["status"], "secret_refused");
    // Chaque appel reçoit son propre secret.
    let second = f.post(
        "/api/voice/call/start",
        json!({"conversation_id":conversation}),
    );
    assert_ne!(second["secret"], secret);
}

#[test]
fn speaking_over_the_bot_or_pressing_escape_cuts_its_voice() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    f.say(&id, "Raconte-moi une longue histoire.");
    let speaking = f.post(CALL, json!({"command":"status","token":id}));
    assert_eq!(speaking["speaking"], true);
    // Charles reprend la parole : la voix du bot s’arrête.
    let over = f.post(CALL, json!({"command":"feed","token":id,"text":"Attends"}));
    assert_eq!(over["speaking"], false);
    assert!(over["barged"].as_u64().unwrap() > speaking["barged"].as_u64().unwrap());
    // Échap coupe aussi la voix, sans que Charles parle.
    f.say(&id, "Et maintenant ?");
    assert_eq!(
        f.post(CALL, json!({"command":"status","token":id}))["speaking"],
        true
    );
    let escaped = f.post(CALL, json!({"command":"interrupt","token":id}));
    assert_eq!(escaped["speaking"], false);
}

#[test]
fn a_card_waiting_for_a_click_holds_the_voice_and_speech_never_answers_it() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    f.post(CALL, json!({"command":"pause","token":id}));
    let held = f.say(&id, "Oui, vas-y, valide.");
    assert_eq!(held["speaking"], false, "la voix attend");
    assert_eq!(held["held"], 1, "la parole est retenue, pas transmise");
    assert!(held["lines"].as_array().unwrap().is_empty());
    let resumed = f.post(CALL, json!({"command":"resume","token":id}));
    assert_eq!(resumed["held"], 0);
    assert!(
        resumed["lines"].as_array().unwrap().is_empty(),
        "la parole retenue n’est jamais rejouée à la carte"
    );
}

#[test]
fn the_call_listens_to_the_chosen_microphone_and_speaks_with_the_voice_of_the_bot() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    let status = f.say(&id, "Bonjour");
    assert_eq!(status["microphone_id"], "fake-usb");
    assert_eq!(status["microphone_fallback"], false);
    assert_eq!(
        status["recorded_voice"],
        json!({"voice":"fake-fr-CA","speed":1.2,"language":"fr"})
    );
    f.post(CALL, json!({"command":"cancel","token":id}));
    f.post("/api/voice/call/end", json!({"id":id}));
    let started = f.post(
        "/api/voice/call/start",
        json!({"conversation_id":conversation}),
    );
    let id = started["call"]["id"].as_str().unwrap();
    let missing = f.post(
        CALL,
        json!({"command":"start","token":id,"secret":started["secret"],"microphone":"micro-disparu"}),
    );
    assert_eq!(missing["microphone_id"], "fake-built-in");
    assert_eq!(missing["microphone_fallback"], true);
}

#[test]
fn the_call_and_the_voice_mode_refuse_each_other_and_a_second_call_is_refused() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    // Le mode voix tient le microphone : l’appel est refusé.
    assert_eq!(
        f.post(MODE, json!({"command":"start","token":"mode"}))["status"],
        "listening"
    );
    let started = f.post(
        "/api/voice/call/start",
        json!({"conversation_id":conversation}),
    );
    let id = started["call"]["id"].as_str().unwrap().to_owned();
    let refused = f.post(
        CALL,
        json!({"command":"start","token":id,"secret":started["secret"]}),
    );
    assert_eq!(refused["status"], "microphone_busy");
    f.post(MODE, json!({"command":"cancel","token":"mode"}));
    f.post("/api/voice/call/end", json!({"id":id}));
    // L’inverse : l’appel tient le microphone, le mode voix est refusé.
    let (call, _) = f.call(&conversation);
    assert_eq!(
        f.post(MODE, json!({"command":"start","token":"mode"}))["status"],
        "microphone_busy"
    );
    // Un seul appel à la fois.
    let again = f.denied(
        "/api/voice/call/start",
        json!({"conversation_id":conversation}),
    );
    assert!(again.contains("déjà en cours"), "{again}");
    f.post(CALL, json!({"command":"cancel","token":call}));
    assert_eq!(
        f.post(MODE, json!({"command":"start","token":"mode"}))["status"],
        "listening"
    );
}

#[test]
fn closing_the_app_ends_the_call_keeps_the_written_transcript_and_never_restarts() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    f.say(&id, "Première phrase.");
    let status = f.post(CALL, json!({"command":"status","token":id}));
    f.record_lines(&id, &status);
    f.reopen();
    let state = f.state();
    assert!(
        state["voice_call"]["active"].is_null(),
        "l’appel ne redémarre pas seul"
    );
    let card = f
        .thread(&conversation)
        .into_iter()
        .find(|m| m["kind"] == "voice_call")
        .expect("le transcript déjà écrit est là");
    assert_eq!(card["payload"]["status"], "interrupted");
    assert_eq!(card["payload"]["lines"][0]["text"], "Première phrase.");
    // La session du fournisseur n’est pas rouverte non plus.
    assert!(f
        .denied(CALL, json!({"command":"status","token":id}))
        .contains("plus actif"));
}

#[test]
fn an_interrupted_call_without_a_single_line_leaves_nothing_in_the_thread() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    f.call(&conversation);
    f.reopen();
    assert!(f
        .thread(&conversation)
        .iter()
        .all(|m| m["kind"] != "voice_call"));
    assert!(f.state()["voice_call"]["active"].is_null());
}

#[test]
fn lines_are_written_once_checked_and_refused_once_the_call_has_ended() {
    let mut f = Fixture::new();
    let conversation = f.conversation();
    let (id, _) = f.call(&conversation);
    let line = json!({"id":id,"seq":0,"role":"user","text":"Bonjour"});
    f.post("/api/voice/call/line", line.clone());
    f.post("/api/voice/call/line", line);
    assert!(f
        .denied(
            "/api/voice/call/line",
            json!({"id":id,"seq":1,"role":"robot","text":"x"})
        )
        .contains("invalide"));
    assert!(f
        .denied(
            "/api/voice/call/line",
            json!({"id":id,"seq":1,"role":"user","text":"   "})
        )
        .contains("invalide"));
    f.post("/api/voice/call/end", json!({"id":id}));
    f.post("/api/voice/call/end", json!({"id":id}));
    let card = f
        .thread(&conversation)
        .into_iter()
        .filter(|m| m["kind"] == "voice_call")
        .collect::<Vec<_>>();
    assert_eq!(card.len(), 1, "un seul transcript dans le fil");
    assert_eq!(card[0]["payload"]["lines"].as_array().unwrap().len(), 1);
    assert!(f
        .denied(
            "/api/voice/call/line",
            json!({"id":id,"seq":1,"role":"user","text":"Trop tard"})
        )
        .contains("terminé"));
}

#[test]
fn the_call_is_offered_on_the_mac_only() {
    let mut f = Fixture::new();
    let state = f.state();
    assert_eq!(state["voice_call"]["available"], true);
    assert!(state["voice_call"]["debug_fake"].as_bool().unwrap());
}
