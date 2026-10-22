//! Réunions : le parcours du catalogue contre un fournisseur Recall de boucle.
//! Le fournisseur compte les invitations et les sorties ; aucun vrai appel Recall.
mod support;
use grokbot_engine::{
    contracts::{RunEventKind, ToolOutcome},
    store,
};
use grokbot_forms::requests;
use grokbot_wrapper::{features::recall, App, Provider};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};
use tiny_http::{Header, Response, Server};

const KEY: &str = "recall-key-never-visible-0123456789abcdef";
const MEET: &str = "https://meet.google.com/abc-defg-hij";

struct SeenRecall {
    creates: Vec<(String, Value)>,
    leaves: Vec<String>,
    code: String,
    create_status: u16,
    leave_status: u16,
    create_delay: Duration,
}
struct Recall {
    base: String,
    seen: Arc<Mutex<SeenRecall>>,
    stop: Arc<AtomicBool>,
}
impl Recall {
    fn new() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let seen = Arc::new(Mutex::new(SeenRecall {
            creates: vec![],
            leaves: vec![],
            code: "joining_call".into(),
            create_status: 201,
            leave_status: 200,
            create_delay: Duration::ZERO,
        }));
        let captured = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Some(mut req) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                let url = req.url().to_owned();
                let method = req.method().as_str().to_owned();
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.to_string())
                    .unwrap_or_default();
                let mut body = String::new();
                req.as_reader().read_to_string(&mut body).unwrap();
                let mut seen = captured.lock().unwrap();
                let (status, data) = if method == "POST" && url == "/api/v1/bot/" {
                    seen.creates
                        .push((auth, serde_json::from_str(&body).unwrap_or(Value::Null)));
                    let delay = seen.create_delay;
                    drop(seen);
                    std::thread::sleep(delay);
                    seen = captured.lock().unwrap();
                    (
                        seen.create_status,
                        json!({"id":"bot-1","status_changes":[]}),
                    )
                } else if method == "POST" && url.ends_with("/leave_call/") {
                    seen.leaves.push(url.clone());
                    (seen.leave_status, json!({}))
                } else if method == "GET" && url.starts_with("/api/v1/bot/") {
                    (
                        200,
                        json!({"id":"bot-1","status_changes":[{"code":"joining_call"},{"code":seen.code}]}),
                    )
                } else {
                    (404, json!({}))
                };
                drop(seen);
                let _ = req.respond(
                    Response::from_string(data.to_string())
                        .with_status_code(status)
                        .with_header(
                            Header::from_bytes("Content-Type", "application/json").unwrap(),
                        ),
                );
            }
        });
        Self { base, seen, stop }
    }
    fn creates(&self) -> usize {
        self.seen.lock().unwrap().creates.len()
    }
    fn leaves(&self) -> usize {
        self.seen.lock().unwrap().leaves.len()
    }
    fn code(&self, code: &str) {
        self.seen.lock().unwrap().code = code.into();
    }
}
impl Drop for Recall {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
static LOCK: Mutex<()> = Mutex::new(());
struct Fixture {
    app: App,
    path: PathBuf,
    ai: FakeOpenAi,
    recall: Recall,
    _guard: MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "ezerbot-recall-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let ai = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, ai.endpoints(), false);
        let recall = Recall::new();
        std::env::set_var("GROKBOT_FAKE_RECALL", &recall.base);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        Self {
            app,
            path,
            ai,
            recall,
            _guard: guard,
        }
    }
    fn db(&self) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(4)).unwrap();
        c
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn refused(&mut self, path: &str, body: Value) -> grokbot_wrapper::ApiError {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap_err()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn item(&mut self) -> Value {
        self.state()["mcp"]["catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "reunions")
            .unwrap()
            .clone()
    }
    fn connect(&mut self) {
        self.post("/api/recall/connect", json!({"key":KEY}));
    }
    fn sync(&self) {
        recall::sync(&self.app.feature_context());
    }
    fn meeting_state(&mut self) -> String {
        self.item()["meeting"]["state"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }
    fn request(&self, run: &str) -> requests::HumanInputRequest {
        wait("approval", || {
            !requests::list_for_run(&self.db(), run).unwrap().is_empty()
        });
        requests::list_for_run(&self.db(), run).unwrap().remove(0)
    }
    fn decide(&mut self, run: &str, decision: Value) {
        let r = self.request(run);
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":format!("decide-{run}"),"decision":decision}));
        self.app.tick(0);
        wait("terminal host action", || {
            store::required(&self.db(), run)
                .unwrap()
                .status
                .is_terminal()
        });
    }
    fn join(&mut self, link: &str) -> String {
        self.post("/api/recall/join", json!({"link":link}))["run_id"]
            .as_str()
            .unwrap()
            .into()
    }
    /// Invite et accepte : le bot est alors en cours de connexion.
    fn joined(&mut self) {
        let run = self.join(MEET);
        self.decide(&run, json!("allow_once"));
        assert_eq!(self.meeting_state(), "joining");
    }
    fn outcome(&self, run: &str) -> ToolOutcome {
        let event = store::events(&self.db(), run, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult)
            .unwrap();
        serde_json::from_value(event.payload["outcome"].clone()).unwrap()
    }
    fn script_call(&self, name: &str, id: &str, arguments: Value) {
        let mut seen = self.ai.seen.lock().unwrap();
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":arguments.to_string()}}),
            json!({"type":"response.completed"}),
        ]));
    }
    fn script_done(&self) {
        self.ai
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&[
                json!({"type":"response.completed"}),
            ]));
    }
    fn say(&mut self, id: &str) -> String {
        let conversation = self.post("/api/conversations", json!({"title":"Réunion"}))
            ["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.post(
            "/api/send",
            json!({"conversation_id":conversation,"message_id":id,"body":"Va dans la réunion"}),
        );
        self.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get::<_,String>(0)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        recall::pause_background(false);
        std::env::remove_var("GROKBOT_FAKE_RECALL");
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}

#[test]
fn the_catalogue_offers_meetings_and_never_shows_the_key_again() {
    let mut f = Fixture::new();
    let item = f.item();
    assert_eq!(item["name"], "Réunions");
    assert_eq!(item["status"], "disconnected");
    assert_eq!(item["has_key"], false);
    let state = f.state();
    assert_eq!(state["mcp"]["catalogue"].as_array().unwrap().len(), 7);
    // Les six autres entrées restent telles quelles.
    for id in ["gmail", "calendar", "drive", "slack", "notion", "github"] {
        assert!(state["mcp"]["catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == id && m["needs_client"].is_boolean()));
    }
    for empty in ["", "   "] {
        let error = f.refused("/api/recall/connect", json!({"key":empty}));
        assert_eq!(error.status, 400);
        assert!(error.message.contains("La clé Recall n’est pas configurée"));
    }
    assert_eq!(f.item()["status"], "disconnected");
    f.connect();
    let item = f.item();
    assert_eq!(item["status"], "connected");
    assert_eq!(item["has_key"], true);
    // La clé est au coffre de cette entrée, nulle part ailleurs.
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    assert_eq!(
        vault.get("REUNIONS_RECALL_KEY").unwrap().as_deref(),
        Some(KEY)
    );
    let public = f.state().to_string();
    assert!(!public.contains(KEY));
    for file in [f.path.clone(), f.path.with_extension("sqlite-wal")] {
        if let Ok(bytes) = std::fs::read(file) {
            assert!(!bytes.windows(KEY.len()).any(|w| w == KEY.as_bytes()));
        }
    }
    assert_eq!(f.recall.creates(), 0);
}

#[test]
fn a_link_outside_meet_zoom_and_teams_is_refused_before_any_card_or_recall_call() {
    let mut f = Fixture::new();
    // Sans clé, l’invitation dit que la clé n’est pas configurée.
    let error = f.refused("/api/recall/join", json!({"link":MEET}));
    assert!(error.message.contains("La clé Recall n’est pas configurée"));
    f.connect();
    for link in [
        "https://example.com/reunion",
        "http://meet.google.com/abc",
        "https://meet.google.com.evil.test/abc",
        "pas un lien",
    ] {
        let error = f.refused("/api/recall/join", json!({"link":link}));
        assert_eq!(error.status, 400);
        assert_eq!(
            error.message,
            "Ce lien n’est pas une réunion Google Meet, Zoom ou Microsoft Teams. Collez le lien d’invitation de l’un de ces services."
        );
    }
    assert_eq!(f.recall.creates(), 0);
    let requests: i64 = f
        .db()
        .query_row("SELECT count(*) FROM forms_requests", [], |r| r.get(0))
        .unwrap();
    assert_eq!(requests, 0);
}

#[test]
fn the_consent_card_gates_the_invitation_and_cancel_invites_nobody() {
    let mut f = Fixture::new();
    f.connect();
    let run = f.join(MEET);
    // La carte est ouverte, Recall n’a encore rien reçu.
    let request = f.request(&run);
    assert_eq!(f.recall.creates(), 0);
    let state = f.state();
    let card = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap()
        .clone();
    assert_eq!(card["recall"]["link"], MEET);
    assert_eq!(card["recall"]["phrase"], recall::CONSENT);
    assert!(card["recall"]["bot_name"]
        .as_str()
        .is_some_and(|n| !n.is_empty()));
    // Pas de « Toujours » : la règle permanente est refusée.
    assert_eq!(card["policy"]["remember"], false);
    let error = f.refused("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"always","decision":{"always_allow":{"scope":"global"}}}));
    assert_eq!(error.status, 400);
    assert_eq!(f.recall.creates(), 0);
    // Annuler n’invite personne et ne garde aucune invitation en attente.
    f.decide(&run, json!("deny"));
    assert_eq!(f.recall.creates(), 0);
    assert_eq!(f.item()["meeting"], Value::Null);
    // Accepter : Connexion, puis En cours quand Recall voit le bot dans l’appel.
    let run = f.join(MEET);
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.recall.creates(), 1);
    assert_eq!(
        f.recall.seen.lock().unwrap().creates[0].0,
        format!("Token {KEY}")
    );
    assert_eq!(
        f.recall.seen.lock().unwrap().creates[0].1["meeting_url"],
        MEET
    );
    assert_eq!(f.item()["meeting"]["label"], "Connexion");
    f.recall.code("in_call_recording");
    f.sync();
    assert_eq!(f.item()["meeting"]["label"], "En cours");
    // Une réunion Recall n'occupe pas le micro du Mac : la capture locale reste intacte.
    let local = f.state()["meetings"].clone();
    assert_eq!(local["interrupted"], json!([]));
    assert_eq!(local["max_seconds"], 600);
    // Une seconde invitation pendant En cours est refusée sans carte ni appel.
    let error = f.refused("/api/recall/join", json!({"link":MEET}));
    assert_eq!(error.status, 409);
    assert!(error.message.contains("déjà en cours"));
    assert_eq!(f.recall.creates(), 1);
}

#[test]
fn leaving_cancels_a_connecting_bot_and_ends_a_running_one() {
    let mut f = Fixture::new();
    f.connect();
    f.joined();
    let id = f.item()["meeting"]["id"].as_str().unwrap().to_owned();
    f.post("/api/recall/leave", json!({"id":id}));
    assert_eq!(f.recall.leaves(), 1);
    assert_eq!(f.item()["meeting"]["label"], "Annulée");
    f.joined();
    f.recall.code("in_call_recording");
    f.sync();
    assert_eq!(f.meeting_state(), "in_call");
    f.post("/api/recall/leave", json!({}));
    assert_eq!(f.recall.leaves(), 2);
    assert_eq!(f.item()["meeting"]["label"], "Terminée");
    // Sortir deux fois n’appelle pas Recall une seconde fois.
    assert_eq!(f.refused("/api/recall/leave", json!({})).status, 409);
    assert_eq!(f.recall.leaves(), 2);
    // Une sortie que Recall ne confirme pas ne dit pas que le bot est sorti.
    f.joined();
    f.recall.seen.lock().unwrap().leave_status = 500;
    let error = f.refused("/api/recall/leave", json!({}));
    assert!(error.message.contains("n’a pas confirmé"));
    assert_eq!(f.meeting_state(), "joining");
}

#[test]
fn a_recall_refusal_or_an_unknown_state_is_a_failure_without_a_retry_loop() {
    let mut f = Fixture::new();
    f.connect();
    f.recall.seen.lock().unwrap().create_status = 400;
    let run = f.join(MEET);
    f.decide(&run, json!("allow_once"));
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.item()["meeting"]["label"], "Échec");
    assert_eq!(
        f.item()["meeting"]["message"],
        "Recall a refusé ce lien de réunion."
    );
    f.sync();
    f.sync();
    assert_eq!(f.recall.creates(), 1);
    f.recall.seen.lock().unwrap().create_status = 201;
    f.joined();
    f.recall.code("nouvel_etat_inconnu");
    f.sync();
    let item = f.item();
    assert_eq!(item["meeting"]["label"], "Échec");
    let message = item["meeting"]["message"].as_str().unwrap();
    assert!(message.contains("ne reconnaît pas"));
    assert!(!message.contains("nouvel_etat_inconnu"));
}

#[test]
fn a_clean_close_removes_the_bot_and_a_crash_never_leaves_in_progress() {
    let mut f = Fixture::new();
    f.connect();
    f.joined();
    f.recall.code("in_call_recording");
    f.sync();
    assert_eq!(f.meeting_state(), "in_call");
    let left = f.post("/api/recall/shutdown", json!({}));
    assert_eq!(left["left"], 1);
    assert!(f.recall.leaves() >= 1);
    assert_eq!(f.item()["meeting"]["label"], "Terminée");
    // Arrêt brutal : rien ne ferme la réunion, la réouverture ne dit plus En cours.
    f.joined();
    f.recall.code("in_call_recording");
    f.sync();
    assert_eq!(f.meeting_state(), "in_call");
    let before = f.recall.leaves();
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("models", || provider.status()["models_ready"] == true);
    let item = f.item();
    assert_eq!(item["meeting"]["label"], "Échec");
    assert_eq!(item["meeting"]["message"], recall::RESUME);
    assert_eq!(item["meeting"]["active"], false);
    // Ezerbot redemande la sortie une seule fois, sans boucle.
    f.sync();
    f.sync();
    assert_eq!(f.recall.leaves(), before + 1);
    assert_eq!(f.item()["meeting"]["message"], recall::RESUME);
}

#[test]
fn disconnecting_asks_for_a_confirmation_and_leaves_a_running_meeting_first() {
    let mut f = Fixture::new();
    f.connect();
    f.joined();
    let run = f.post("/api/recall/disconnect", json!({}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let request = f.request(&run);
    let state = f.state();
    let card = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap()
        .clone();
    assert_eq!(card["recall"]["action"], "remove");
    assert_eq!(card["recall"]["name"], "Réunions");
    assert_eq!(card["policy"]["remember"], false);
    let error = f.refused("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"never","decision":{"never":{"scope":"global"}}}));
    assert_eq!(error.status, 400);
    // Annuler laisse la clé et la réunion.
    f.decide(&run, json!("deny"));
    assert_eq!(f.item()["status"], "connected");
    assert_eq!(f.recall.leaves(), 0);
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    assert!(vault.get("REUNIONS_RECALL_KEY").unwrap().is_some());
    // Confirmer quitte la réunion, puis retire la clé.
    let run = f.post("/api/recall/disconnect", json!({}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.recall.leaves(), 1);
    assert_eq!(f.item()["status"], "disconnected");
    assert!(vault.get("REUNIONS_RECALL_KEY").unwrap().is_none());
    assert_eq!(f.item()["meeting"]["label"], "Annulée");
}

#[test]
fn a_bot_asked_in_the_thread_follows_the_same_gate_and_can_be_told_to_leave() {
    let mut f = Fixture::new();
    f.connect();
    // Un lien hors Meet, Zoom et Teams : refus français, ni carte ni appel.
    f.script_call(
        recall::JOIN,
        "join-bad",
        json!({"lien":"https://example.com/x"}),
    );
    f.script_done();
    let run = f.say("bad-link");
    wait("bad link outcome", || {
        store::events(&f.db(), &run, 0)
            .unwrap()
            .iter()
            .any(|e| e.kind == RunEventKind::ToolResult)
    });
    match f.outcome(&run) {
        ToolOutcome::Failed { reason } => {
            assert!(reason.contains("Google Meet, Zoom ou Microsoft Teams"))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.recall.creates(), 0);
    // Un bon lien ouvre la carte ; rien ne part avant Accepter.
    f.script_call(recall::JOIN, "join-ok", json!({"lien":MEET}));
    f.script_done();
    let run = f.say("good-link");
    let request = f.request(&run);
    assert_eq!(f.recall.creates(), 0);
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"accept","decision":"allow_once"}));
    f.app.tick(0);
    wait("join", || f.recall.creates() == 1);
    wait("meeting", || {
        f.db()
            .query_row(
                "SELECT count(*) FROM wrapper_recall_meetings WHERE state='joining'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    // « Quitte la réunion » : sans carte, le bot sort.
    f.script_call(recall::LEAVE, "leave-1", json!({}));
    f.script_done();
    let run = f.say("leave");
    wait("leave", || f.recall.leaves() == 1);
    wait("left", || {
        f.db()
            .query_row(
                "SELECT count(*) FROM wrapper_recall_meetings WHERE state='cancelled'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    assert!(requests::list_for_run(&f.db(), &run).unwrap().is_empty());
    // La clé n’entre dans aucune entrée du journal ni dans le contexte du modèle.
    let journal: String = f
        .db()
        .query_row(
            "SELECT group_concat(payload) FROM engine_run_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!journal.contains(KEY));
    assert!(!format!("{:?}", f.ai.seen.lock().unwrap().bodies).contains(KEY));
}

#[test]
fn a_clean_close_waits_for_an_invitation_still_awaiting_recall() {
    let mut f = Fixture::new();
    f.connect();
    f.recall.seen.lock().unwrap().create_delay = Duration::from_millis(1500);
    let run = f.join(MEET);
    let request = f.request(&run);
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"accept","decision":"allow_once"}));
    f.app.tick(0);
    // La réunion existe (Connexion) mais Recall n'a pas encore répondu : pas d'identifiant de bot.
    wait("meeting row", || {
        f.db().query_row("SELECT count(*) FROM wrapper_recall_meetings WHERE state='joining' AND bot_id IS NULL",[],|r|r.get::<_,i64>(0)).unwrap() == 1
    });
    let left = f.post("/api/recall/shutdown", json!({}));
    assert_eq!(left["left"], 1);
    // Le bot créé après coup est bien retiré, la réunion est Terminée.
    assert_eq!(f.recall.leaves(), 1);
    assert_eq!(f.item()["meeting"]["label"], "Terminée");
}

#[test]
fn an_unknown_state_still_asks_recall_once_to_remove_the_bot() {
    let mut f = Fixture::new();
    f.connect();
    f.joined();
    f.recall.code("nouvel_etat_inconnu");
    f.sync();
    assert_eq!(f.item()["meeting"]["label"], "Échec");
    assert_eq!(f.recall.leaves(), 0);
    f.sync();
    f.sync();
    assert_eq!(f.recall.leaves(), 1);
}

#[test]
fn the_consent_card_shows_the_result_of_its_own_call_not_the_latest_of_the_run() {
    let mut f = Fixture::new();
    f.connect();
    f.script_call(recall::JOIN, "join-own", json!({"lien":MEET}));
    f.script_done();
    let run = f.say("own-result");
    let request = f.request(&run);
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"accept","decision":"allow_once"}));
    f.app.tick(0);
    wait("join", || f.recall.creates() == 1);
    wait("result", || {
        store::events(&f.db(), &run, 0)
            .unwrap()
            .iter()
            .any(|e| e.kind == RunEventKind::ToolResult)
    });
    // Un autre outil du même tour échoue ensuite : la carte de l'invitation ne change pas.
    f.db()
        .execute(
            "INSERT INTO engine_run_events(run_id,seq,kind,payload) SELECT ?1,max(seq)+1,'tool_result',?2 FROM engine_run_events WHERE run_id=?1",
            rusqlite::params![run, json!({"call_id":"another-call","outcome":{"status":"failed","reason":"Autre outil en échec."}}).to_string()],
        )
        .unwrap();
    let state = f.state();
    let card = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap()
        .clone();
    assert_eq!(card["configuration_result"]["status"], "done");
    assert!(card["configuration_result"]["output"]["message"]
        .as_str()
        .unwrap()
        .contains("Le bot rejoint la réunion"));
}

#[test]
fn a_failed_compensating_removal_keeps_the_bot_identity_until_recall_confirms() {
    let mut f = Fixture::new();
    f.connect();
    f.recall.seen.lock().unwrap().create_delay = Duration::from_millis(1500);
    let run = f.join(MEET);
    let request = f.request(&run);
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"accept","decision":"allow_once"}));
    f.app.tick(0);
    wait("meeting row", || {
        f.db().query_row("SELECT count(*) FROM wrapper_recall_meetings WHERE state='joining' AND bot_id IS NULL",[],|r|r.get::<_,i64>(0)).unwrap() == 1
    });
    // Quitter devance la réponse de Recall ; la sortie compensatoire échoue ensuite.
    f.recall.seen.lock().unwrap().leave_status = 500;
    f.post("/api/recall/leave", json!({}));
    assert_eq!(f.item()["meeting"]["label"], "Annulée");
    wait("bot created", || {
        f.db().query_row("SELECT count(*) FROM wrapper_recall_meetings WHERE bot_id='bot-1' AND needs_leave=1",[],|r|r.get::<_,i64>(0)).unwrap() == 1
    });
    f.recall.seen.lock().unwrap().leave_status = 200;
    f.sync();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM wrapper_recall_meetings WHERE needs_leave=0",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(f.recall.leaves(), 2);
}

#[test]
fn disconnecting_first_drains_a_pending_removal_before_the_key_goes() {
    let mut f = Fixture::new();
    f.connect();
    f.joined();
    f.recall.code("nouvel_etat_inconnu");
    f.sync();
    assert_eq!(f.item()["meeting"]["label"], "Échec");
    assert_eq!(f.recall.leaves(), 0);
    let run = f.post("/api/recall/disconnect", json!({}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    // Le suivi de fond peut aussi avoir demandé la sortie : au moins une, jamais zéro.
    assert!(f.recall.leaves() >= 1);
    assert_eq!(f.item()["status"], "disconnected");
    let pending: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM wrapper_recall_meetings WHERE needs_leave=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}

#[test]
fn an_unknown_status_never_counts_as_a_confirmed_removal_before_the_key_goes() {
    let mut f = Fixture::new();
    recall::pause_background(true);
    f.connect();
    f.joined();
    f.recall.code("nouvel_etat_inconnu");
    f.sync();
    assert_eq!(f.item()["meeting"]["label"], "Échec");
    // Recall refuse la sortie et répond encore par un état inconnu : la clé reste.
    f.recall.seen.lock().unwrap().leave_status = 500;
    let run = f.post("/api/recall/disconnect", json!({}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.item()["status"], "connected");
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    assert!(vault.get("REUNIONS_RECALL_KEY").unwrap().is_some());
    let pending: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM wrapper_recall_meetings WHERE needs_leave=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[test]
fn disconnecting_waits_for_an_invitation_still_awaiting_recall() {
    let mut f = Fixture::new();
    recall::pause_background(true);
    f.connect();
    f.recall.seen.lock().unwrap().create_delay = Duration::from_millis(1500);
    let run = f.join(MEET);
    let request = f.request(&run);
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"conversation_id":request.route.target_conversation_id,"submission_id":"accept","decision":"allow_once"}));
    f.app.tick(0);
    wait("meeting row", || {
        f.db().query_row("SELECT count(*) FROM wrapper_recall_meetings WHERE state='joining' AND bot_id IS NULL",[],|r|r.get::<_,i64>(0)).unwrap() == 1
    });
    let remove = f.post("/api/recall/disconnect", json!({}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&remove, json!("allow_once"));
    // Le bot créé pendant l'attente est retiré avant que la clé ne parte.
    assert_eq!(f.recall.leaves(), 1);
    assert_eq!(f.item()["status"], "disconnected");
    assert_eq!(f.item()["meeting"]["label"], "Annulée");
}

#[test]
fn waking_from_sleep_stops_tracking_and_asks_recall_to_remove_the_bot() {
    let mut f = Fixture::new();
    recall::pause_background(true);
    f.connect();
    f.joined();
    f.recall.code("in_call_recording");
    f.sync();
    assert_eq!(f.meeting_state(), "in_call");
    recall::reconcile_wake(&f.app.feature_context());
    let item = f.item();
    assert_eq!(item["meeting"]["label"], "Échec");
    assert_eq!(item["meeting"]["message"], recall::RESUME);
    assert_eq!(item["meeting"]["active"], false);
    f.sync();
    f.sync();
    assert_eq!(f.recall.leaves(), 1);
}

#[test]
fn a_clean_close_also_sends_the_removals_already_owed() {
    let mut f = Fixture::new();
    recall::pause_background(true);
    f.connect();
    f.joined();
    f.recall.code("nouvel_etat_inconnu");
    f.sync();
    assert_eq!(f.item()["meeting"]["label"], "Échec");
    assert_eq!(f.recall.leaves(), 0);
    f.post("/api/recall/shutdown", json!({}));
    assert_eq!(f.recall.leaves(), 1);
    let pending: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM wrapper_recall_meetings WHERE needs_leave=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
}
