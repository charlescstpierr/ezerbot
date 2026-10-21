//! Binary-level tests: every launchctl call is recorded below a throwaway HOME.
//! No test invokes launchctl or touches the user's LaunchAgents directory.
//! Installation still asks /usr/bin/plutil to read the agent plist, so these
//! tests run on macOS only.
#![cfg(target_os = "macos")]
mod support;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Fixture {
    root: PathBuf,
    app: Option<Child>,
    url: String,
    fake: FakeOpenAi,
    binary: PathBuf,
    database: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-routines-bg-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("workspace.chatgpt-auth.json"),json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let fake = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let mut f = Self {
            database: root.join("workspace.sqlite"),
            root,
            app: None,
            url: String::new(),
            fake,
            binary: env!("CARGO_BIN_EXE_grokbot-wrapper").into(),
        };
        f.open();
        f
    }
    fn command(&self) -> Command {
        let mut c = Command::new(&self.binary);
        c.args(["--db"])
            .arg(&self.database)
            .current_dir(&self.root)
            .env("HOME", &self.root)
            .env("CFFIXED_USER_HOME", &self.root)
            .env("GROKBOT_LAUNCH_AGENT_TEST_ROOT", &self.root)
            .env("GROKBOT_FAKE_OPENAI", &self.fake.base)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        c
    }
    fn open(&mut self) {
        let mut child = self.command().args(["--port", "0"]).spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        self.url = serde_json::from_str::<Value>(&line).unwrap()["url"]
            .as_str()
            .unwrap()
            .into();
        self.app = Some(child);
        wait("model catalogue", || {
            self.state()["provider"]["models_ready"] == true
        });
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.app.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
    fn state(&self) -> Value {
        ureq::get(&format!("{}/api/state", self.url))
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn post(&self, path: &str, body: Value) -> Value {
        ureq::post(&format!("{}{path}", self.url))
            .set("X-Grokbot-Request", "local")
            .send_json(body)
            .unwrap()
            .into_json()
            .unwrap()
    }
    fn request(&self, enabled: bool) -> Value {
        let result = self.post("/api/routines/background", json!({"enabled":enabled}));
        let mut request = Value::Null;
        wait("background approval", || {
            request = self.state()["forms"]["requests"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["run_id"] == result["run_id"])
                .cloned()
                .unwrap_or(Value::Null);
            !request.is_null()
        });
        request
    }
    fn decide(&self, request: &Value, decision: Value) {
        self.post("/api/approvals/resolve",json!({"entry_id":request["entry_id"],"submission_id":format!("decision-{}",now()),"conversation_id":request["route"]["target_conversation_id"],"decision":decision}));
    }
    fn configure(&self, enabled: bool) {
        let request = self.request(enabled);
        self.decide(&request, json!("allow_once"));
        wait("background configuration", || {
            self.state()["routines_background"]["enabled"] == enabled
                && self.state()["routines_background"]["installed"] == enabled
        });
    }
    fn plist(&self) -> PathBuf {
        self.root
            .join("Library/LaunchAgents/local.ezerbot.routines.plist")
    }
    fn routine(&self, at: i64) -> String {
        self.post("/api/routines",json!({"bot_id":"grokbot","name":"Vérification fermée","prompt":"Rédige le bilan de test.","trigger":{"kind":"once","at":at},"time_zone":"UTC","enabled":true}))["routine"]["routine_id"].as_str().unwrap().into()
    }
    fn wake(&self) -> std::process::Output {
        self.command().arg("--headless-routines").output().unwrap()
    }
    fn history(&self, id: &str) -> Value {
        ureq::get(&format!(
            "{}/api/routines/history?bot_id=grokbot&routine_id={id}",
            self.url
        ))
        .call()
        .unwrap()
        .into_json()
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
fn tool(name: &str, args: Value) -> ScriptedResponse {
    ScriptedResponse::events(&[
        json!({"type":"response.output_item.done","item":{"id":format!("fc-{name}"),"status":"completed","type":"function_call","call_id":format!("test-{name}"),"name":name,"arguments":args.to_string()}}),
        json!({"type":"response.completed"}),
    ])
}
fn one_row(path: &Path, sql: &str) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[test]
fn installation_denial_error_restart_and_clean_uninstall() {
    let mut f = Fixture::new();
    assert_eq!(f.state()["routines_background"]["enabled"], false);
    let request = f.request(true);
    let duplicate = f.post("/api/routines/background", json!({"enabled":true}));
    assert_eq!(duplicate["run_id"], request["run_id"]);
    assert_eq!(duplicate["approval_message_id"], request["entry_id"]);
    f.decide(&request, json!("deny"));
    std::thread::sleep(Duration::from_millis(150));
    assert!(!f.plist().exists());
    assert!(!f.root.join("launchctl-recording.jsonl").exists());
    f.configure(true);
    assert!(Command::new("/usr/bin/plutil")
        .arg("-lint")
        .arg(f.plist())
        .status()
        .unwrap()
        .success());
    let recorded = fs::read_to_string(f.root.join("launchctl-recording.jsonl")).unwrap();
    assert!(recorded.contains("bootstrap"));
    f.stop();
    f.open();
    assert_eq!(f.state()["routines_background"]["installed"], true);
    fs::write(f.root.join("launchctl-fail"), "").unwrap();
    let request = f.request(false);
    f.decide(&request, json!("allow_once"));
    wait("launchctl failure visible", || {
        f.state()["routines_background"]["error"]
            .as_str()
            .is_some_and(|s| s.contains("simulé"))
    });
    assert!(f.plist().exists());
    wait("failure on decision card", || {
        f.state()["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| {
                r["run_id"] == request["run_id"] && r["configuration_result"]["status"] == "failed"
            })
    });
    assert_eq!(f.state()["routines_background"]["enabled"], true);
    fs::remove_file(f.root.join("launchctl-fail")).unwrap();
    f.configure(false);
    assert!(!f.plist().exists());
    assert!(fs::read_to_string(f.root.join("launchctl-recording.jsonl"))
        .unwrap()
        .contains("bootout"));
    f.stop();
    assert!(f.wake().status.success());
}

#[test]
fn due_run_finishes_persists_notifies_once_and_skips_idle_and_foreground_owner() {
    let mut f = Fixture::new();
    f.configure(true);
    f.post("/api/notifications/settings",json!({"enabled":true,"dnd":false,"reply":false,"approval":false,"form":false,"secret":false,"routine":true,"delegation":false}));
    let id = f.routine(now() + 1800);
    // No work while the foreground app owns the engine, even after it's due.
    assert!(f.wake().status.success());
    f.stop();
    let requests = f.fake.seen.lock().unwrap().bodies.len();
    assert!(f.wake().status.success()); // not due: no engine/provider startup
    assert_eq!(f.fake.seen.lock().unwrap().bodies.len(), requests);
    f.fake.seen.lock().unwrap().responses.push_back(tool(
        "complete_task",
        json!({"summary":"Bilan terminé dans le moteur fermé."}),
    ));
    std::thread::sleep(Duration::from_millis(1850));
    assert!(f.wake().status.success());
    let db = f.root.join("workspace.sqlite");
    assert_eq!(
        one_row(
            &db,
            "SELECT count(*) FROM scheduler_occurrences WHERE status='done'"
        ),
        1
    );
    let records = fs::read_to_string(f.root.join("notifications-recording.jsonl")).unwrap();
    assert_eq!(records.lines().count(), 1);
    assert!(records.contains("Routine terminée"));
    assert!(f.wake().status.success());
    assert_eq!(
        fs::read_to_string(f.root.join("notifications-recording.jsonl")).unwrap(),
        records
    );
    f.open();
    let h = f.history(&id);
    assert_eq!(h["occurrences"].as_array().unwrap().len(), 1);
    assert_eq!(h["occurrences"][0]["status"]["status"], "done");
    assert!(h["occurrences"][0]["conversation_id"].is_string());
    assert!(f.state()["routines_background"]["last_run"].is_number());
    let inbox: Value = ureq::get(&format!("{}/api/notifications", f.url))
        .call()
        .unwrap()
        .into_json()
        .unwrap();
    assert!(inbox["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["title"] == "Routine terminée"));
}

#[test]
fn failed_run_notifies_and_protected_action_waits_for_app_without_execution() {
    let mut f = Fixture::new();
    f.configure(true);
    f.post("/api/notifications/settings",json!({"enabled":true,"dnd":false,"reply":false,"approval":false,"form":false,"secret":false,"routine":true,"delegation":false}));
    let id = f.routine(now() + 700);
    f.fake
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            400,
            json!({"error":{"message":"Échec contrôlé","code":"invalid_request_error"}}),
        ));
    f.stop();
    std::thread::sleep(Duration::from_millis(750));
    assert!(f.wake().status.success());
    assert!(
        fs::read_to_string(f.root.join("notifications-recording.jsonl"))
            .unwrap()
            .contains("Échec de la routine")
    );
    f.open();
    assert!(f.state()["routines_background"]["error"]
        .as_str()
        .unwrap()
        .contains("échoué"));
    assert_eq!(
        f.history(&id)["occurrences"][0]["status"]["status"],
        "failed"
    );
    f.post(
        "/api/local/bot",
        json!({"bot_id":"grokbot","shell":true,"files":false}),
    );
    let id2 = f.routine(now() + 700);
    f.fake.seen.lock().unwrap().responses.push_back(tool("run_shell",json!({"command":"touch forbidden-sentinel","cwd":f.root,"timeout_ms":5000,"output_limit":4096})));
    f.stop();
    std::thread::sleep(Duration::from_millis(750));
    assert!(f.wake().status.success());
    assert!(!f.root.join("forbidden-sentinel").exists());
    f.open();
    assert_eq!(
        f.history(&id2)["occurrences"][0]["status"]["status"],
        "running"
    );
    assert!(f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["kind"]["tool"] == "run_shell" && r["status"]["status"] == "pending"));
}

#[test]
fn opening_app_during_headless_run_waits_for_owner_and_does_not_duplicate() {
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new();
    f.configure(true);
    let id = f.routine(now() + 700);
    let gate = Arc::new(AtomicBool::new(false));
    {
        let mut seen = f.fake.seen.lock().unwrap();
        seen.response_gate = Some((seen.bodies.len() + 1, gate.clone()));
        seen.responses
            .push_back(tool("complete_task", json!({"summary":"Exécution unique"})));
    }
    f.stop();
    std::thread::sleep(Duration::from_millis(750));
    let mut headless = f.command().arg("--headless-routines").spawn().unwrap();
    wait("headless model call", || {
        !f.fake.seen.lock().unwrap().bodies.is_empty()
    });
    assert!(f.wake().status.success());
    let mut foreground = f.command().args(["--port", "0"]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(foreground.try_wait().unwrap().is_none());
    assert_eq!(
        one_row(
            &f.root.join("workspace.sqlite"),
            "SELECT count(*) FROM engine_runs WHERE trigger_kind='occurrence'"
        ),
        1
    );
    // Handover asks the live worker to checkpoint instead of killing it.
    f.fake.seen.lock().unwrap().responses.push_back(tool(
        "complete_task",
        json!({"summary":"Reprise de la même occurrence"}),
    ));
    gate.store(true, Ordering::SeqCst);
    assert!(headless.wait().unwrap().success());
    let mut line = String::new();
    BufReader::new(foreground.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    f.url = serde_json::from_str::<Value>(&line).unwrap()["url"]
        .as_str()
        .unwrap()
        .into();
    f.app = Some(foreground);
    assert_eq!(f.history(&id)["occurrences"].as_array().unwrap().len(), 1);
    wait("foreground completes handed-over occurrence", || {
        f.history(&id)["occurrences"][0]["status"]["status"] == "done"
    });
}

#[test]
fn killed_headless_owner_releases_lock_and_restart_reuses_occurrence_and_run() {
    use std::sync::atomic::AtomicBool;
    let mut f = Fixture::new();
    f.configure(true);
    f.routine(now() + 700);
    let gate = Arc::new(AtomicBool::new(false));
    {
        let mut seen = f.fake.seen.lock().unwrap();
        seen.response_gate = Some((seen.bodies.len() + 1, gate.clone()));
    }
    f.stop();
    std::thread::sleep(Duration::from_millis(750));
    let mut child = f.command().arg("--headless-routines").spawn().unwrap();
    wait("headless interrupted model call", || {
        !f.fake.seen.lock().unwrap().bodies.is_empty()
    });
    child.kill().unwrap();
    child.wait().unwrap();
    f.fake.seen.lock().unwrap().responses.push_back(tool(
        "complete_task",
        json!({"summary":"Reprise sans seconde occurrence"}),
    ));
    gate.store(true, Ordering::SeqCst);
    assert!(f.wake().status.success());
    let db = f.root.join("workspace.sqlite");
    assert_eq!(
        one_row(&db, "SELECT count(*) FROM scheduler_occurrences"),
        1
    );
    assert_eq!(
        one_row(
            &db,
            "SELECT count(*) FROM engine_runs WHERE trigger_kind='occurrence'"
        ),
        1
    );
    assert_eq!(
        one_row(
            &db,
            "SELECT count(*) FROM scheduler_occurrences WHERE status='done'"
        ),
        1
    );
}

#[test]
fn moved_binary_can_repair_and_uninstall_but_another_workspace_cannot() {
    let mut f = Fixture::new();
    f.configure(true);
    f.stop();
    let moved = f.root.join("renamed-wrapper");
    fs::copy(&f.binary, &moved).unwrap();
    f.binary = moved.clone();
    f.open();
    // Re-approve activation directly to repair the executable path.
    let request = f.request(true);
    f.decide(&request, json!("allow_once"));
    wait("agent points to moved binary", || {
        fs::read_to_string(f.plist())
            .unwrap()
            .contains(moved.to_str().unwrap())
    });
    f.configure(false);
    assert!(!f.plist().exists());
    f.configure(true);
    f.stop();
    // A moved database retains the old ownership path and can also withdraw.
    let old = f.root.join("workspace.sqlite");
    let saved = f.root.join("moved.sqlite");
    {
        let conn = rusqlite::Connection::open(&old).unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
    }
    fs::rename(&old, &saved).unwrap();
    fs::copy(
        f.root.join("workspace.chatgpt-auth.json"),
        f.root.join("moved.chatgpt-auth.json"),
    )
    .unwrap();
    f.database = saved;
    f.open();
    // The DB moved: disable still recognizes the persisted previous plist path.
    f.configure(false);
    f.configure(true);
    let body = fs::read_to_string(f.plist()).unwrap();
    let foreign = body.replace(
        f.database.to_str().unwrap(),
        "/tmp/another-workspace.sqlite",
    );
    fs::write(f.plist(), &foreign).unwrap();
    let request = f.request(false);
    f.decide(&request, json!("allow_once"));
    wait("foreign agent refused", || {
        f.state()["routines_background"]["error"]
            .as_str()
            .is_some_and(|s| s.contains("autre espace"))
    });
    assert_eq!(fs::read_to_string(f.plist()).unwrap(), foreign);
    fs::write(f.plist(), body).unwrap();
    f.configure(false);
    let records = fs::read_to_string(f.root.join("launchctl-recording.jsonl")).unwrap();
    assert!(records.contains("bootout"));
}

#[test]
fn deadline_checkpoints_elapsed_budget_and_cannot_replay_forever() {
    let mut f = Fixture::new();
    f.configure(true);
    f.routine(now() + 700);
    f.fake.seen.lock().unwrap().response_delay = Some(Duration::from_millis(900));
    f.stop();
    std::thread::sleep(Duration::from_millis(750));
    let db = f.root.join("workspace.sqlite");
    let wake = || {
        f.command()
            .arg("--routine-wake")
            .env("GROKBOT_ROUTINE_PASS_MS", "500")
            .output()
            .unwrap()
    };
    assert!(!wake().status.success());
    assert_eq!(
        one_row(
            &db,
            "SELECT count(*) FROM engine_runs WHERE status='interrupted'"
        ),
        1
    );
    let first = one_row(&db, "SELECT json_extract(usage,'$.elapsed_ms') FROM engine_runs WHERE trigger_kind='occurrence'");
    assert!(first >= 850, "elapsed interval lost: {first}");
    // Two passes consume the run's budget, not 12 repeated charged calls.
    rusqlite::Connection::open(&db).unwrap().execute(
        "UPDATE engine_runs SET record=json_set(record,'$.budgets.duration_ms',?1) WHERE trigger_kind='occurrence'", [first + 700],
    ).unwrap();
    assert!(!wake().status.success());
    let second = one_row(&db, "SELECT json_extract(usage,'$.elapsed_ms') FROM engine_runs WHERE trigger_kind='occurrence'");
    assert!(second > first);
    assert!(f.wake().status.success()); // budget already exhausted, no third request
    assert_eq!(one_row(&db, "SELECT json_extract(usage,'$.model_calls') FROM engine_runs WHERE trigger_kind='occurrence'"), 2);
    assert_eq!(
        one_row(
            &db,
            "SELECT count(*) FROM scheduler_occurrences WHERE status='failed'"
        ),
        1
    );
    assert_eq!(
        one_row(&db, "SELECT count(*) FROM scheduler_occurrences"),
        1
    );
    f.open();
    assert!(f.state()["routines_background"]["error"]
        .as_str()
        .unwrap()
        .contains("échoué"));
}

#[test]
fn debug_build_cannot_install_without_explicit_throwaway_override() {
    let f = Fixture::new();
    f.stopless_debug_safety();
}
impl Fixture {
    fn stopless_debug_safety(&self) {
        // Test the environment boundary in a separate process, never mutating
        // this test runner's shared HOME or invoking an actual launchctl.
        let root = self.root.join("unsafe-override");
        fs::create_dir_all(&root).unwrap();
        let mut command = self.command();
        command
            .env_remove("GROKBOT_LAUNCH_AGENT_TEST_ROOT")
            .arg("--port")
            .arg("0")
            .arg("--db")
            .arg(root.join("workspace.sqlite"));
        let mut child = command.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let url = serde_json::from_str::<Value>(&line).unwrap()["url"]
            .as_str()
            .unwrap()
            .to_owned();
        let response = ureq::post(&format!("{url}/api/routines/background"))
            .set("X-Grokbot-Request", "local")
            .send_json(json!({"enabled":true}));
        assert!(response.is_err());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!self.plist().exists());
    }
}
