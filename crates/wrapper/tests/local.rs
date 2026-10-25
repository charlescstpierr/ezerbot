//! All shell/file effects are confined to these disposable test directories.
mod support;
use grokbot_engine::{
    contracts::RunEventKind,
    model::{Run, RunStatus},
    store,
};
use grokbot_forms::requests::{self, HumanInputRequest};
use grokbot_wrapper::{App, Provider};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    os::unix::process::CommandExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};
struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    chat: String,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-local");
        let root = guard.to_path_buf();
        // Match production layout: app-private data and ordinary working files
        // are distinct, even when a broad grant contains both directories.
        std::fs::create_dir_all(root.join("data")).unwrap();
        let path = root.join("data/workspace.sqlite");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        support::ask_before_acting(&mut app);
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api(
                "POST",
                "/api/conversations",
                br#"{"title":"Outils locaux"}"#,
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
            _dir: guard,
        }
    }
    fn root(&self) -> PathBuf {
        self.path.parent().unwrap().parent().unwrap().to_path_buf()
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
    fn settings(&mut self, shell: bool, files: bool) {
        self.post(
            "/api/local/bot",
            json!({"bot_id":"grokbot","shell":shell,"files":files}),
        );
    }
    fn script(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn send(&mut self, id: &str, name: &str, args: Value) {
        self.script(vec![json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":self.chat,"message_id":id,"body":"Exécute cet outil local"}),
        );
    }
    fn run(&self, id: &str) -> Run {
        let text:String=self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn request(&self, id: &str) -> HumanInputRequest {
        wait("approval and drained owner", || {
            self.run(id).status == RunStatus::AwaitingApproval
                && self.provider.status()["titling"] == false
        });
        requests::list_for_run(&self.db(), &self.run(id).run_id)
            .unwrap()
            .remove(0)
    }
    fn decide(&mut self, r: &HumanInputRequest, decision: Value) {
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"submission_id":"decision","conversation_id":r.route.target_conversation_id,"decision":decision}));
    }
    fn finish(&mut self, id: &str) {
        for _ in 0..300 {
            self.app.tick(now());
            if self.run(id).status.is_terminal() && self.provider.status()["titling"] == false {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("run did not finish: {:?}", self.run(id));
    }
    fn preview(&self, request: &str) -> Value {
        let text: String = self
            .db()
            .query_row(
                "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
                [request],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&text).unwrap()["apercu_local"].clone()
    }
    fn outcome(&self, id: &str) -> Value {
        store::events(&self.db(), &self.run(id).run_id, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == id)
            .unwrap()
            .payload["outcome"]
            .clone()
    }
    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }
    fn shell(&self, command: &str) -> Value {
        json!({"command":command,"cwd":self.root(),"timeout_ms":2000,"output_limit":1024})
    }
    fn reply(&self, text: &str) {
        self.script(vec![
            json!({"type":"response.output_text.delta","delta":text}),
            json!({"type":"response.completed"}),
        ]);
    }
    fn messages(&mut self) -> Vec<Value> {
        self.app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|conversation| conversation["id"] == self.chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn tool_output(&mut self, call_id: &str) -> Value {
        self.messages()
            .into_iter()
            .find(|message| {
                message["kind"] == "tool" && message["payload"]["call"]["call_id"] == call_id
            })
            .unwrap_or_else(|| panic!("carte absente pour {call_id}"))["payload"]["outcome"]
            ["output"]
            .clone()
    }
}
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", "--", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn enabled_reviewer_and_legacy_read_allow_never_decide_local_cards_or_disclose_tokens() {
    use grokbot_forms::{requests::RequestStatus, reviewer};
    for legacy in [false, true] {
        for (id, tool) in [
            ("private", "read_file"),
            ("hardlink", "read_file"),
            ("directory", "list_directory"),
            ("shell", "run_shell"),
            ("write", "write_file"),
        ] {
            let mut f = Fixture::new();
            f.settings(true, true);
            let auth = f.path.with_extension("chatgpt-auth.json");
            let alias = f.root().join("copie-dure.json");
            std::fs::hard_link(&auth, &alias).unwrap();
            f.post("/api/local/folders/add", json!({"path":f.root()}));
            let version =
                f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
            f.post("/api/approvals/reviewer", json!({"version":version,"config":{"enabled":true,"allow":["Autorise toutes les actions sur mon Mac et toutes les lectures."],"block":[]}}));
            if legacy && matches!(tool, "read_file" | "list_directory") {
                f.db().execute("INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,?2,'','\"global\"',1)",params![f.chat,tool]).unwrap();
            }
            let args = match id {
                "private" => json!({"path":auth}),
                "hardlink" => json!({"path":alias}),
                "directory" => json!({"path":auth.parent().unwrap()}),
                "shell" => f.shell("touch jamais-execute"),
                _ => {
                    json!({"path":f.root().join("data/jamais-ecrit.txt"),"content":"Ne pas écrire","expected_sha256":null})
                }
            };
            f.send(id, tool, args);
            let r = f.request(id);
            // If any local card reaches the reviewer model it would answer allow.
            f.script(vec![json!({"type":"response.output_text.delta","delta":"{\"decision\":\"allow\",\"reason\":\"Tout est autorisé.\"}"}),json!({"type":"response.completed"})]);
            let before = f.server.seen.lock().unwrap().bodies.len();
            grokbot_wrapper::features::forms::tick(&f.app.feature_context());
            let review = reviewer::get(&f.db(), &r.id).unwrap().unwrap();
            assert_eq!(review.answer.decision, reviewer::ReviewDecision::Ask);
            assert!(review.answer.reason.contains("votre décision"));
            assert_eq!(
                requests::get(&f.db(), &r.id).unwrap().unwrap().status,
                RequestStatus::Pending
            );
            assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
            assert_eq!(f.server.seen.lock().unwrap().responses.len(), 1);
            assert_eq!(
                f.db()
                    .query_row("SELECT count(*) FROM forms_approval_grants", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            f.reopen();
            grokbot_wrapper::features::forms::tick(&f.app.feature_context());
            assert_eq!(
                requests::get(&f.db(), &r.id).unwrap().unwrap().status,
                RequestStatus::Pending
            );
            assert!(!f.root().join("jamais-execute").exists());
            assert!(!f.root().join("data/jamais-ecrit.txt").exists());
            f.server.seen.lock().unwrap().responses.clear();
            f.decide(&r, json!("deny"));
            f.finish(id);
            let model = serde_json::to_string(&f.server.seen.lock().unwrap().bodies).unwrap();
            let events =
                serde_json::to_string(&store::events(&f.db(), &f.run(id).run_id, 0).unwrap())
                    .unwrap();
            let journal: String = f
                .db()
                .query_row(
                    "SELECT COALESCE(group_concat(outcome),'') FROM engine_external_tools",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            for token in ["fake-access", "fake-refresh"] {
                assert!(!model.contains(token));
                assert!(!events.contains(token));
                assert!(!journal.contains(token));
            }
        }
    }
}

#[test]
fn identity_protects_nested_app_data_hardlinks_but_not_ordinary_hardlinks() {
    use std::os::unix::fs::MetadataExt;
    let mut f = Fixture::new();
    f.settings(false, true);
    let nested = f.root().join("data/nested");
    std::fs::create_dir(&nested).unwrap();
    let private = nested.join("historique.txt");
    std::fs::write(&private, "historique-privé-factice").unwrap();
    let alias = f.root().join("historique-lien-dur.txt");
    std::fs::hard_link(&private, &alias).unwrap();
    let a = std::fs::metadata(&private).unwrap();
    let b = std::fs::metadata(&alias).unwrap();
    assert_eq!((a.dev(), a.ino()), (b.dev(), b.ino()));
    f.post("/api/local/folders/add", json!({"path":f.root()}));
    f.send("private-alias", "read_file", json!({"path":alias}));
    let r = f.request("private-alias");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&r.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&display).unwrap()["apercu_local"]["donnees_ezerbot"],
        true
    );
    f.decide(&r, json!("deny"));
    f.finish("private-alias");
    let plain = f.root().join("ordinaire.txt");
    std::fs::write(&plain, "document ordinaire").unwrap();
    let plain_alias = f.root().join("ordinaire-lien.txt");
    std::fs::hard_link(&plain, &plain_alias).unwrap();
    f.send("plain-alias", "read_file", json!({"path":plain_alias}));
    f.finish("plain-alias");
    assert_eq!(
        f.outcome("plain-alias")["output"]["content"],
        "document ordinaire"
    );
    assert!(
        requests::list_for_run(&f.db(), &f.run("plain-alias").run_id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn write_preview_pins_parent_symlink_destination_and_marks_app_data_per_folder() {
    let mut f = Fixture::new();
    f.settings(false, true);
    let first = f.root().join("premier");
    let second = f.root().join("second");
    let link = f.root().join("projet");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    std::os::unix::fs::symlink(&first, &link).unwrap();
    f.send(
        "create-link",
        "write_file",
        json!({"path":link.join("nouveau.txt"),"content":"test","expected_sha256":null}),
    );
    let r = f.request("create-link");
    assert_eq!(
        f.preview(&r.id)["cible_reelle"],
        json!(first.canonicalize().unwrap().join("nouveau.txt"))
    );
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    f.decide(&r, json!("allow_once"));
    f.finish("create-link");
    assert!(f.outcome("create-link")["reason"]
        .as_str()
        .unwrap()
        .contains("destination réelle a changé"));
    assert!(!first.join("nouveau.txt").exists() && !second.join("nouveau.txt").exists());
    f.send(
        "private-create",
        "write_file",
        json!({"path":f.root().join("data/nouveau.txt"),"content":"test","expected_sha256":null}),
    );
    let r = f.request("private-create");
    assert_eq!(f.preview(&r.id)["donnees_ezerbot"], true);
    f.decide(&r, json!("allow_once"));
    f.finish("private-create");
    assert_eq!(
        std::fs::read_to_string(f.root().join("data/nouveau.txt")).unwrap(),
        "test"
    );
    f.post("/api/local/folders/add", json!({"path":first}));
    let state = f.post("/api/local/folders/add", json!({"path":f.root()}));
    assert_eq!(
        state["folders_with_app_data"],
        json!([f.root().canonicalize().unwrap()])
    );
}

#[test]
fn floating_panel_shell_request_still_waits_for_main_conversation_approval_and_honors_denial() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let args = f.shell("touch panneau-jamais-execute");
    f.script(vec![json!({"type":"response.output_item.done","item":{"id":"fc-panel","type":"function_call","status":"completed","call_id":"panel-approval","name":"run_shell","arguments":args.to_string()}}),json!({"type":"response.completed"})]);
    f.post("/api/floating/send",json!({"conversation_id":f.chat,"message_id":"panel-approval","body":"Commande proposée depuis le panneau"}));
    let request = f.request("panel-approval");
    assert_eq!(request.route.target_conversation_id, f.chat);
    assert!(!f.root().join("panneau-jamais-execute").exists());
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == f.chat)
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "approval"));
    f.decide(&request, json!("deny"));
    f.finish("panel-approval");
    assert!(!f.root().join("panneau-jamais-execute").exists());
}

#[test]
fn floating_panel_provider_failure_is_visible_in_the_shared_transcript() {
    let mut f = Fixture::new();
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            500,
            json!({"error":{"message":"Échec simulé"}}),
        ));
    f.post("/api/floating/send",json!({"conversation_id":f.chat,"message_id":"panel-error","body":"Test d’erreur du panneau"}));
    f.finish("panel-error");
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let messages = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == f.chat)
        .unwrap()["messages"]
        .as_array()
        .unwrap();
    assert!(messages
        .iter()
        .any(|m| m["role"] == "assistant" && m["state"] == "failed"));
    assert_eq!(
        messages.iter().filter(|m| m["id"] == "panel-error").count(),
        1
    );
}

#[test]
fn shell_off_by_default_then_denial_and_allow_once_after_restart() {
    let mut f = Fixture::new();
    let args = f.shell("printf bonjour; printf erreur >&2; exit 7");
    f.send("off", "run_shell", args.clone());
    f.finish("off");
    assert_eq!(f.outcome("off")["status"], "failed");
    assert!(f.outcome("off")["reason"]
        .as_str()
        .unwrap()
        .contains("désactivé pour ce bot"));
    assert!(!f.server.seen.lock().unwrap().bodies[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "run_shell"));
    f.settings(true, false);
    f.post("/api/local/bot", json!({"bot_id":"grokbot","files":true}));
    let settings = f.app.api("GET", "/api/state", b"").unwrap()["local"]["bots"][0].clone();
    assert_eq!(settings["shell"], true);
    assert_eq!(settings["files"], true);
    f.settings(true, false);
    f.send("deny", "run_shell", f.shell("touch denied"));
    let r = f.request("deny");
    assert!(!f.root().join("denied").exists());
    f.decide(&r, json!("deny"));
    f.finish("deny");
    assert!(!f.root().join("denied").exists());
    f.send("ok", "run_shell", args);
    let r = f.request("ok");
    f.reopen();
    f.decide(&r, json!("allow_once"));
    f.finish("ok");
    let output = &f.outcome("ok")["output"];
    assert_eq!(output["stdout"], "bonjour");
    assert_eq!(output["stderr"], "erreur");
    assert_eq!(output["exit_code"], 7);
    f.reopen();
    assert_eq!(f.outcome("ok")["output"], *output);
}
#[test]
fn a_command_approved_after_its_turn_ended_runs_once_and_the_bot_reports_it() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.post(
        "/api/approvals/waiting",
        json!({"task":true,"chat":true,"validity":"24h","reminder":"1h"}),
    );
    let marker = f.root().join("late-marker");
    let command = format!("printf x >> {}", marker.display());
    // The model asks for the command, learns it is waiting, and ends its turn.
    f.script(vec![json!({"type":"response.output_item.done","item":{"id":"fc-late","type":"function_call","status":"completed","call_id":"late","name":"run_shell","arguments":f.shell(&command).to_string()}}),json!({"type":"response.completed"})]);
    f.reply("J’attends son accord.");
    f.post(
        "/api/send",
        json!({"conversation_id":f.chat,"message_id":"late","body":"Exécute cet outil local"}),
    );
    f.finish("late");
    let original = f.run("late");
    assert_eq!(original.status, RunStatus::Finished);
    let card = requests::list_for_run(&f.db(), &original.run_id)
        .unwrap()
        .remove(0);
    assert!(!marker.exists());
    // Charles decides later: the bot is woken in the same conversation and the command runs.
    f.reply("C’est fait.");
    f.decide(&card, json!("allow_once"));
    assert!(!marker.exists());
    let follow: String = f
        .db()
        .query_row(
            "SELECT run_id FROM engine_runs WHERE run_id LIKE 'late-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    f.finish(&follow);
    assert_eq!(f.run(&follow).status, RunStatus::Finished);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    let events = store::events(&f.db(), &follow, 0).unwrap();
    let result = events
        .iter()
        .find(|e| e.kind == RunEventKind::ToolResult)
        .unwrap();
    assert_eq!(result.payload["outcome"]["status"], "done");
    assert_eq!(result.payload["outcome"]["output"]["exit_code"], 0);
    assert_eq!(
        result.payload["authorized_by"],
        json!({"kind":"allow_once","request_id":card.id})
    );
    // The model was told who approved what, and saw the result of the very command.
    let bodies = f.server.seen.lock().unwrap().bodies.clone();
    let body = bodies.last().unwrap().to_string();
    assert!(body.contains("Charles a approuvé : run_shell"), "{body}");
    assert!(body.contains("function_call_output"), "{body}");
    // Another device deciding again, and further ticks, change nothing.
    let again = json!({"entry_id":card.entry_id,"submission_id":"other-device","conversation_id":card.route.target_conversation_id,"decision":"allow_once"});
    assert!(f
        .app
        .api(
            "POST",
            "/api/approvals/resolve",
            again.to_string().as_bytes()
        )
        .is_err());
    for _ in 0..20 {
        f.app.tick(now());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "x");
    let runs: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM engine_runs WHERE run_id LIKE 'late-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 1);
}
#[test]
fn a_voice_message_keeps_its_short_answer_in_the_turn_opened_by_a_late_approval() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.post(
        "/api/approvals/waiting",
        json!({"task":true,"chat":true,"validity":"24h","reminder":"1h"}),
    );
    let command = format!("printf x >> {}", f.root().join("voice-marker").display());
    f.script(vec![json!({"type":"response.output_item.done","item":{"id":"fc-voice","type":"function_call","status":"completed","call_id":"voice","name":"run_shell","arguments":f.shell(&command).to_string()}}),json!({"type":"response.completed"})]);
    f.reply("J’attends son accord.");
    f.post(
        "/api/send",
        json!({"conversation_id":f.chat,"message_id":"voice","body":"Lance cette commande","voice":true}),
    );
    f.finish("voice");
    let card = requests::list_for_run(&f.db(), &f.run("voice").run_id)
        .unwrap()
        .remove(0);
    f.reply("C’est fait.");
    f.decide(&card, json!("allow_once"));
    let follow: String = f
        .db()
        .query_row(
            "SELECT run_id FROM engine_runs WHERE run_id LIKE 'late-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    f.finish(&follow);
    let bodies = f.server.seen.lock().unwrap().bodies.clone();
    let body = bodies.last().unwrap().to_string();
    assert!(body.contains("Charles a approuvé : run_shell"), "{body}");
    assert!(body.contains("une ou deux phrases"), "{body}");
    assert!(body.contains("check_delegations avec portee"), "{body}");
}
#[test]
fn an_app_data_listing_approved_after_its_turn_ended_runs_in_the_follow_up_turn() {
    let mut f = Fixture::new();
    f.settings(false, true);
    f.post(
        "/api/approvals/waiting",
        json!({"task":true,"chat":true,"validity":"24h","reminder":"1h"}),
    );
    let data = f.path.parent().unwrap().to_path_buf();
    f.script(vec![json!({"type":"response.output_item.done","item":{"id":"fc-data","type":"function_call","status":"completed","call_id":"data","name":"list_directory","arguments":json!({"path":data}).to_string()}}),json!({"type":"response.completed"})]);
    f.reply("J’attends son accord.");
    f.post(
        "/api/send",
        json!({"conversation_id":f.chat,"message_id":"data","body":"Liste les données"}),
    );
    f.finish("data");
    let card = requests::list_for_run(&f.db(), &f.run("data").run_id)
        .unwrap()
        .remove(0);
    f.reply("Voilà.");
    f.decide(&card, json!("allow_once"));
    let follow: String = f
        .db()
        .query_row(
            "SELECT run_id FROM engine_runs WHERE run_id LIKE 'late-%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    f.finish(&follow);
    // The data-read guard still wants a human card, and it finds the original one.
    let result = store::events(&f.db(), &follow, 0)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == RunEventKind::ToolResult)
        .unwrap();
    assert_eq!(
        result.payload["outcome"]["status"], "done",
        "{}",
        result.payload
    );
}
#[test]
fn standing_permissions_apply_to_future_commands_and_are_revocable() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("first", "run_shell", f.shell("printf oui"));
    let r = f.request("first");
    f.decide(&r, json!({"always_allow":{"scope":"global"}}));
    f.finish("first");
    f.reopen();
    f.send("second", "run_shell", f.shell("printf encore"));
    f.finish("second");
    assert_eq!(f.outcome("second")["output"]["stdout"], "encore");
    assert!(requests::list_for_run(&f.db(), &f.run("second").run_id)
        .unwrap()
        .is_empty());
    f.post("/api/approvals/rules/revoke",json!({"requesting_bot_id":"grokbot","target_bot_id":"grokbot","tool":"run_shell","args_digest":"","scope":"global"}));
    f.send("third", "run_shell", f.shell("printf attente"));
    let r = f.request("third");
    f.decide(&r, json!("deny"));
    f.finish("third");
}
#[test]
fn files_create_read_list_grants_symlink_escape_and_stale_write() {
    let mut f = Fixture::new();
    f.settings(false, true);
    let file = f.root().join("note.txt");
    f.send(
        "write-denied",
        "write_file",
        json!({"path":file,"content":"Refusé","expected_sha256":null}),
    );
    let denied = f.request("write-denied");
    f.decide(&denied, json!("deny"));
    f.finish("write-denied");
    assert!(!file.exists());
    f.send(
        "write",
        "write_file",
        json!({"path":file,"content":"Bonjour\n","expected_sha256":null}),
    );
    let r = f.request("write");
    assert!(!file.exists());
    f.decide(&r, json!("allow_once"));
    f.finish("write");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Bonjour\n");
    f.send("read-ask", "read_file", json!({"path":file}));
    let r = f.request("read-ask");
    f.decide(&r, json!({"always_allow":{"scope":"global"}}));
    f.finish("read-ask");
    // A legacy broad read permission cannot bypass folder consent.
    f.send("read-still-asks", "read_file", json!({"path":file}));
    let again = f.request("read-still-asks");
    f.decide(&again, json!("deny"));
    f.finish("read-still-asks");
    let mut hash = f.outcome("read-ask")["output"]["sha256"].clone();
    f.post("/api/local/folders/add", json!({"path":f.root()}));
    f.reopen();
    f.send("read-granted", "read_file", json!({"path":file}));
    f.finish("read-granted");
    assert_eq!(f.outcome("read-granted")["output"]["content"], "Bonjour\n");
    assert!(
        requests::list_for_run(&f.db(), &f.run("read-granted").run_id)
            .unwrap()
            .is_empty()
    );
    f.send("ls", "list_directory", json!({"path":f.root()}));
    f.finish("ls");
    assert!(f.outcome("ls")["output"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["name"] == "note.txt"));
    f.send(
        "overwrite",
        "write_file",
        json!({"path":file,"content":"Deuxième version\n","expected_sha256":hash}),
    );
    let overwrite = f.request("overwrite");
    f.decide(&overwrite, json!("allow_once"));
    f.finish("overwrite");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "Deuxième version\n"
    );
    hash = f.outcome("overwrite")["output"]["sha256"].clone();
    f.send(
        "stale",
        "write_file",
        json!({"path":file,"content":"Après\n","expected_sha256":hash}),
    );
    let r = f.request("stale");
    std::fs::write(&file, "Modification indépendante").unwrap();
    f.decide(&r, json!("allow_once"));
    f.finish("stale");
    assert_eq!(f.outcome("stale")["status"], "failed");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "Modification indépendante"
    );
    // A subfolder grant must not follow a symlink to an ungranted sibling.
    let sub = f.root().join("granted");
    std::fs::create_dir(&sub).unwrap();
    std::os::unix::fs::symlink(&file, sub.join("escape")).unwrap();
    f.post(
        "/api/local/folders/remove",
        json!({"path":f.root().canonicalize().unwrap()}),
    );
    f.post("/api/local/folders/add", json!({"path":sub}));
    f.send("escape", "read_file", json!({"path":sub.join("escape")}));
    let r = f.request("escape");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&r.id],
            |row| row.get(0),
        )
        .unwrap();
    let display: Value = serde_json::from_str(&display).unwrap();
    assert_eq!(
        display["apercu_local"]["cible_reelle"],
        json!(file.canonicalize().unwrap())
    );
    assert_eq!(display["apercu_local"]["hors_dossiers_accordes"], true);
    f.decide(&r, json!("deny"));
    f.finish("escape");
    assert_eq!(f.outcome("escape")["status"], "failed");
}
#[test]
fn timeout_output_cap_and_cancel_kill_background_process_group() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("truncate", "run_shell", f.shell("yes x | head -c 8000"));
    let r = f.request("truncate");
    f.decide(&r, json!("allow_once"));
    f.finish("truncate");
    let out = f.outcome("truncate");
    assert_eq!(out["output"]["truncated"], true);
    assert!(out["output"]["stdout"].as_str().unwrap().len() <= 1024);
    let mut args = f.shell("(sleep 1; touch timeout-leak) & wait");
    args["timeout_ms"] = json!(100);
    f.send("timeout", "run_shell", args);
    let r = f.request("timeout");
    f.decide(&r, json!("allow_once"));
    f.finish("timeout");
    assert_eq!(f.outcome("timeout")["output"]["status"], "délai dépassé");
    f.send(
        "cancel",
        "run_shell",
        f.shell("(sleep 1; touch cancel-leak) & echo ready; wait"),
    );
    let r = f.request("cancel");
    f.decide(&r, json!("allow_once"));
    for _ in 0..100 {
        f.app.tick(now());
        let n: i64 = f
            .db()
            .query_row(
                "SELECT count(*) FROM engine_external_tools WHERE run_id=?1",
                [f.run("cancel").run_id],
                |r| r.get(0),
            )
            .unwrap();
        if n > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let start = std::time::Instant::now();
    f.post(
        "/api/stop",
        json!({"conversation_id":f.chat,"message_id":"cancel"}),
    );
    f.finish("cancel");
    assert!(start.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(1100));
    assert!(!f.root().join("cancel-leak").exists());
    assert!(!f.root().join("timeout-leak").exists());
}
#[test]
fn bounded_reads_invalid_utf8_arguments_and_missing_cwd_fail_closed() {
    let mut f = Fixture::new();
    f.settings(true, true);
    f.post("/api/local/folders/add", json!({"path":f.root()}));
    let large = f.root().join("large.txt");
    std::fs::write(&large, vec![b'x'; 1048577]).unwrap();
    let binary = f.root().join("binary.txt");
    std::fs::write(&binary, [0xff]).unwrap();
    for (id, path) in [("large", large), ("binary", binary)] {
        f.send(id, "read_file", json!({"path":path}));
        f.finish(id);
        assert_eq!(f.outcome(id)["status"], "failed");
        assert!(requests::list_for_run(&f.db(), &f.run(id).run_id)
            .unwrap()
            .is_empty());
    }
    f.send("null-run", "run_shell", Value::Null);
    f.finish("null-run");
    assert_eq!(f.outcome("null-run")["status"], "failed");
    f.send("relative", "read_file", json!({"path":"../interdit"}));
    f.finish("relative");
    assert_eq!(f.outcome("relative")["status"], "failed");
    let mut missing = f.shell("printf impossible");
    missing["cwd"] = json!(f.root().join("absent"));
    f.send("missing-cwd", "run_shell", missing);
    let r = f.request("missing-cwd");
    f.decide(&r, json!("allow_once"));
    f.finish("missing-cwd");
    assert_eq!(f.outcome("missing-cwd")["status"], "failed");
    f.send(
        "invalid-output",
        "run_shell",
        f.shell("i=0; while [ \"$i\" -lt 1000 ]; do printf '\\377'; i=$((i+1)); done"),
    );
    let r = f.request("invalid-output");
    f.decide(&r, json!("allow_once"));
    f.finish("invalid-output");
    let output = f.outcome("invalid-output");
    assert!(output["output"]["stdout"].as_str().unwrap().len() <= 1024);
    assert_eq!(output["output"]["truncated"], true);
}

#[test]
fn replacement_diff_uses_granted_bytes_or_an_approved_read_not_an_implicit_read() {
    use sha2::{Digest, Sha256};
    let mut f = Fixture::new();
    f.settings(false, true);
    let path = f.root().join("diff.txt");
    let old = "un\ndeux\ntrois\nquatre\n";
    let new = "un\ndeux\nNOUVEAU\nquatre\n";
    std::fs::write(&path, old).unwrap();
    let hash = format!("{:x}", Sha256::digest(old.as_bytes()));
    let args = json!({"path":path,"content":new,"expected_sha256":hash});
    f.send("implicit", "write_file", args.clone());
    f.finish("implicit");
    assert_eq!(f.outcome("implicit")["status"], "failed");
    assert!(f.outcome("implicit")["reason"]
        .as_str()
        .unwrap()
        .contains("lisez d’abord"));
    assert!(requests::list_for_run(&f.db(), &f.run("implicit").run_id)
        .unwrap()
        .is_empty());
    f.send("approved-read", "read_file", json!({"path":path}));
    let r = f.request("approved-read");
    f.decide(&r, json!("allow_once"));
    f.finish("approved-read");
    f.send("diff", "write_file", args);
    let r = f.request("diff");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&r.id],
            |row| row.get(0),
        )
        .unwrap();
    let display: Value = serde_json::from_str(&display).unwrap();
    let preview = &display["apercu_local"];
    assert_eq!(preview["lignes_ajoutees"], 1);
    assert_eq!(preview["lignes_supprimees"], 1);
    assert_eq!(preview["lignes_proposees"], 4);
    assert_eq!(preview["source"], "lecture_approuvee");
    assert!(preview["extrait"]
        .as_str()
        .unwrap()
        .contains("-trois\n+NOUVEAU\n"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
    f.decide(&r, json!("allow_once"));
    f.finish("diff");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), new);
    f.post("/api/local/folders/add", json!({"path":f.root()}));
    let large = f.root().join("grand.txt");
    let old: String = (1..=300).map(|n| format!("ligne {n}\n")).collect();
    let proposed = old
        .replacen("ligne 1\n", "PREMIÈRE modifiée\n", 1)
        .replace("ligne 300\n", "DERNIÈRE modifiée\n");
    std::fs::write(&large, &old).unwrap();
    f.send("sparse","write_file",json!({"path":large,"content":proposed,"expected_sha256":format!("{:x}",Sha256::digest(old.as_bytes()))}));
    let r = f.request("sparse");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&r.id],
            |row| row.get(0),
        )
        .unwrap();
    let preview: Value = serde_json::from_str::<Value>(&display).unwrap()["apercu_local"].clone();
    assert_eq!(preview["lignes_ajoutees"], 2);
    assert_eq!(preview["lignes_supprimees"], 2);
    assert_eq!(preview["tronque"], false);
    assert!(preview["extrait"]
        .as_str()
        .unwrap()
        .contains("+DERNIÈRE modifiée"));
    f.decide(&r, json!("deny"));
    f.finish("sparse");
    assert_eq!(std::fs::read_to_string(large).unwrap(), old);
}

#[test]
fn app_data_never_inherits_a_broad_folder_or_legacy_read_allow() {
    let mut f = Fixture::new();
    f.settings(false, true);
    let ordinary = f.root().join("document.txt");
    std::fs::write(&ordinary, "document factice").unwrap();
    f.send("legacy-allow", "read_file", json!({"path":ordinary}));
    let r = f.request("legacy-allow");
    f.decide(&r, json!({"always_allow":{"scope":"global"}}));
    f.finish("legacy-allow");
    let state = f.post("/api/local/folders/add", json!({"path":f.root()}));
    assert_eq!(state["contains_app_data"], true);
    let auth = f.path.with_extension("chatgpt-auth.json");
    let alias = f.root().join("alias-identifiants");
    std::os::unix::fs::symlink(&auth, &alias).unwrap();
    for (id, name, path) in [
        ("credential", "read_file", auth),
        ("alias", "read_file", alias),
        (
            "data-list",
            "list_directory",
            f.path.parent().unwrap().to_path_buf(),
        ),
    ] {
        f.send(id, name, json!({"path":path}));
        let r = f.request(id);
        let display: String = f
            .db()
            .query_row(
                "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
                [&r.id],
                |row| row.get(0),
            )
            .unwrap();
        let display: Value = serde_json::from_str(&display).unwrap();
        assert_eq!(display["apercu_local"]["donnees_ezerbot"], true);
        assert_eq!(
            f.db()
                .query_row(
                    "SELECT count(*) FROM engine_external_tools WHERE run_id=?1",
                    [&r.run_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        f.decide(&r, json!("deny"));
        f.finish(id);
        assert_eq!(f.outcome(id)["status"], "failed");
        let events = serde_json::to_string(&store::events(&f.db(), &r.run_id, 0).unwrap()).unwrap();
        assert!(!events.contains("fake-refresh"));
        assert!(!events.contains("fake-access"));
    }
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .all(|v| !v.to_string().contains("fake-refresh")));
    let journal: String = f
        .db()
        .query_row(
            "SELECT COALESCE(group_concat(outcome), '') FROM engine_external_tools",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!journal.contains("fake-refresh"));
}

#[test]
fn legacy_read_denials_win_inside_and_outside_granted_folders() {
    for name in ["read_file", "list_directory"] {
        let mut f = Fixture::new();
        f.settings(false, true);
        let work = f.root().join("travail");
        std::fs::create_dir(&work).unwrap();
        let file = work.join("document.txt");
        std::fs::write(&file, "contenu factice").unwrap();
        let path = if name == "read_file" {
            file
        } else {
            work.clone()
        };
        f.send("rule", name, json!({"path":path}));
        let r = f.request("rule");
        f.decide(&r, json!({"never":{"scope":"global"}}));
        f.finish("rule");
        for (id, grant) in [("outside", false), ("inside", true)] {
            if grant {
                f.post("/api/local/folders/add", json!({"path":work}));
            }
            f.send(id, name, json!({"path":path}));
            f.finish(id);
            assert!(f.outcome(id)["reason"]
                .as_str()
                .unwrap()
                .contains("Une autorisation permanente refuse cette action."));
            assert!(requests::list_for_run(&f.db(), &f.run(id).run_id)
                .unwrap()
                .is_empty());
        }
        if name == "read_file" {
            use sha2::{Digest, Sha256};
            f.send("preview-denied","write_file",json!({"path":path,"content":"modification","expected_sha256":format!("{:x}",Sha256::digest(b"contenu factice"))}));
            f.finish("preview-denied");
            assert!(f.outcome("preview-denied")["reason"]
                .as_str()
                .unwrap()
                .contains("lecture nécessaire à la comparaison est interdite"));
            assert!(
                requests::list_for_run(&f.db(), &f.run("preview-denied").run_id)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(std::fs::read_to_string(path).unwrap(), "contenu factice");
        }
    }
}

#[test]
fn a_missing_target_cannot_become_a_link_and_disabled_pending_reads_explain_why() {
    let mut f = Fixture::new();
    f.settings(false, true);
    let missing = f.root().join("absent.txt");
    let target = f.path.with_extension("chatgpt-auth.json");
    f.send("missing", "read_file", json!({"path":missing}));
    let r = f.request("missing");
    std::os::unix::fs::symlink(&target, &missing).unwrap();
    f.decide(&r, json!("allow_once"));
    f.finish("missing");
    assert!(f.outcome("missing")["reason"]
        .as_str()
        .unwrap()
        .contains("cible réelle a changé"));
    f.send("disabled-pending", "read_file", json!({"path":target}));
    let r = f.request("disabled-pending");
    f.settings(false, false);
    f.decide(&r, json!("allow_once"));
    f.finish("disabled-pending");
    assert!(f.outcome("disabled-pending")["reason"]
        .as_str()
        .unwrap()
        .contains("Cet outil est désactivé"));
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .all(|v| !v.to_string().contains("fake-refresh")));
}

#[test]
fn a_changed_symlink_target_cannot_reuse_an_individual_read_approval() {
    let mut f = Fixture::new();
    f.settings(false, true);
    let first = f.root().join("premier.txt");
    let second = f.root().join("second.txt");
    let link = f.root().join("lien.txt");
    std::fs::write(&first, "première cible").unwrap();
    std::fs::write(&second, "autre cible").unwrap();
    std::os::unix::fs::symlink(&first, &link).unwrap();
    f.send("changing-link", "read_file", json!({"path":link}));
    let r = f.request("changing-link");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    f.decide(&r, json!("allow_once"));
    f.finish("changing-link");
    assert_eq!(f.outcome("changing-link")["status"], "failed");
    assert!(f.outcome("changing-link")["reason"]
        .as_str()
        .unwrap()
        .contains("cible réelle a changé"));
}

#[test]
fn creation_line_count_permissions_and_shared_output_quota_match_the_preview() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    f.settings(true, true);
    let control = f.root().join("mode-reference");
    std::fs::write(&control, "").unwrap();
    let path = f.root().join("empty.txt");
    f.send(
        "empty",
        "write_file",
        json!({"path":path,"content":"","expected_sha256":null}),
    );
    let r = f.request("empty");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&r.id],
            |row| row.get(0),
        )
        .unwrap();
    let display: Value = serde_json::from_str(&display).unwrap();
    assert_eq!(display["apercu_local"]["lignes_proposees"], 0);
    f.decide(&r, json!("allow_once"));
    f.finish("empty");
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        std::fs::metadata(control).unwrap().permissions().mode() & 0o777
    );
    let mut args = f.shell("printf '%03001d' 0");
    args["output_limit"] = json!(4096);
    f.send("one-stream", "run_shell", args);
    let r = f.request("one-stream");
    f.decide(&r, json!("allow_once"));
    f.finish("one-stream");
    assert_eq!(
        f.outcome("one-stream")["output"]["stdout"]
            .as_str()
            .unwrap()
            .len(),
        3001
    );
    assert_eq!(f.outcome("one-stream")["output"]["truncated"], false);
    let mut args = f.shell("printf '%03000d' 0; printf '%03000d' 0 >&2");
    args["output_limit"] = json!(4096);
    f.send("two-streams", "run_shell", args);
    let r = f.request("two-streams");
    f.decide(&r, json!("allow_once"));
    f.finish("two-streams");
    let output = f.outcome("two-streams");
    assert_eq!(
        output["output"]["stdout"].as_str().unwrap().len()
            + output["output"]["stderr"].as_str().unwrap().len(),
        4096
    );
    assert_eq!(output["output"]["truncated"], true);
}

#[test]
fn concurrent_approved_creates_and_overwrites_do_not_lose_updates() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    f.settings(false, true);
    f.post("/api/local/folders/add", json!({"path":f.root()}));
    let path = f.root().join("concurrent.txt");
    for (id, content) in [("create-a", "A"), ("create-b", "B")] {
        f.send(
            id,
            "write_file",
            json!({"path":path,"content":content,"expected_sha256":null}),
        );
        let r = f.request(id);
        f.decide(&r, json!("allow_once"));
    }
    f.finish("create-a");
    f.finish("create-b");
    let done = ["create-a", "create-b"]
        .iter()
        .filter(|id| f.outcome(id)["status"] == "done")
        .count();
    assert_eq!(done, 1);
    let bytes = std::fs::read(&path).unwrap();
    let hash = format!("{:x}", Sha256::digest(bytes));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    for (id, content) in [("overwrite-a", "C"), ("overwrite-b", "D")] {
        f.send(
            id,
            "write_file",
            json!({"path":path,"content":content,"expected_sha256":hash}),
        );
        let r = f.request(id);
        f.decide(&r, json!("allow_once"));
    }
    f.finish("overwrite-a");
    f.finish("overwrite-b");
    let done = ["overwrite-a", "overwrite-b"]
        .iter()
        .filter(|id| f.outcome(id)["status"] == "done")
        .count();
    assert_eq!(done, 1);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(std::fs::read_dir(f.root()).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn timeout_does_not_wait_forever_for_an_escaped_inherited_output_descriptor() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let mut a=f.shell("/usr/bin/python3 -c 'import os,time; os.setsid(); print(\"détaché\",flush=True); time.sleep(3)' & wait");
    a["timeout_ms"] = json!(500);
    f.send("escaped-pipe", "run_shell", a);
    let r = f.request("escaped-pipe");
    f.decide(&r, json!("allow_once"));
    let start = std::time::Instant::now();
    f.finish("escaped-pipe");
    assert!(start.elapsed() < Duration::from_millis(1500));
    assert_eq!(
        f.outcome("escaped-pipe")["output"]["status"],
        "délai dépassé"
    );
    // Deliberately escaped groups are not a shell sandbox. This owned fixture
    // exits by itself; wait for it so the test never leaves a child behind.
    std::thread::sleep(Duration::from_millis(3100));
}

#[test]
fn incomplete_external_admission_is_not_reexecuted_after_restart() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("crash", "run_shell", f.shell("touch duplicate"));
    let r = f.request("crash");
    // Simulate crash after a committed admission, before a durable result.
    f.db().execute("INSERT INTO engine_external_tools(run_id,call_id,authorized_by) VALUES (?1,'crash','null')",[&r.run_id]).unwrap();
    f.reopen();
    f.decide(&r, json!("allow_once"));
    f.finish("crash");
    assert!(!f.root().join("duplicate").exists());
    assert!(f.outcome("crash")["reason"]
        .as_str()
        .unwrap()
        .contains("Résultat inconnu"));
}
#[test]
fn file_error_and_bot_isolation_are_per_call_failures() {
    let mut f = Fixture::new();
    f.settings(false, true);
    f.post("/api/bots",json!({"id":"other","name":"Autre bot","color":"citron","description":"Test local","instructions":""}));
    assert!(!grokbot_wrapper::features::local::enabled(
        &f.db(),
        "other",
        "read_file"
    ));
    f.post("/api/local/bot", json!({"bot_id":"other","shell":true}));
    assert!(!grokbot_wrapper::features::local::enabled(
        &f.db(),
        "grokbot",
        "run_shell"
    ));
    assert!(grokbot_wrapper::features::local::enabled(
        &f.db(),
        "grokbot",
        "read_file"
    ));
    f.send(
        "missing",
        "read_file",
        json!({"path":f.root().join("absent")}),
    );
    let r = f.request("missing");
    f.decide(&r, json!("allow_once"));
    f.finish("missing");
    assert_eq!(f.outcome("missing")["status"], "failed");
    assert!(f
        .app
        .api(
            "POST",
            "/api/local/bot",
            br#"{"bot_id":"absent","shell":true,"files":true}"#
        )
        .is_err());
    let active: bool = f
        .db()
        .query_row(
            "SELECT shell FROM wrapper_local_bots WHERE bot_id='grokbot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!active);
    f.db()
        .execute(
            "UPDATE wrapper_local_bots SET files=0 WHERE bot_id='grokbot'",
            params![],
        )
        .unwrap();
    f.send("disabled", "read_file", json!({"path":f.path}));
    f.finish("disabled");
    assert_eq!(f.outcome("disabled")["status"], "failed");
}

#[test]
fn background_shell_answers_before_the_command_finishes_then_the_card_shows_the_code() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let marker = f.root().join("fini.txt");
    let mut args = f.shell("sleep 2 && echo fini && touch fini.txt");
    args["background"] = json!(true);
    args["timeout_ms"] = json!(100);
    f.send("long", "run_shell", args);
    let request = f.request("long");
    assert!(f
        .messages()
        .iter()
        .any(|message| message["kind"] == "approval"));
    f.reply("bonjour");
    let started = std::time::Instant::now();
    f.decide(&request, json!("allow_once"));
    loop {
        f.app.tick(now());
        let replied = f.messages().iter().any(|message| {
            message["role"] == "assistant"
                && message["body"].as_str().unwrap_or("").contains("bonjour")
        });
        if replied {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "le tour est resté bloqué par la commande"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !marker.exists(),
        "la commande a fini avant la réponse du bot"
    );
    assert_eq!(f.tool_output("long")["status"], "en cours");
    assert!(f.tool_output("long")["exit_code"].is_null());
    f.finish("long");
    assert_eq!(f.outcome("long")["output"]["status"], "en cours");
    let mut output = Value::Null;
    for _ in 0..150 {
        output = f.tool_output("long");
        if output["status"] == "terminé" {
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    assert_eq!(output["status"], "terminé");
    assert_eq!(output["exit_code"], json!(0));
    assert!(output["stdout"].as_str().unwrap_or("").contains("fini"));
    assert!(marker.exists());
}

#[test]
fn stop_kills_only_that_group_and_quitting_marks_the_other_stopped() {
    let mut f = Fixture::new();
    f.settings(true, false);
    for (id, file) in [("one", "one"), ("two", "two")] {
        let mut args = f.shell(&format!(
            "echo $$ > {file}.pid; sleep 12; touch {file}.done"
        ));
        args["background"] = json!(true);
        args["timeout_ms"] = json!(100);
        f.send(id, "run_shell", args);
        let request = f.request(id);
        f.reply("ok");
        f.decide(&request, json!("allow_once"));
        f.finish(id);
    }
    let read_pid = |f: &Fixture, name: &str| -> u32 {
        let path = f.root().join(format!("{name}.pid"));
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(pid) = text.trim().parse() {
                    return pid;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("pid absent : {name}");
    };
    let one = read_pid(&f, "one");
    let two = read_pid(&f, "two");
    assert!(process_alive(one) && process_alive(two));
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("one").run_id, "call_id": "one"}),
    );
    for _ in 0..100 {
        if !process_alive(one) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!process_alive(one));
    assert!(process_alive(two));
    let mut stopped = Value::Null;
    for _ in 0..100 {
        stopped = f.tool_output("one");
        if stopped["status"] == "arrêtée" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(stopped["status"], "arrêtée");
    assert!(stopped["exit_code"].is_null());
    assert_eq!(f.tool_output("two")["status"], "en cours");
    f.reopen();
    for _ in 0..150 {
        if !process_alive(two) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!process_alive(two), "quitter l’app laisse le processus");
    let mut other = Value::Null;
    for _ in 0..50 {
        other = f.tool_output("two");
        if other["status"] == "arrêtée" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(other["status"], "arrêtée");
    std::thread::sleep(Duration::from_millis(400));
    assert!(!f.root().join("one.done").exists());
    assert!(!f.root().join("two.done").exists());
    assert_eq!(f.tool_output("one")["status"], "arrêtée");
    assert_eq!(f.tool_output("two")["status"], "arrêtée");
}

fn start_background(f: &mut Fixture, id: &str, command: &str) {
    let mut args = f.shell(command);
    args["background"] = json!(true);
    args["timeout_ms"] = json!(100);
    f.send(id, "run_shell", args);
    let request = f.request(id);
    f.reply("ok");
    f.decide(&request, json!("allow_once"));
    f.finish(id);
}

fn task_input(f: &mut Fixture, id: &str, line: &str) -> Result<Value, grokbot_wrapper::ApiError> {
    let run_id = f.run(id).run_id;
    f.app.api(
        "POST",
        "/api/local/tasks/input",
        json!({"run_id": run_id, "call_id": id, "line": line})
            .to_string()
            .as_bytes(),
    )
}

fn wait_output(f: &mut Fixture, id: &str, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..200 {
        let output = f.tool_output(id);
        if done(&output) {
            return output;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("{what} : {}", f.tool_output(id));
}

#[test]
fn a_line_sent_to_a_running_task_is_read_by_the_process_and_shown_in_its_output() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(
        &mut f,
        "ask",
        "read -r answer; printf '%s' \"$answer\" > lu.txt; echo \"recu=$answer\"",
    );
    assert_eq!(f.tool_output("ask")["status"], "en cours");
    assert!(!f.root().join("lu.txt").exists());
    let reply = task_input(&mut f, "ask", "oui").unwrap();
    assert_eq!(reply["status"], "envoyée");
    let output = wait_output(&mut f, "ask", "la tâche n’a pas fini", |o| {
        o["status"] == "terminé"
    });
    assert_eq!(
        std::fs::read_to_string(f.root().join("lu.txt")).unwrap(),
        "oui",
        "le processus n’a pas lu la ligne envoyée"
    );
    let stdout = output["stdout"].as_str().unwrap();
    assert!(stdout.contains("recu=oui"), "{stdout:?}");
    assert!(
        stdout.lines().any(|line| line == "oui"),
        "la ligne envoyée n’est pas recopiée dans la sortie : {stdout:?}"
    );
    assert_eq!(output["exit_code"], json!(0));
}

#[test]
fn a_finished_or_stopped_task_refuses_a_line_in_french_and_its_output_does_not_change() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(&mut f, "done", "read -r a; echo \"recu=$a\"");
    task_input(&mut f, "done", "un").unwrap();
    let finished = wait_output(&mut f, "done", "la tâche n’a pas fini", |o| {
        o["status"] == "terminé"
    });
    let error = task_input(&mut f, "done", "deux").unwrap_err();
    assert_eq!(error.status, 409);
    assert!(
        error.message.contains("n’est plus en cours")
            && error.message.contains("n’a pas été envoyée"),
        "{}",
        error.message
    );
    start_background(&mut f, "halt", "echo debut; cat; echo fin");
    wait_output(&mut f, "halt", "pas de sortie", |o| {
        o["stdout"].as_str().unwrap_or("").contains("debut")
    });
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("halt").run_id, "call_id": "halt"}),
    );
    wait_output(&mut f, "halt", "pas arrêtée", |o| {
        o["status"] == "arrêtée"
    });
    // The worker still publishes the drained output once after the stop.
    std::thread::sleep(Duration::from_millis(600));
    let stopped = f.tool_output("halt");
    let error = task_input(&mut f, "halt", "trop tard").unwrap_err();
    assert_eq!(error.status, 409);
    assert!(
        error.message.contains("n’est plus en cours"),
        "{}",
        error.message
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(f.tool_output("done"), finished);
    assert_eq!(f.tool_output("halt"), stopped);
    assert!(!finished["stdout"].as_str().unwrap().contains("deux"));
    assert!(!stopped["stdout"].as_str().unwrap().contains("trop tard"));
}

#[test]
fn a_password_prompt_receives_no_byte_and_the_refusal_names_the_prompt() {
    let mut f = Fixture::new();
    f.settings(true, false);
    for (id, prompt) in [
        ("pw1", "printf 'Mot de passe : '"),
        ("pw2", "printf 'Enter passphrase for key: ' >&2"),
    ] {
        start_background(
            &mut f,
            id,
            &format!("{prompt}; read -r secret; printf '%s' \"$secret\" > {id}.txt; echo fin"),
        );
        let sent = format!("hunter-{id}");
        wait_output(&mut f, id, "l’invite n’est pas affichée", |o| {
            let text = format!(
                "{}{}",
                o["stdout"].as_str().unwrap_or(""),
                o["stderr"].as_str().unwrap_or("")
            );
            text.contains("Mot de passe") || text.contains("passphrase")
        });
        let error = task_input(&mut f, id, &sent).unwrap_err();
        assert_eq!(error.status, 409);
        assert!(error.message.contains("mot de passe"), "{}", error.message);
        assert!(
            error.message.contains("n’a pas été envoyée"),
            "{}",
            error.message
        );
        std::thread::sleep(Duration::from_millis(400));
        let output = f.tool_output(id);
        assert_eq!(
            output["status"], "en cours",
            "le programme a reçu des octets"
        );
        assert!(!f.root().join(format!("{id}.txt")).exists());
        assert!(!output["stdout"].as_str().unwrap().contains(&sent));
        assert!(!output["stderr"].as_str().unwrap().contains(&sent));
        f.post(
            "/api/local/tasks/stop",
            json!({"run_id": f.run(id).run_id, "call_id": id}),
        );
    }
}

#[test]
fn a_prompt_that_is_not_for_a_password_still_receives_the_line_and_one_line_at_a_time() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(
        &mut f,
        "yn",
        "printf 'Continuer ? [y/N] '; read -r a; echo \"reponse=$a\"",
    );
    wait_output(&mut f, "yn", "l’invite n’est pas affichée", |o| {
        o["stdout"].as_str().unwrap_or("").contains("[y/N]")
    });
    for refused in ["a\nb", "a\rb", ""] {
        let error = task_input(&mut f, "yn", refused);
        if refused.is_empty() {
            assert!(error.is_ok(), "une ligne vide est une réponse valide");
            break;
        }
        assert_eq!(error.unwrap_err().status, 400);
    }
    let output = wait_output(&mut f, "yn", "la tâche n’a pas fini", |o| {
        o["status"] == "terminé"
    });
    assert!(output["stdout"].as_str().unwrap().contains("reponse="));
}

#[test]
fn the_echo_of_a_line_counts_against_the_output_cap_and_the_card_says_so() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(&mut f, "cap", "cat");
    let line = "x".repeat(100);
    for _ in 0..12 {
        task_input(&mut f, "cap", &line).unwrap();
    }
    let output = wait_output(&mut f, "cap", "la coupure n’est pas signalée", |o| {
        o["truncated"] == json!(true)
    });
    assert!(output["stdout"].as_str().unwrap().len() <= 1024);
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("cap").run_id, "call_id": "cap"}),
    );
}

#[test]
fn a_password_prompt_ending_with_a_newline_or_following_an_echo_still_refuses_the_secret() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(
        &mut f,
        "nl",
        "echo 'Password:'; read -r secret; printf '%s' \"$secret\" > nl.txt; echo fin",
    );
    wait_output(&mut f, "nl", "l’invite n’est pas affichée", |o| {
        o["stdout"].as_str().unwrap_or("").contains("Password:")
    });
    let error = task_input(&mut f, "nl", "hunter-nl").unwrap_err();
    assert_eq!(error.status, 409, "{}", error.message);
    assert!(error.message.contains("mot de passe"), "{}", error.message);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!f.root().join("nl.txt").exists());
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("nl").run_id, "call_id": "nl"}),
    );
    // The echo of an ordinary answer must not hide the next prompt on stderr.
    for round in 0..5 {
        let id = format!("echo{round}");
        start_background(
            &mut f,
            &id,
            "read -r user; printf 'Password: ' >&2; read -r secret; printf '%s' \"$secret\" > secret.txt; echo fin",
        );
        task_input(&mut f, &id, "bob").unwrap();
        wait_output(&mut f, &id, "l’invite n’est pas affichée", |o| {
            o["stderr"].as_str().unwrap_or("").contains("Password:")
        });
        std::thread::sleep(Duration::from_millis(200));
        let error = task_input(&mut f, &id, "hunter-echo").unwrap_err();
        assert_eq!(error.status, 409, "{}", error.message);
        assert!(!f.root().join("secret.txt").exists());
        f.post(
            "/api/local/tasks/stop",
            json!({"run_id": f.run(&id).run_id, "call_id": id}),
        );
    }
}

#[test]
fn a_stopped_task_is_killed_before_its_input_closes() {
    // An input closed before the kill is an end of file: `read` returns and the
    // rest of the command runs although the task was stopped.
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(&mut f, "eof", "read -r ligne; printf fin > apres.txt");
    let root = f.root();
    // Destroying the App raises the monitor's stop flag and nothing else: the
    // monitor alone decides what the program sees first.
    let Fixture { app, _dir, .. } = f;
    drop(app);
    assert!(
        !root.join("apres.txt").exists(),
        "la tâche arrêtée a continué après la fin de son entrée"
    );
}

#[test]
fn a_stopped_task_stays_stopped_when_no_process_can_start() {
    // The process limit is process-wide and macOS does not let it rise again:
    // the test runs again alone in a child process, which ends with it.
    const CHILD: &str = "EZERBOT_STOP_WITHOUT_SPAWN";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a_stopped_task_stays_stopped_when_no_process_can_start",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(&mut f, "nproc", "read -r ligne; printf fin > apres.txt");
    let limit = getrlimit(Resource::Nproc);
    setrlimit(
        Resource::Nproc,
        Rlimit {
            current: Some(0),
            maximum: limit.maximum,
        },
    )
    .unwrap();
    let refused = std::process::Command::new("/bin/echo").output().is_err();
    let stopped = f.app.api(
        "POST",
        "/api/local/tasks/stop",
        json!({"run_id": f.run("nproc").run_id, "call_id": "nproc"})
            .to_string()
            .as_bytes(),
    );
    // Destroying the App joins the task's monitor while the limit holds: its
    // kill and the closing of the input are both behind us.
    let root = f.root();
    let Fixture { app, _dir, .. } = f;
    drop(app);
    assert!(refused, "la limite de processus n’a pas pris");
    assert_eq!(stopped.unwrap()["status"], "arrêtée");
    assert!(
        !root.join("apres.txt").exists(),
        "la tâche arrêtée a continué après la fin de son entrée"
    );
}

#[test]
fn a_password_prompt_written_one_byte_at_a_time_still_refuses_the_secret() {
    let mut f = Fixture::new();
    f.settings(true, false);
    // The program pauses between bytes, so the host reads the prompt in as many
    // pieces as it has bytes: every cut point arrives on its own.
    let bytewise = "for c in P a s s w o r d :; do printf %s \"$c\"; sleep 0.02; done";
    for (id, command, stream) in [
        (
            "octets",
            format!("{bytewise}; echo; read -r secret; printf '%s' \"$secret\" > octets.txt"),
            "stdout",
        ),
        (
            "octets-echo",
            format!("read -r user; {{ {bytewise}; printf ' '; }} >&2; read -r secret; printf '%s' \"$secret\" > octets-echo.txt"),
            "stderr",
        ),
    ] {
        start_background(&mut f, id, &command);
        if stream == "stderr" {
            task_input(&mut f, id, "bob").unwrap();
        }
        wait_output(&mut f, id, "l’invite n’est pas affichée", |o| {
            o[stream].as_str().unwrap_or("").contains("Password:")
        });
        let error = task_input(&mut f, id, "hunter-octets").unwrap_err();
        assert_eq!(error.status, 409, "{id} : {}", error.message);
        assert!(error.message.contains("mot de passe"), "{id} : {}", error.message);
        f.post(
            "/api/local/tasks/stop",
            json!({"run_id": f.run(id).run_id, "call_id": id}),
        );
        assert!(!f.root().join(format!("{id}.txt")).exists(), "{id}");
    }
}

#[test]
fn a_password_prompt_on_one_stream_stays_refused_while_the_other_stream_writes() {
    let mut f = Fixture::new();
    f.settings(true, false);
    for (id, command) in [
        (
            "blank",
            "printf 'Password: ' >&2; sleep 1; printf '\\n'; read -r secret; printf '%s' \"$secret\" > blank.txt",
        ),
        (
            "other",
            "printf 'Password: ' >&2; echo travail; read -r secret; printf '%s' \"$secret\" > other.txt",
        ),
    ] {
        start_background(&mut f, id, command);
        wait_output(&mut f, id, "l’invite n’est pas affichée", |o| {
            o["stderr"].as_str().unwrap_or("").contains("Password:")
        });
        std::thread::sleep(Duration::from_millis(1500));
        let error = task_input(&mut f, id, "hunter-stream").unwrap_err();
        assert_eq!(error.status, 409, "{id} : {}", error.message);
        std::thread::sleep(Duration::from_millis(300));
        assert!(!f.root().join(format!("{id}.txt")).exists());
        f.post(
            "/api/local/tasks/stop",
            json!({"run_id": f.run(id).run_id, "call_id": id}),
        );
    }
}

#[test]
fn a_long_password_prompt_still_refuses_the_secret() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let long = "dossier/".repeat(900);
    start_background(
        &mut f,
        "long",
        &format!(
            "printf \"Enter passphrase for key '/Users/x/{long}id': \"; read -r secret; printf '%s' \"$secret\" > long.txt"
        ),
    );
    // The output cap (1 Kio) cuts the prompt well before its end.
    wait_output(&mut f, "long", "l’invite n’est pas arrivée", |o| {
        o["truncated"] == json!(true)
    });
    std::thread::sleep(Duration::from_millis(300));
    let error = task_input(&mut f, "long", "hunter-long").unwrap_err();
    assert_eq!(error.status, 409, "{}", error.message);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!f.root().join("long.txt").exists());
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("long").run_id, "call_id": "long"}),
    );
}

#[test]
fn an_accepted_line_still_shows_when_the_program_closed_its_stdout() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(
        &mut f,
        "closed",
        "exec 1>&-; read -r answer; printf '%s' \"$answer\" > lu.txt; sleep 3",
    );
    std::thread::sleep(Duration::from_millis(300));
    task_input(&mut f, "closed", "oui").unwrap();
    let output = wait_output(
        &mut f,
        "closed",
        "la ligne n’est pas dans la sortie",
        |o| o["stdout"].as_str().unwrap_or("").contains("oui"),
    );
    assert_eq!(output["status"], "en cours");
    assert_eq!(
        std::fs::read_to_string(f.root().join("lu.txt")).unwrap_or_default(),
        "oui"
    );
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("closed").run_id, "call_id": "closed"}),
    );
}

#[test]
fn foreground_shell_keeps_its_timeout_ceiling_and_background_shows_a_nonzero_code() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let mut args = f.shell("touch trop-long");
    args["timeout_ms"] = json!(120_001);
    f.send("ceiling", "run_shell", args);
    f.reply("non");
    f.finish("ceiling");
    assert_eq!(f.outcome("ceiling")["status"], "failed");
    assert!(f.outcome("ceiling")["reason"]
        .as_str()
        .unwrap_or("")
        .contains("120"));
    assert!(requests::list_for_run(&f.db(), &f.run("ceiling").run_id)
        .unwrap()
        .is_empty());
    assert!(!f.root().join("trop-long").exists());
    let mut args = f.shell("echo raté >&2; exit 3");
    args["background"] = json!(true);
    args["timeout_ms"] = json!(120_001);
    f.send("code", "run_shell", args);
    let request = f.request("code");
    f.reply("vu");
    f.decide(&request, json!("allow_once"));
    f.finish("code");
    let mut output = Value::Null;
    for _ in 0..80 {
        output = f.tool_output("code");
        if output["status"] == "échec" {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(output["status"], "échec");
    assert_eq!(output["exit_code"], json!(3));
    assert!(output["stderr"].as_str().unwrap_or("").contains("raté"));
}

#[test]
fn background_output_cap_is_marked_on_the_card() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let mut args = f.shell("yes x | head -c 8000");
    args["background"] = json!(true);
    args["output_limit"] = json!(1024);
    f.send("cap", "run_shell", args);
    let request = f.request("cap");
    f.reply("vu");
    f.decide(&request, json!("allow_once"));
    f.finish("cap");
    let mut output = Value::Null;
    for _ in 0..100 {
        output = f.tool_output("cap");
        if output["truncated"] == true {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(output["truncated"], true);
    assert!(output["stdout"].as_str().unwrap_or("").len() <= 1024);
}

#[test]
fn crash_while_a_background_command_is_starting_recovers_on_the_next_launch() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("seed", "run_shell", f.shell("echo seed"));
    let request = f.request("seed");
    f.reply("ok");
    f.decide(&request, json!("allow_once"));
    f.finish("seed");
    let run_id = f.run("seed").run_id.clone();
    let marker = f.root().join("orphan.done");
    let token = "ezerbot-shell-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut child = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            &format!("sleep 30; touch {}", marker.display()),
            token,
        ])
        .current_dir(f.root())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pgid = child.id();
    let started =
        grokbot_wrapper::features::local::shell_leader_started(pgid).expect("identité du chef");
    f.db()
        .execute(
            "INSERT INTO engine_external_tools(run_id,call_id,authorized_by,outcome) VALUES (?1,'crash','null',NULL)",
            [&run_id],
        )
        .unwrap();
    f.db()
        .execute(
            "INSERT INTO wrapper_shell_tasks(run_id,call_id,status,exit_code,pgid,leader,started) VALUES (?1,'crash','en cours',NULL,?2,?3,?4)",
            params![run_id, pgid, token, started],
        )
        .unwrap();
    grokbot_wrapper::features::local::startup(&f.db()).unwrap();
    let mut reaped = false;
    for _ in 0..100 {
        if child.try_wait().unwrap().is_some() {
            reaped = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(reaped, "le processus orphelin tourne encore");
    assert!(!marker.exists(), "la commande interrompue a continué");
    let (status, exit_code): (String, Option<i64>) = f
        .db()
        .query_row(
            "SELECT status, exit_code FROM wrapper_shell_tasks WHERE call_id='crash'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "arrêtée");
    assert!(exit_code.is_none());
    let outcome: String = f
        .db()
        .query_row(
            "SELECT outcome FROM engine_external_tools WHERE call_id='crash'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let outcome: Value = serde_json::from_str(&outcome).unwrap();
    assert_eq!(outcome["output"]["status"], "arrêtée");
    assert!(outcome["output"]["exit_code"].is_null());
    f.reopen();
    grokbot_wrapper::features::local::startup(&f.db()).unwrap();
    let status: String = f
        .db()
        .query_row(
            "SELECT status FROM wrapper_shell_tasks WHERE call_id='crash'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "arrêtée");
    let stop = f.app.api(
        "POST",
        "/api/local/tasks/stop",
        serde_json::to_vec(&json!({"run_id": run_id, "call_id": "crash"}))
            .unwrap()
            .as_slice(),
    );
    assert_eq!(stop.unwrap_err().status, 409);
    assert!(!marker.exists());
}

#[test]
fn reused_pgid_of_another_group_survives_restart() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("seed", "run_shell", f.shell("echo seed"));
    let request = f.request("seed");
    f.reply("ok");
    f.decide(&request, json!("allow_once"));
    f.finish("seed");
    let run_id = f.run("seed").run_id.clone();
    let marker = f.root().join("reused.done");
    let mut child = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            &format!("sleep 30; touch {}", marker.display()),
            "unrelated-group",
        ])
        .current_dir(f.root())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pgid = child.id();
    let started = grokbot_wrapper::features::local::shell_leader_started(pgid)
        .expect("identité du groupe réutilisé");
    f.db()
        .execute(
            "INSERT INTO engine_external_tools(run_id,call_id,authorized_by,outcome) VALUES (?1,'reuse','null',NULL)",
            [&run_id],
        )
        .unwrap();
    // The row still names this pid, with the real start time of whoever holds
    // it now, but the watchdog token belongs to a group that already exited.
    f.db()
        .execute(
            "INSERT INTO wrapper_shell_tasks(run_id,call_id,status,exit_code,pgid,leader,started) VALUES (?1,'reuse','en cours',NULL,?2,?3,?4)",
            params![
                run_id,
                pgid,
                "ezerbot-shell-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                started
            ],
        )
        .unwrap();
    grokbot_wrapper::features::local::startup(&f.db()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.try_wait().unwrap().is_none(),
        "un groupe réutilisé a été tué"
    );
    assert!(process_alive(pgid));
    assert!(!marker.exists());
    let status: String = f
        .db()
        .query_row(
            "SELECT status FROM wrapper_shell_tasks WHERE call_id='reuse'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "arrêtée");
    let outcome: String = f
        .db()
        .query_row(
            "SELECT outcome FROM engine_external_tools WHERE call_id='reuse'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let outcome: Value = serde_json::from_str(&outcome).unwrap();
    assert_eq!(outcome["output"]["status"], "arrêtée");
    assert!(outcome["output"]["exit_code"].is_null());
    let _ = std::process::Command::new("/bin/kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .status();
    let _ = child.wait();
    assert!(!marker.exists());
}

#[test]
fn unreadable_vault_stops_the_background_process() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let marker = f.root().join("vault.done");
    let mut args = f.shell(&format!(
        "echo $$ > vault.pid; sleep 30; touch {}",
        marker.display()
    ));
    args["background"] = json!(true);
    args["timeout_ms"] = json!(100);
    f.send("vault", "run_shell", args);
    let request = f.request("vault");
    f.reply("ok");
    f.decide(&request, json!("allow_once"));
    f.finish("vault");
    let pid = {
        let path = f.root().join("vault.pid");
        let mut found = None;
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    found = Some(pid);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        found.expect("pid absent")
    };
    assert!(process_alive(pid));
    std::fs::write(f.path.with_extension("secrets.json"), b"coffre illisible").unwrap();
    let mut dead = false;
    for _ in 0..150 {
        if !process_alive(pid) {
            dead = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        dead,
        "le processus continue alors que la tâche n’est plus arrêtable"
    );
    let status: String = f
        .db()
        .query_row(
            "SELECT status FROM wrapper_shell_tasks WHERE call_id='vault'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(status, "en cours");
    let stop = f.app.api(
        "POST",
        "/api/local/tasks/stop",
        serde_json::to_vec(&json!({"run_id": f.run("vault").run_id, "call_id": "vault"}))
            .unwrap()
            .as_slice(),
    );
    assert_eq!(stop.unwrap_err().status, 409);
    std::thread::sleep(Duration::from_millis(200));
    assert!(!marker.exists());
}

#[test]
fn a_removed_secret_never_returns_through_the_approval_policy() {
    const SECRET: &str = "jeton-secret-valeur-8123";
    let mut f = Fixture::new();
    f.settings(true, false);
    f.post(
        "/api/secrets/put",
        json!({"bot_id":"grokbot","name":"jeton","description":"Jeton de test","value":SECRET}),
    );
    f.send("masked", "run_shell", f.shell(&format!("printf {SECRET}")));
    let r = f.request("masked");
    let card = |f: &mut Fixture| {
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        let card = state["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|request| request["id"] == r.id)
            .unwrap()
            .clone();
        (state.to_string(), card)
    };
    let (before, shown) = card(&mut f);
    assert!(!before.contains(SECRET));
    assert_eq!(shown["policy"]["remember"], false, "{shown}");
    f.post(
        "/api/secrets/delete",
        json!({"bot_id":"grokbot","name":"jeton"}),
    );
    let (after, shown) = card(&mut f);
    assert!(!after.contains(SECRET), "{after}");
    assert_eq!(shown["policy"]["remember"], false);
    assert_eq!(shown["policy"]["pattern"], "");
    f.decide(&r, json!("deny"));
    f.finish("masked");
}

fn approval_policy(f: &mut Fixture, id: &str) -> Value {
    let run = f.run(id).run_id;
    f.app.api("GET", "/api/state", b"").unwrap()["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"] == f.chat)
        .flat_map(|c| c["messages"].as_array().unwrap().clone())
        .find(|m| m["kind"] == "approval" && m["payload"]["run_id"] == run)
        .expect("carte d’approbation")["payload"]["policy"]
        .clone()
}

#[test]
fn always_allow_same_command_skips_the_card_survives_restart_and_revoke_asks_again() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("first", "run_shell", f.shell("printf oui"));
    let card = f.request("first");
    let policy = approval_policy(&mut f, "first");
    assert_eq!(policy["remember"], true);
    assert_eq!(policy["scope"], "pattern");
    assert_eq!(policy["pattern"], "printf oui");
    f.decide(&card, json!({"always_allow":{"scope":"pattern"}}));
    f.finish("first");
    assert_eq!(f.outcome("first")["output"]["stdout"], "oui");
    let other_cwd = f.root().join("ailleurs");
    std::fs::create_dir(&other_cwd).unwrap();
    let mut same = f.shell("printf oui");
    same["cwd"] = json!(other_cwd);
    same["timeout_ms"] = json!(1500);
    f.send("same", "run_shell", same);
    f.finish("same");
    assert!(requests::list_for_run(&f.db(), &f.run("same").run_id)
        .unwrap()
        .is_empty());
    assert_eq!(f.outcome("same")["output"]["stdout"], "oui");
    f.reopen();
    f.send("restarted", "run_shell", f.shell("printf oui"));
    f.finish("restarted");
    assert!(requests::list_for_run(&f.db(), &f.run("restarted").run_id)
        .unwrap()
        .is_empty());
    assert_eq!(f.outcome("restarted")["output"]["stdout"], "oui");
    let rules = f.post(
        "/api/approvals/rules/revoke",
        json!({
            "requesting_bot_id":"grokbot","target_bot_id":"grokbot","tool":"run_shell",
            "args_digest":"printf oui","scope":"pattern"
        }),
    );
    assert_eq!(rules["rules"], json!([]));
    f.send("revoked", "run_shell", f.shell("printf oui"));
    let again = f.request("revoked");
    assert!(matches!(
        again.status,
        grokbot_forms::requests::RequestStatus::Pending
    ));
    f.decide(&again, json!("deny"));
    f.finish("revoked");
    assert_eq!(f.outcome("revoked")["status"], "failed");
}

#[test]
fn destructive_shell_still_asks_when_a_permanent_shell_rule_exists() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("allow-shell", "run_shell", f.shell("printf oui"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    let target = f.root().join("victime.txt");
    std::fs::write(&target, "garde").unwrap();
    f.send("rm", "run_shell", f.shell("rm victime.txt"));
    wait("rm asked or finished", || {
        let status = f.run("rm").status;
        status == RunStatus::AwaitingApproval || status.is_terminal()
    });
    assert_eq!(
        f.run("rm").status,
        RunStatus::AwaitingApproval,
        "une suppression ne suit pas la règle permanente"
    );
    assert!(target.exists());
    let card = f.request("rm");
    let policy = approval_policy(&mut f, "rm");
    assert_eq!(policy["remember"], false);
    assert_eq!(policy["protection"], "destructive");
    let refused = f.app.api(
        "POST",
        "/api/approvals/resolve",
        json!({
            "entry_id": card.entry_id,
            "submission_id": "always-rm",
            "conversation_id": card.route.target_conversation_id,
            "decision": {"always_allow":{"scope":"global"}}
        })
        .to_string()
        .as_bytes(),
    );
    assert!(refused.is_err());
    assert!(target.exists());
    f.decide(&card, json!("allow_once"));
    f.finish("rm");
    assert!(!target.exists());
}

#[test]
fn exact_shell_key_and_unproven_commands_stay_on_the_card() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("pattern", "run_shell", f.shell("printf oui"));
    let card = f.request("pattern");
    assert_eq!(approval_policy(&mut f, "pattern")["pattern"], "printf oui");
    f.decide(&card, json!({"always_allow":{"scope":"pattern"}}));
    f.finish("pattern");
    f.send("spaced", "run_shell", f.shell("printf  oui"));
    let spaced = f.request("spaced");
    assert_eq!(approval_policy(&mut f, "spaced")["pattern"], "printf  oui");
    f.decide(&spaced, json!("deny"));
    f.finish("spaced");
    f.send("global", "run_shell", f.shell("printf encore"));
    let global = f.request("global");
    f.decide(&global, json!({"always_allow":{"scope":"global"}}));
    f.finish("global");
    let cases = [
        ("semi", "printf ok; rm note", Some("destructive")),
        ("and", "cd /tmp && printf ok", None),
        ("exec", "find . -exec ls {} +", None),
        ("git", "git clean -fd", Some("destructive")),
        ("interp", "bash -c 'printf oui'", None),
        ("mail", "sendmail someone", Some("outbound")),
    ];
    for (id, command, protection) in cases {
        f.send(id, "run_shell", f.shell(command));
        wait("carte obligatoire", || {
            let status = f.run(id).status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        assert_eq!(f.run(id).status, RunStatus::AwaitingApproval, "{command}");
        let policy = approval_policy(&mut f, id);
        assert_eq!(policy["remember"], false, "{command}");
        if let Some(kind) = protection {
            assert_eq!(policy["protection"], kind, "{command}");
        }
        let card = f.request(id);
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{id}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"global"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{command}");
        f.decide(&card, json!("deny"));
        f.finish(id);
    }
}

#[test]
fn git_deletions_still_ask_when_a_permanent_shell_rule_exists() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(f.root())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "i",
    ]);
    git(&["branch", "vieille"]);
    std::fs::write(f.root().join("note"), "garde").unwrap();
    git(&["add", "note"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "-m",
        "note",
    ]);
    f.send("allow-shell", "run_shell", f.shell("git status --short"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    for (id, command) in [
        ("rm", "git rm -f note"),
        ("c-rm", "git -C . rm note"),
        ("restore", "git restore note"),
        ("checkout", "git checkout -- note"),
        ("checkout-tree", "git checkout HEAD note"),
        ("branch", "git branch -D vieille"),
        ("stash", "git stash drop"),
        ("worktree", "git worktree remove ../ailleurs"),
    ] {
        std::fs::write(f.root().join("note"), "modifiée").unwrap();
        f.send(id, "run_shell", f.shell(command));
        wait("carte ou fin", || {
            let status = f.run(id).status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        assert_eq!(f.run(id).status, RunStatus::AwaitingApproval, "{command}");
        assert_eq!(
            std::fs::read_to_string(f.root().join("note")).unwrap(),
            "modifiée",
            "{command}"
        );
        let policy = approval_policy(&mut f, id);
        assert_eq!(policy["remember"], false, "{command}");
        assert_eq!(policy["protection"], "destructive", "{command}");
        assert_eq!(policy["label"], "Suppression à confirmer", "{command}");
        let card = f.request(id);
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{id}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"pattern"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{command}");
        f.decide(&card, json!("deny"));
        f.finish(id);
        assert!(f.root().join("note").exists(), "{command}");
    }
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "vieille"])
        .current_dir(f.root())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).contains("vieille"));
    // Non-destructive git commands follow the permanent rule without a card.
    for (id, command) in [("list", "git stash list"), ("plain-branch", "git branch")] {
        f.send(id, "run_shell", f.shell(command));
        f.finish(id);
        assert!(
            requests::list_for_run(&f.db(), &f.run(id).run_id)
                .unwrap()
                .is_empty(),
            "{command}"
        );
        assert_eq!(f.outcome(id)["status"], "done", "{command}");
    }
}

#[test]
fn settled_read_card_keeps_its_policy_and_shows_its_decision() {
    let mut f = Fixture::new();
    f.settings(true, true);
    let file = f.root().join("lu.txt");
    std::fs::write(&file, "Bonjour\n").unwrap();
    f.send("read", "read_file", json!({"path":file}));
    let card = f.request("read");
    let pending = approval_policy(&mut f, "read");
    assert_eq!(pending["remember"], false);
    assert_eq!(pending["scope"], "none");
    assert!(pending.get("protection").is_none());
    f.decide(&card, json!("allow_once"));
    f.finish("read");
    assert_eq!(f.outcome("read")["output"]["content"], "Bonjour\n");
    assert_eq!(approval_policy(&mut f, "read"), pending);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let decision = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == card.id)
        .unwrap()["decision"]
        .clone();
    assert_eq!(decision, json!("allow_once"));
}

#[test]
fn attached_interpreter_code_still_asks_when_a_permanent_shell_rule_exists() {
    let mut f = Fixture::new();
    f.settings(true, false);
    std::fs::write(f.root().join("script.pl"), "print \"script\";\n").unwrap();
    f.send("plain", "run_shell", f.shell("perl script.pl"));
    let card = f.request("plain");
    let policy = approval_policy(&mut f, "plain");
    assert_eq!(policy["remember"], true);
    assert_eq!(policy["scope"], "pattern");
    f.decide(&card, json!({"always_allow":{"scope":"pattern"}}));
    f.finish("plain");
    assert_eq!(f.outcome("plain")["output"]["stdout"], "script");
    f.send("plain-again", "run_shell", f.shell("perl script.pl"));
    f.finish("plain-again");
    assert!(
        requests::list_for_run(&f.db(), &f.run("plain-again").run_id)
            .unwrap()
            .is_empty()
    );
    f.send("allow-shell", "run_shell", f.shell("printf oui"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    let note = f.root().join("note");
    for (id, command) in [
        ("perl-e", "perl -e'unlink \"note\"'"),
        ("perl-ne", "perl -ne 'unlink \"note\"' script.pl"),
        ("python-bc", "python3 -Bc 'import os; os.remove(\"note\")'"),
        ("awk", "awk 'BEGIN{system(\"rm note\")}'"),
        ("checkout-one", "git checkout note"),
    ] {
        std::fs::write(&note, "garde").unwrap();
        f.send(id, "run_shell", f.shell(command));
        wait("carte ou fin", || {
            let status = f.run(id).status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        assert_eq!(f.run(id).status, RunStatus::AwaitingApproval, "{command}");
        assert!(note.exists(), "{command}");
        assert_eq!(approval_policy(&mut f, id)["remember"], false, "{command}");
        let card = f.request(id);
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{id}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"global"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{command}");
        f.decide(&card, json!("deny"));
        f.finish(id);
        assert!(note.exists(), "{command}");
    }
}

#[test]
fn git_inline_aliases_still_ask_when_a_permanent_shell_rule_exists() {
    let mut f = Fixture::new();
    f.settings(true, false);
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(f.root())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(f.root().join("note"), "garde").unwrap();
    git(&["add", "note"]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t",
        "commit",
        "-q",
        "-m",
        "note",
    ]);
    f.send("allow-shell", "run_shell", f.shell("git status --short"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    f.send("plain", "run_shell", f.shell("git -C . status --short"));
    f.finish("plain");
    assert!(requests::list_for_run(&f.db(), &f.run("plain").run_id)
        .unwrap()
        .is_empty());
    let cases = [
        ("alias", "git -c alias.wipe=rm wipe -q note"),
        ("glued", "git -calias.wipe=rm wipe -q note"),
        ("dir", "git -C . -c alias.wipe=rm wipe -q note"),
        (
            "env",
            "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.wipe GIT_CONFIG_VALUE_0=rm git wipe -q note",
        ),
    ];
    for (id, command) in cases {
        f.send(id, "run_shell", f.shell(command));
        wait("carte ou fin", || {
            let status = f.run(id).status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        assert_eq!(f.run(id).status, RunStatus::AwaitingApproval, "{command}");
        assert!(f.root().join("note").exists(), "{command}");
        assert_eq!(approval_policy(&mut f, id)["remember"], false, "{command}");
        let card = f.request(id);
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{id}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"global"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{command}");
        f.decide(&card, json!("deny"));
        f.finish(id);
        assert!(f.root().join("note").exists(), "{command}");
    }
    // The alias really deletes: only the human's one-time approval runs it.
    f.send(
        "once",
        "run_shell",
        f.shell("git -c alias.wipe=rm wipe -q note"),
    );
    let card = f.request("once");
    f.decide(&card, json!("allow_once"));
    f.finish("once");
    assert!(!f.root().join("note").exists());
}

#[test]
fn forge_cli_sends_still_ask_when_a_permanent_shell_rule_exists() {
    // Nonexistent repositories: even a regression could not post anywhere.
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("allow-shell", "run_shell", f.shell("printf oui"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    f.send(
        "read",
        "run_shell",
        f.shell("gh pr view 7 --repo ezerbot-inexistant-0/rien"),
    );
    f.finish("read");
    assert!(requests::list_for_run(&f.db(), &f.run("read").run_id)
        .unwrap()
        .is_empty());
    for (id, command) in [
        (
            "issue-comment",
            "gh issue comment 1 --repo ezerbot-inexistant-0/rien --body bonjour",
        ),
        (
            "pr-comment",
            "gh pr comment 7 -R ezerbot-inexistant-0/rien -b salut",
        ),
        (
            "repo-first",
            "gh -R ezerbot-inexistant-0/rien issue comment 1 -b salut",
        ),
        (
            "pr-review",
            "gh pr review 7 -R ezerbot-inexistant-0/rien --approve",
        ),
        (
            "api-post",
            "gh api -X POST repos/ezerbot-inexistant-0/rien/issues/1/comments",
        ),
        (
            "api-field",
            "gh api repos/ezerbot-inexistant-0/rien/issues/1/comments -f body=salut",
        ),
        (
            "glab-note",
            "glab mr note 3 -R ezerbot-inexistant-0/rien -m salut",
        ),
    ] {
        f.send(id, "run_shell", f.shell(command));
        wait("carte ou fin", || {
            let status = f.run(id).status;
            status == RunStatus::AwaitingApproval || status.is_terminal()
        });
        assert_eq!(f.run(id).status, RunStatus::AwaitingApproval, "{command}");
        assert_eq!(approval_policy(&mut f, id)["remember"], false, "{command}");
        let card = f.request(id);
        let refused = f.app.api(
            "POST",
            "/api/approvals/resolve",
            json!({
                "entry_id": card.entry_id,
                "submission_id": format!("always-{id}"),
                "conversation_id": card.route.target_conversation_id,
                "decision": {"always_allow":{"scope":"global"}}
            })
            .to_string()
            .as_bytes(),
        );
        assert!(refused.is_err(), "{command}");
        f.decide(&card, json!("deny"));
        f.finish(id);
        assert_eq!(f.outcome(id)["status"], "failed", "{command}");
    }
}

#[test]
fn background_destructive_shell_still_asks_when_a_permanent_shell_rule_exists() {
    let mut f = Fixture::new();
    f.settings(true, false);
    f.send("allow-shell", "run_shell", f.shell("printf oui"));
    let allowed = f.request("allow-shell");
    f.decide(&allowed, json!({"always_allow":{"scope":"global"}}));
    f.finish("allow-shell");
    let target = f.root().join("victime.txt");
    std::fs::write(&target, "garde").unwrap();
    let mut args = f.shell("rm victime.txt");
    args["background"] = json!(true);
    f.send("bg-rm", "run_shell", args);
    wait("background rm asked or finished", || {
        let status = f.run("bg-rm").status;
        status == RunStatus::AwaitingApproval || status.is_terminal()
    });
    assert_eq!(f.run("bg-rm").status, RunStatus::AwaitingApproval);
    assert!(target.exists());
    let card = f.request("bg-rm");
    let policy = approval_policy(&mut f, "bg-rm");
    assert_eq!(policy["remember"], false);
    assert_eq!(policy["protection"], "destructive");
    f.decide(&card, json!("deny"));
    f.finish("bg-rm");
    assert!(target.exists());
}

#[test]
fn a_finished_background_command_is_listed_with_its_exit_code_in_the_next_reply() {
    let mut f = Fixture::new();
    f.settings(true, false);
    start_background(&mut f, "code", "echo raté >&2; exit 3");
    wait_output(&mut f, "code", "code de sortie", |o| o["status"] == "échec");
    f.send("fil", "check_delegations", json!({"portee":"fil"}));
    f.reply("vu");
    f.finish("fil");
    let task = f.outcome("fil")["output"]["tasks"][0].clone();
    assert_eq!(task["kind"], "command");
    assert_eq!(task["summary"], "echo raté >&2; exit 3");
    assert_eq!(task["status"], "échec");
    assert_eq!(task["exit_code"], json!(3));
}
