//! Photo d’une Page ou d’un site local : sans session d’écran, pour un bot comme
//! pour un sous-bot, rangée comme une capture d’écran. Le rendu est l’image
//! fictive du mode test (une par thème) ; le rendu natif est un autre ticket.
#![cfg(debug_assertions)]
mod support;
use grokbot_delegation::graph::{self, DelegationStatus};
use grokbot_engine::{
    contracts::RunEventKind,
    model::{Run, RunStatus},
    store,
};
use grokbot_wrapper::{ApiResponse, App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const SOURCE: &str = r#"export default function Page() {
  const [offre, setOffre] = useState("Essentiel");
  return (
    <article>
      <h1>Trois forfaits</h1>
      <p>Offre choisie : {offre}</p>
    </article>
  );
}
"#;

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    chat: String,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}})
}
fn completed() -> Value {
    json!({"type":"response.completed"})
}
fn page_id(bot: &str) -> String {
    format!(
        "page_{:x}",
        Sha256::digest(format!("{bot}\0photo.page.tsx").as_bytes())
    )
}
fn write(id: &str) -> Value {
    call(
        id,
        "write_page",
        json!({"name":"photo.page.tsx","title":"Trois forfaits","source":SOURCE}),
    )
}
fn shot(id: &str, target: &str, theme: Option<&str>) -> Value {
    let mut args = json!({"target": target});
    if let Some(theme) = theme {
        args["theme"] = json!(theme);
    }
    call(id, "screenshot_page", args)
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-page-snapshot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("data")).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("data/workspace.sqlite");
        std::fs::write(path.with_extension("chatgpt-auth.json"), json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        support::ask_before_acting(&mut app);
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api("POST", "/api/conversations", br#"{"title":"Photos"}"#)
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .into();
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
    fn queue(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    /// One model answer per tool call, then a closing sentence, for the message `id`.
    fn ask(&mut self, id: &str, calls: Vec<Value>) {
        for c in calls {
            self.queue(vec![c, completed()]);
        }
        self.queue(vec![
            json!({"type":"response.output_text.delta","delta":"C’est fait."}),
            completed(),
        ]);
        self.post(
            "/api/send",
            json!({"conversation_id":self.chat,"message_id":id,"body":"Photographie ma Page"}),
        );
        self.finish(id);
    }
    fn run(&self, id: &str) -> Run {
        let s: String = self.db().query_row(
            "SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",
            [id],
            |r| r.get(0),
        ).unwrap();
        serde_json::from_str(&s).unwrap()
    }
    fn finish(&mut self, id: &str) {
        for _ in 0..500 {
            self.app.tick(now());
            if self.run(id).status.is_terminal() && self.provider.status()["titling"] == false {
                assert_eq!(self.run(id).status, RunStatus::Finished);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("run inachevée {:?}", self.run(id));
    }
    fn outcome(&self, message: &str, call_id: &str) -> Value {
        store::events(&self.db(), &self.run(message).run_id, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == call_id)
            .unwrap()
            .payload["outcome"]
            .clone()
    }
    fn shots(&self) -> i64 {
        self.db()
            .query_row("SELECT count(*) FROM wrapper_screen_shots", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn allow_pending(&mut self) {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        for m in state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["messages"].as_array().unwrap())
            .filter(|m| m["kind"] == "approval" && m["payload"]["status"]["status"] == "pending")
        {
            let entry = m["payload"]["entry_id"].as_str().unwrap();
            self.post(
                "/api/approvals/resolve",
                json!({"entry_id":entry,"submission_id":format!("allow-{entry}"),"conversation_id":m["payload"]["route"]["target_conversation_id"],"decision":"allow_once"}),
            );
        }
    }
    fn png(&mut self, conversation: &str, capture: &str) -> Vec<u8> {
        let path = format!(
            "/api/screen/shot?conversation_id={}&id={}",
            conversation.replace(':', "%3A"),
            capture.replace(':', "%3A")
        );
        match self.app.api_response("GET", &path, b"").unwrap() {
            ApiResponse::Bytes { body, content_type } => {
                assert_eq!(content_type, "image/png");
                assert!(body.starts_with(b"\x89PNG"), "pas un PNG");
                body
            }
            ApiResponse::Json(value) => panic!("image non binaire : {value}"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap().parent().unwrap());
    }
}

#[test]
fn a_page_photo_gives_an_image_and_a_capture_id_usable_in_the_same_thread() {
    let mut f = Fixture::new();
    let id = page_id("grokbot");
    f.ask(
        "photo",
        vec![write("page"), shot("shot", &id, Some("light"))],
    );
    let out = f.outcome("photo", "shot");
    assert_eq!(out["status"], "done", "{out}");
    let capture = out["output"]["capture_id"].as_str().unwrap().to_owned();
    assert_eq!(out["output"]["width"], 1280);
    assert_eq!(out["output"]["height"], 800);
    let chat = f.chat.clone();
    assert!(!f.png(&chat, &capture).is_empty());

    // La même vignette que les captures d’écran, dans la carte d’outil du fil.
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let thumbnails: Vec<_> = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["messages"].as_array().unwrap())
        .filter_map(|m| m["payload"]["screen_image"].as_str())
        .collect();
    assert_eq!(thumbnails.len(), 1, "{thumbnails:?}");
    assert!(thumbnails[0].starts_with("/api/screen/shot?"));

    // Le modèle reçoit l’image au tour suivant, présentée comme une photo de Page.
    let seen = f.server.seen.lock().unwrap();
    let last = seen.bodies.last().unwrap().to_string();
    assert!(last.contains("data:image/png;base64,"), "image absente");
    assert!(last.contains("Photo de la Page ou du site local"));
    assert!(!last.contains("Coordonnées en pixels"));
    drop(seen);

    // Aucune session ni réglage d’écran n’a été nécessaire.
    assert_eq!(state["screen"]["session"], Value::Null);
    assert_eq!(state["screen"]["bots"], json!([]));
}

#[test]
fn an_address_off_this_mac_and_every_lookalike_is_refused_without_a_capture() {
    let mut f = Fixture::new();
    let id = page_id("grokbot");
    let targets = [
        "https://example.com/",
        "file:///Users/charles/.ssh/id_ed25519",
        "data:text/html,<h1>x</h1>",
        "http://user@localhost:3000/",
        "http://localhost.exemple.com/",
        "http://127.0.0.1.nip.io/",
        "http://localhost@evil.example/",
        "http://192.168.1.10:3000/",
    ];
    let mut calls: Vec<_> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| shot(&format!("bad-{i}"), t, None))
        .collect();
    calls.push(shot("unknown", &id, None));
    calls.push(shot("bad-theme", "http://localhost:3000/", Some("sepia")));
    f.ask("refus", calls);
    for (i, target) in targets.iter().enumerate() {
        let out = f.outcome("refus", &format!("bad-{i}"));
        assert_eq!(out["status"], "failed", "{target}");
        let reason = out["reason"].as_str().unwrap();
        assert!(
            reason.contains("localhost") || reason.contains("identifiant"),
            "{target}: {reason}"
        );
    }
    let unknown = f.outcome("refus", "unknown");
    assert_eq!(unknown["reason"], "Cette Page est introuvable.");
    assert_eq!(f.outcome("refus", "bad-theme")["status"], "failed");
    assert_eq!(f.shots(), 0);
}

#[test]
fn a_local_site_is_accepted_and_a_page_that_does_not_compile_is_not_photographed() {
    let mut f = Fixture::new();
    let id = page_id("grokbot");
    f.ask(
        "local",
        vec![
            shot("site", "http://localhost:5173/accueil?x=1", None),
            shot("loopback", "http://[::1]:8080/", Some("dark")),
            call(
                "broken",
                "write_page",
                json!({"name":"photo.page.tsx","title":"Cassée","source":"export default function Page() { return <table><tr><td>x</tr></table>; }"}),
            ),
            shot("unready", &id, None),
        ],
    );
    assert_eq!(f.outcome("local", "site")["status"], "done");
    assert_eq!(f.outcome("local", "loopback")["status"], "done");
    let unready = f.outcome("local", "unready");
    assert_eq!(unready["status"], "failed");
    assert!(unready["reason"]
        .as_str()
        .unwrap()
        .starts_with("Cette Page n’est pas prête"));
    assert_eq!(f.shots(), 2);
}

#[test]
fn light_and_dark_give_two_different_images_and_light_is_the_default() {
    let mut f = Fixture::new();
    let id = page_id("grokbot");
    f.ask(
        "themes",
        vec![
            write("page"),
            shot("clair", &id, Some("light")),
            shot("sombre", &id, Some("dark")),
            shot("defaut", &id, None),
        ],
    );
    let capture = |f: &Fixture, name: &str| {
        f.outcome("themes", name)["output"]["capture_id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (light, dark, default) = (
        capture(&f, "clair"),
        capture(&f, "sombre"),
        capture(&f, "defaut"),
    );
    let chat = f.chat.clone();
    let (light, dark, default) = (
        f.png(&chat, &light),
        f.png(&chat, &dark),
        f.png(&chat, &default),
    );
    assert_ne!(light, dark);
    assert_eq!(light, default);
}

#[test]
fn a_sub_bot_without_a_screen_session_photographs_a_page_in_its_own_thread() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"child","name":"child","color":"citron","description":"Worker","instructions":""}),
    );
    f.queue(vec![
        call(
            "delegate-call",
            "delegate",
            json!({"child_bot":"child","objective":"Photographier la Page","selected_context":{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}}),
        ),
        completed(),
    ]);
    f.post(
        "/api/send",
        json!({"conversation_id":f.chat,"message_id":"parent","body":"Délègue la photo"}),
    );
    // Lancer le sous-bot demande une carte ; la photo, elle, n’en demande aucune.
    let mut allowed = false;
    wait("parent waits for the child", || {
        if f.run("parent").status == RunStatus::AwaitingApproval && !allowed {
            f.allow_pending();
            f.app.tick_suspended(now());
            allowed = true;
        }
        f.run("parent").status == RunStatus::AwaitingChildren
    });
    let edge = graph::descendants(&f.db(), &f.run("parent").run_id)
        .unwrap()
        .remove(0);
    let child_id = page_id("child");
    f.queue(vec![write("child-page"), completed()]);
    f.queue(vec![
        shot("child-shot", &child_id, Some("dark")),
        completed(),
    ]);
    f.queue(vec![
        call(
            "finish-child",
            "complete_task",
            json!({"summary":"Page photographiée"}),
        ),
        completed(),
    ]);
    f.app.tick(now());
    wait("child done", || {
        matches!(
            graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
            DelegationStatus::Done { .. }
        )
    });
    wait("owner released", || f.provider.status()["titling"] == false);

    let child_thread = edge.conversation_id.clone().unwrap();
    let (capture, conversation, run): (String, String, String) = f
        .db()
        .query_row(
            "SELECT id, conversation_id, run_id FROM wrapper_screen_shots",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(conversation, child_thread);
    assert_ne!(conversation, f.chat);
    assert_eq!(
        Some(run),
        graph::get(&f.db(), &edge.id).unwrap().unwrap().child_run_id
    );
    assert!(!f.png(&child_thread, &capture).is_empty());

    // Ni session ni réglage d’écran pour ce sous-bot.
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["screen"]["session"], Value::Null);
    assert_eq!(state["screen"]["bots"], json!([]));
    // La capture du sous-bot ne s’ouvre pas depuis le fil du parent.
    let path = format!(
        "/api/screen/shot?conversation_id={}&id={}",
        f.chat,
        capture.replace(':', "%3A")
    );
    assert!(f.app.api_response("GET", &path, b"").is_err());
}
