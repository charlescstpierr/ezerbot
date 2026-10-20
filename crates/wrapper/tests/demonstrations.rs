//! Safe end-to-end Teach → Responses → ordinary propose_skill approval.
//! Debug fixture only: no screen, input observation, native app or TCC calls.
#![cfg(debug_assertions)]
mod support;
use grokbot_forms::requests::{self, HumanInputRequest};
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: std::path::PathBuf,
    chat: String,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ezerbot-demo-e2e-{}-{}", std::process::id(), now()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("workspace.sqlite");
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api(
                "POST",
                "/api/conversations",
                r#"{"title":"Démonstration fictive"}"#.as_bytes(),
            )
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        Self {
            app,
            provider,
            server,
            path,
            chat,
        }
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn status(&self, id: &str) -> String {
        self.db()
            .query_row(
                "SELECT status FROM wrapper_demonstrations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn record(&mut self) -> String {
        let id = self.post(
            "/api/demonstrations/start",
            json!({"bot_id":"grokbot","conversation_id":self.chat,"consent":true}),
        )["session"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.post("/api/demonstrations/stop", json!({"id":id}));
        id
    }
    fn draft(&mut self, id: &str, name: &str, keep: bool) -> HumanInputRequest {
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","call_id":id,"name":"propose_skill","arguments":json!({"name":name,"description":"Reproduire la tâche montrée","body":"1. Ouvrir l’éditeur.\n2. Cliquer dans le document.\n3. Saisir soi-même le texte (non enregistré)."}).to_string()}}),json!({"type":"response.completed"})]));
        self.post("/api/demonstrations/draft",json!({"id":id,"explanation":"Créer une note sans inclure de secrets.","keep_images":keep}));
        wait("demonstration approval", || {
            self.db()
                .query_row(
                    "SELECT request_id IS NOT NULL FROM wrapper_demonstrations WHERE id=?1",
                    [id],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap_or(false)
        });
        let request: String = self
            .db()
            .query_row(
                "SELECT request_id FROM wrapper_demonstrations WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        requests::get(&self.db(), &request).unwrap().unwrap()
    }
    fn decide(&mut self, r: &HumanInputRequest, decision: Value) {
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"submission_id":"manual","conversation_id":r.route.target_conversation_id,"decision":decision}));
    }
    fn finish(&mut self, id: &str, status: &str) {
        for _ in 0..500 {
            self.app.tick(now());
            let terminal=self.db().query_row("SELECT NOT EXISTS(SELECT 1 FROM engine_runs WHERE status IN ('running','queued','awaiting_approval','awaiting_human_input'))",[],|r|r.get::<_,bool>(0)).unwrap();
            if self.status(id) == status && terminal && self.provider.status()["titling"] == false {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("demo status {} expected {status}", self.status(id));
    }
    fn restart(&mut self) {
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        grokbot_wrapper::features::demonstrations::startup(&self.db(), &self.path).unwrap();
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
#[test]
fn demonstration_to_skill_save_discard_cancel_restart_and_raw_retention() {
    let mut f = Fixture::new();
    for (i, keep) in [false, true].into_iter().enumerate() {
        let id = f.record();
        let request = f.draft(&id, &format!("Note démontrée {i}"), keep);
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        let card = state["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == request.id)
            .unwrap();
        assert_eq!(card["arguments"]["demonstration"]["id"], id);
        assert_eq!(
            f.db()
                .query_row("SELECT count(*) FROM skills_library", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            i as i64
        );
        assert!(
            f.server
                .seen
                .lock()
                .unwrap()
                .bodies
                .iter()
                .any(|b| b.to_string().contains("data:image/png;base64,")),
            "recorded fixture must reach the real model path"
        );
        // Double draft is idempotent and cannot create another run/card.
        f.post(
            "/api/demonstrations/draft",
            json!({"id":id,"explanation":"Autre texte ignoré","keep_images":!keep}),
        );
        f.decide(&request, json!("allow_once"));
        f.finish(&id, "saved");
        let d = f
            .app
            .api("GET", &format!("/api/demonstrations/summary?id={id}"), b"")
            .unwrap();
        assert!(d["demonstration"]["summary"]
            .as_str()
            .unwrap()
            .contains("Éditeur fictif"));
        assert_eq!(!d["images"].as_array().unwrap().is_empty(), keep);
        let skill = d["demonstration"]["skill_id"].clone();
        f.restart();
        let links = f.app.api("GET", "/api/state", b"").unwrap()["demonstrations"]["links"].clone();
        assert!(links
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["skill_id"] == skill));
    }
    let count = f
        .db()
        .query_row("SELECT count(*) FROM skills_library", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    for cancel in [false, true] {
        let id = f.record();
        let request = f.draft(
            &id,
            if cancel {
                "Note annulée"
            } else {
                "Note ignorée"
            },
            true,
        );
        if cancel {
            f.post("/api/forms/cancel",json!({"entry_id":request.entry_id,"submission_id":"cancel-demo","conversation_id":request.route.target_conversation_id}));
        } else {
            f.decide(&request, json!("deny"));
        }
        f.finish(&id, "discarded");
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT summary FROM wrapper_demonstrations WHERE id=?1",
                    [&id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            ""
        );
        assert!(f
            .app
            .api("GET", &format!("/api/demonstrations/summary?id={id}"), b"")
            .is_err());
    }
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM skills_library", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        count
    );
    // Pending approval survives restart, but raw pixels never do, even when keep was
    // checked. The existing approval is still required and retains the summary.
    let id = f.record();
    let request = f.draft(&id, "Note après redémarrage", true);
    f.restart();
    let after_restart = f.app.api("GET", "/api/state", b"").unwrap();
    let card = after_restart["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == request.id)
        .unwrap();
    assert_eq!(card["arguments"]["demonstration"]["keep_images"], false);
    f.decide(&request, json!("allow_once"));
    f.finish(&id, "saved");
    let d = f
        .app
        .api("GET", &format!("/api/demonstrations/summary?id={id}"), b"")
        .unwrap();
    assert!(d["images"].as_array().unwrap().is_empty());
    assert!(!d["demonstration"]["summary"].as_str().unwrap().is_empty());
    // A late media-write failure rolls back the skill and its provenance too.
    let before: i64 = f
        .db()
        .query_row("SELECT count(*) FROM skills_library", [], |r| r.get(0))
        .unwrap();
    let id = f.record();
    let request = f.draft(&id, "Note stockage en échec", true);
    f.db().execute_batch("CREATE TRIGGER demo_fail_media BEFORE INSERT ON wrapper_demonstration_images BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;").unwrap();
    f.decide(&request, json!("allow_once"));
    f.finish(&id, "discarded");
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM skills_library", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    f.db()
        .execute_batch("DROP TRIGGER demo_fail_media")
        .unwrap();
    // Explicit Teach cancel also stops its extraction run (not merely the card).
    let id = f.record();
    let request = f.draft(&id, "Note extraction annulée", false);
    f.post("/api/demonstrations/discard", json!({"id":id}));
    f.finish(&id, "discarded");
    assert!(matches!(
        requests::get(&f.db(), &request.id).unwrap().unwrap().status,
        requests::RequestStatus::Cancelled { .. }
    ));
    // A provider failure leaves a reviewable recording, never an auto-saved skill.
    let id = f.record();
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            500,
            json!({"error":{"message":"Erreur fictive"}}),
        ));
    f.post(
        "/api/demonstrations/draft",
        json!({"id":id,"explanation":"Essai en échec","keep_images":false}),
    );
    f.finish(&id, "review");
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["demonstrations"]["session"]["notice"]
        .as_str()
        .unwrap()
        .contains("Réessayez"));
    f.post("/api/demonstrations/discard", json!({"id":id}));
    // Signing out cancels the pending approval before changing identity. It can
    // no longer save or retain even opt-in media under the next account.
    let id = f.record();
    let request = f.draft(&id, "Note compte précédent", true);
    f.post("/api/provider/signout", json!({}));
    assert_eq!(f.status(&id), "discarded");
    assert!(matches!(
        requests::get(&f.db(), &request.id).unwrap().unwrap().status,
        requests::RequestStatus::Cancelled { .. }
    ));
}
