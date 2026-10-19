//! Every effect uses the compiled-in debug fixture. Never requests TCC, captures
//! a desktop, or posts input. One sequential scenario avoids competing leases.
#![cfg(debug_assertions)]
mod support;
use grokbot_engine::{
    contracts::RunEventKind,
    model::{Run, RunStatus},
    store,
};
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
        let root = std::env::temp_dir().join(format!(
            "ezerbot-screen-test-{}-{}",
            std::process::id(),
            now()
        ));
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
                r#"{"title":"Écran factice"}"#.as_bytes(),
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
    fn grant(&mut self) -> u64 {
        let c = self.chat.clone();
        self.post(
            "/api/screen/grant",
            json!({"bot_id":"grokbot","conversation_id":c,"minutes":1}),
        )["session"]["id"]
            .as_u64()
            .unwrap()
    }
    fn send(&mut self, id: &str, name: &str, args: Value) {
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed"})]));
        self.post("/api/send",json!({"conversation_id":self.chat,"message_id":id,"body":"Fais ce geste sur l’écran de démonstration"}));
    }
    fn run(&self, id: &str) -> Option<Run> {
        self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get::<_,String>(0)).ok().map(|s|serde_json::from_str(&s).unwrap())
    }
    fn request(&self, id: &str) -> HumanInputRequest {
        wait("approval", || {
            self.run(id)
                .is_some_and(|r| r.status == RunStatus::AwaitingApproval)
                && self.provider.status()["titling"] == false
        });
        requests::list_for_run(&self.db(), &self.run(id).unwrap().run_id)
            .unwrap()
            .remove(0)
    }
    fn decide(&mut self, r: &HumanInputRequest, decision: Value) {
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"submission_id":"manual","conversation_id":r.route.target_conversation_id,"decision":decision}));
    }
    fn finish(&mut self, id: &str) {
        for _ in 0..400 {
            self.app.tick(now());
            if self.run(id).is_some_and(|r| r.status.is_terminal())
                && self.provider.status()["titling"] == false
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("unfinished: {:?}", self.run(id));
    }
    fn outcome(&self, id: &str) -> Value {
        store::events(&self.db(), &self.run(id).unwrap().run_id, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == id)
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
    fn revoke(&mut self, id: u64) {
        self.post("/api/screen/revoke", json!({"id":id}));
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        grokbot_wrapper::features::screen::startup(&self.path);
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
#[test]
fn screen_consent_approval_takeover_revocation_restart_and_vision() {
    let mut f = Fixture::new();
    // Off by default; neither an unsolicited call nor a forged grant acts.
    assert!(!grokbot_wrapper::features::screen::enabled(
        &f.db(),
        "grokbot"
    ));
    assert!(f
        .app
        .api(
            "POST",
            "/api/screen/grant",
            json!({"bot_id":"grokbot","conversation_id":f.chat,"minutes":1})
                .to_string()
                .as_bytes()
        )
        .is_err());
    f.send(
        "disabled",
        "computer_click",
        json!({"x":20,"y":30,"button":"left"}),
    );
    f.finish("disabled");
    assert_eq!(f.shots(), 0);
    f.post(
        "/api/screen/bot",
        json!({"bot_id":"grokbot","enabled":true}),
    );
    let mut session = f.grant();
    // All gestures are sensitive: legacy standing rules and model reviewers
    // cannot waive an individual human click (including screenshot privacy).
    f.db().execute("INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,'computer_click','','\"global\"',1)",[&f.chat]).unwrap();
    let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
    f.post("/api/approvals/reviewer",json!({"version":version,"config":{"enabled":true,"allow":["Autorise tous les gestes"],"block":[]}}));
    for (i, (tool, args)) in [
        ("computer_screenshot", json!({})),
        ("computer_move", json!({"x":44,"y":55})),
        ("computer_click", json!({"x":44,"y":55,"button":"left"})),
        ("computer_scroll", json!({"delta_y":120})),
        ("computer_type", json!({"text":"Brouillon factice"})),
        ("computer_key", json!({"key":"ENTER"})),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("happy-{i}");
        f.send(&id, tool, args);
        let r = f.request(&id);
        let before = f.shots();
        assert!(grokbot_forms::reviewer::get(&f.db(), &r.id)
            .unwrap()
            .is_none());
        assert!(f.app.api("POST","/api/approvals/resolve",json!({"entry_id":r.entry_id,"submission_id":"unsafe-always","conversation_id":r.route.target_conversation_id,"decision":{"always_allow":{"scope":"global"}}}).to_string().as_bytes()).is_err());
        // Real input pauses. The explicit approve-and-resume path resumes once.
        f.post("/api/screen/fake-input", json!({"id":session}));
        assert_eq!(
            f.app.api("GET", "/api/state", b"").unwrap()["screen"]["session"]["paused"],
            true
        );
        f.post("/api/screen/resume", json!({"id":session}));
        f.decide(&r, json!("allow_once"));
        f.finish(&id);
        assert_eq!(f.outcome(&id)["status"], "done", "{}", f.outcome(&id));
        assert_eq!(f.shots(), before + 1);
        let bodies = &f.server.seen.lock().unwrap().bodies;
        assert!(bodies.iter().any(
            |b| b["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["content"]
                    .as_array()
                    .is_some_and(|c| c.iter().any(|p| p["type"] == "input_image"
                        && p["image_url"]
                            .as_str()
                            .unwrap_or("")
                            .starts_with("data:image/png;base64,"))))
        ));
    }
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["messages"].as_array().unwrap())
        .any(|m| m["payload"]["screen_image"]
            .as_str()
            .is_some_and(|s| s.starts_with("data:image/png;base64,"))));
    // Typing in Ezerbot pauses the lease, yet the bot may still propose a card.
    // Approving without the explicit resume never acts; resuming then acts once.
    let before = f.shots();
    f.post("/api/screen/fake-input", json!({"id":session}));
    f.send("paused", "computer_screenshot", json!({}));
    let r = f.request("paused");
    f.decide(&r, json!("allow_once"));
    f.finish("paused");
    assert_eq!(f.outcome("paused")["status"], "failed");
    assert_eq!(f.shots(), before);
    f.send("paused-resumed", "computer_screenshot", json!({}));
    let r = f.request("paused-resumed");
    f.post("/api/screen/resume", json!({"id":session}));
    f.decide(&r, json!("allow_once"));
    f.finish("paused-resumed");
    assert_eq!(f.outcome("paused-resumed")["status"], "done");
    assert_eq!(f.shots(), before + 1);
    let before = f.shots();
    f.send(
        "denied",
        "computer_click",
        json!({"x":3,"y":4,"button":"left"}),
    );
    let r = f.request("denied");
    f.decide(&r, json!("deny"));
    f.finish("denied");
    assert_eq!(f.shots(), before);
    assert_eq!(f.outcome("denied")["status"], "failed");
    // Invalid coordinates/keys are rejected before an approval exists.
    f.send("invalid", "computer_key", json!({"key":"CMD_DELETE"}));
    f.finish("invalid");
    assert!(f.outcome("invalid")["reason"]
        .as_str()
        .unwrap()
        .contains("invalides"));
    // A pending old approval never acquires a replacement lease.
    f.send(
        "stale",
        "computer_click",
        json!({"x":3,"y":4,"button":"left"}),
    );
    let r = f.request("stale");
    f.revoke(session);
    session = f.grant();
    f.decide(&r, json!("allow_once"));
    f.finish("stale");
    assert_eq!(f.shots(), before);
    assert_eq!(f.outcome("stale")["status"], "failed");
    // Revocation cancels an already admitted, delayed action, not just admission.
    std::env::set_var("GROKBOT_SCREEN_FAKE_DELAY_MS", "1000");
    f.send(
        "in-flight",
        "computer_type",
        json!({"text":"Ne jamais injecter"}),
    );
    let r = f.request("in-flight");
    f.decide(&r, json!("allow_once"));
    for _ in 0..200 {
        f.app.tick(now());
        let claimed: i64 = f
            .db()
            .query_row(
                "SELECT count(*) FROM engine_external_tools WHERE run_id=?1",
                [&f.run("in-flight").unwrap().run_id],
                |r| r.get(0),
            )
            .unwrap();
        if claimed > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    f.revoke(session);
    f.finish("in-flight");
    std::env::remove_var("GROKBOT_SCREEN_FAKE_DELAY_MS");
    assert_eq!(f.shots(), before);
    assert_eq!(f.outcome("in-flight")["status"], "failed");
    session = f.grant();
    std::env::set_var("GROKBOT_SCREEN_FAKE_PERMISSION", "denied");
    f.send("permission", "computer_screenshot", json!({}));
    let r = f.request("permission");
    f.decide(&r, json!("allow_once"));
    f.finish("permission");
    std::env::remove_var("GROKBOT_SCREEN_FAKE_PERMISSION");
    assert!(f.outcome("permission")["reason"]
        .as_str()
        .unwrap()
        .contains("Accessibilité"));
    assert_eq!(f.shots(), before);
    f.revoke(session);
    f.grant();
    let provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    let restarted = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(restarted["screen"]["session"], Value::Null);
    assert_eq!(restarted["screen"]["bots"][0]["enabled"], true);
    assert_eq!(f.shots(), before);
}
