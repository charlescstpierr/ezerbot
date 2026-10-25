//! Journal local des tours (#461) : lu comme le lit l'utilisateur, par la table
//! `wrapper_turn_log` et par `GET /api/turns/summary`.
mod support;
use grokbot_wrapper::{features::forms, App, Provider};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const DAY_MS: i64 = 86_400_000;
const TRAPS: [&str; 7] = [
    "PIEGE-MESSAGE-81",
    "PIEGE-ARGUMENT-81",
    "PIEGE-SORTIE-81",
    "PIEGE-COFFRE-81-aaaaaaaa",
    "PIEGE-JETON-ACCES-81",
    "PIEGE-JETON-RENOUVELLEMENT-81",
    "PIEGE-RAISON-81",
];

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
        let guard = grokbot_test_support::TestDir::new("ezerbot-turns");
        let root = guard.to_path_buf();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let path = root.join("data/workspace.sqlite");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":TRAPS[4],"refresh_token":TRAPS[5],"expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api("POST", "/api/conversations", br#"{"title":"Journal"}"#)
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
    fn script(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }
    /// Un tour : le modèle appelle `calls`, puis répond par le texte par défaut.
    fn turn(&mut self, id: &str, body: &str, calls: &[(&str, &str, Value)], usage: Value) {
        let mut events: Vec<Value> = calls.iter().map(|(call_id, name, args)| json!({"type":"response.output_item.done","item":{"id":format!("fc-{call_id}"),"type":"function_call","status":"completed","call_id":call_id,"name":name,"arguments":args.to_string()}})).collect();
        events.push(json!({"type":"response.completed","response":{"usage":usage}}));
        self.script(events);
        let chat = self.chat.clone();
        self.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":id,"body":body}),
        );
    }
    fn run_id(&self, id: &str) -> String {
        self.db().query_row("SELECT run_id FROM wrapper_engine_runs WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap()
    }
    fn finish(&mut self, id: &str) {
        for _ in 0..300 {
            self.app.tick(now());
            let status: Option<String> = self.db().query_row("SELECT json_extract(record,'$.status') FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get(0)).ok();
            if matches!(status.as_deref(), Some("finished" | "failed" | "cancelled"))
                && self.provider.status()["titling"] == false
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("le tour {id} ne se termine pas");
    }
    fn logged(&self, id: &str) -> Value {
        let run = self.run_id(id);
        wait("entrée du journal", || {
            self.db()
                .query_row(
                    "SELECT count(*) FROM wrapper_turn_log WHERE run_id=?1",
                    [&run],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
                == 1
        });
        let entry: String = self
            .db()
            .query_row(
                "SELECT entry FROM wrapper_turn_log WHERE run_id=?1",
                [&run],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&entry).unwrap()
    }
    fn dump(&self) -> String {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT run_id || '|' || at_ms || '|' || entry FROM wrapper_turn_log")
            .unwrap();
        statement
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn local_shell(&mut self) {
        self.post(
            "/api/local/bot",
            json!({"bot_id":"grokbot","shell":true,"files":true}),
        );
    }
    fn shell(&self, command: &str) -> Value {
        json!({"command":command,"cwd":self.root(),"timeout_ms":2000,"output_limit":1024})
    }
    fn tool_outcomes(&self, id: &str) -> Vec<Value> {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT payload FROM engine_run_events WHERE run_id=?1 AND kind='tool_result' ORDER BY seq")
            .unwrap();
        statement
            .query_map([self.run_id(id)], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|p| serde_json::from_str(&p.unwrap()).unwrap())
            .collect()
    }
    fn summary(&mut self, day: &str) -> Result<Value, u16> {
        self.app
            .api("GET", &format!("/api/turns/summary?day={day}"), b"")
            .map_err(|e| e.status)
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn entry(at_ms: i64, tokens: Value, tools: Vec<(&str, Option<i64>, &str, &str)>) -> (i64, String) {
    let tools: Vec<Value> = tools
        .into_iter()
        .map(|(name, duration, result, code)| json!({"name":name,"started_ms":duration.map(|_| at_ms - 1000),"duration_ms":duration,"result":result,"code":code}))
        .collect();
    (
        at_ms,
        json!({"at_ms":at_ms,"bot_id":"grokbot","source":"chat","turn":"0123456789abcdef","model":"gpt-test","effort":null,"status":"finished","turn_end":"finished","error_kind":null,"duration_ms":1500,"model_calls":2,"tool_calls":tools.len(),"tokens":tokens,"tools":tools}).to_string(),
    )
}
fn seed(f: &Fixture, rows: &[(i64, String)]) {
    for (index, (at_ms, text)) in rows.iter().enumerate() {
        f.db()
            .execute(
                "INSERT INTO wrapper_turn_log(run_id,at_ms,entry) VALUES (?1,?2,?3)",
                params![format!("seed-{index}"), at_ms, text],
            )
            .unwrap();
    }
}
/// Instant local d'un jour et d'une heure, calculé par SQLite comme le fait le résumé.
fn local_ms(f: &Fixture, day: &str, time: &str) -> i64 {
    f.db()
        .query_row(
            "SELECT CAST(strftime('%s', ?1 || ' ' || ?2, 'utc') AS INTEGER) * 1000",
            [day, time],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_turn_with_two_real_tool_calls_is_logged_with_its_tools_and_tokens() {
    let mut f = Fixture::new();
    f.local_shell();
    std::fs::write(f.root().join("note.txt"), "bonjour").unwrap();
    let path = f.root().join("note.txt");
    f.server.seen.lock().unwrap().usage = Some(
        json!({"input_tokens":50,"output_tokens":7,"input_tokens_details":{"cached_tokens":5}}),
    );
    f.turn(
        "deux-outils",
        "Lis la note puis compte",
        &[
            ("lire", "read_file", json!({"path":path})),
            ("compter", "run_shell", f.shell("echo $((40+2))")),
        ],
        json!({"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":20}}),
    );
    f.finish("deux-outils");
    let turn = f.logged("deux-outils");
    assert_eq!(turn["bot_id"], "grokbot");
    assert_eq!(turn["source"], "chat");
    assert_eq!(turn["status"], "finished");
    assert_eq!(turn["error_kind"], Value::Null);
    assert_eq!(turn["model_calls"], 2);
    assert_eq!(turn["tool_calls"], 2);
    assert_eq!(turn["tokens"], json!({"input":170,"output":37,"cached":25}));
    assert!(turn["duration_ms"].is_u64() && turn["at_ms"].is_u64());
    assert_eq!(turn["turn"].as_str().unwrap().len(), 16);
    let tools = turn["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    for (tool, name) in tools.iter().zip(["read_file", "run_shell"]) {
        assert_eq!(tool["name"], name);
        assert_eq!(tool["result"], "ok");
        assert_eq!(tool["code"], "ok");
        assert!(tool["duration_ms"].is_u64(), "{name} n'est pas chronométré");
        assert!(tool["started_ms"].is_u64());
    }
}

#[test]
fn a_failed_tool_and_a_denied_approval_appear_with_their_code() {
    let mut f = Fixture::new();
    f.local_shell();
    let missing = f.root().join("absent.txt");
    f.turn(
        "en-erreur",
        "Lis un fichier absent",
        &[("absent", "read_file", json!({"path":missing}))],
        json!({"input_tokens":10,"output_tokens":1}),
    );
    f.finish("en-erreur");
    assert_eq!(
        f.tool_outcomes("en-erreur")[0]["outcome"]["status"],
        "failed"
    );
    let tool = f.logged("en-erreur")["tools"][0].clone();
    assert_eq!(tool["name"], "read_file");
    assert_eq!(tool["result"], "erreur");
    assert_eq!(tool["code"], "failed");

    f.post("/api/yolo/mode", json!({"mode":"ask"}));
    f.turn(
        "refuse",
        "Lance une commande",
        &[("cmd", "run_shell", f.shell("echo non"))],
        json!({"input_tokens":10,"output_tokens":1}),
    );
    let run = f.run_id("refuse");
    wait("carte d'approbation", || {
        grokbot_forms::requests::list_for_run(&f.db(), &run)
            .map(|cards| !cards.is_empty())
            .unwrap_or(false)
    });
    let card = grokbot_forms::requests::list_for_run(&f.db(), &run)
        .unwrap()
        .remove(0);
    f.post("/api/approvals/resolve", json!({"entry_id":card.entry_id,"submission_id":"decision","conversation_id":card.route.target_conversation_id,"decision":"deny"}));
    f.finish("refuse");
    let tool = f.logged("refuse")["tools"][0].clone();
    assert_eq!(tool["name"], "run_shell");
    assert_eq!(tool["result"], "approbation refusée");
    assert_eq!(tool["code"], "approval_denied");
}

#[test]
fn a_turn_stopped_while_it_waits_for_an_approval_is_logged() {
    let mut f = Fixture::new();
    f.local_shell();
    f.post("/api/yolo/mode", json!({"mode":"ask"}));
    f.turn(
        "attente",
        "Lance une commande",
        &[("cmd", "run_shell", f.shell("echo attente"))],
        json!({"input_tokens":10,"output_tokens":1}),
    );
    let run = f.run_id("attente");
    wait("carte d'approbation", || {
        grokbot_forms::requests::list_for_run(&f.db(), &run)
            .map(|cards| !cards.is_empty())
            .unwrap_or(false)
    });
    let chat = f.chat.clone();
    f.post(
        "/api/stop",
        json!({"conversation_id":chat,"message_id":"attente"}),
    );
    let turn = f.logged("attente");
    assert_eq!(turn["status"], "cancelled");
    assert_eq!(turn["tools"][0]["code"], "not_executed");
    assert_eq!(turn["tools"][0]["result"], "annulé");
}

#[test]
fn the_log_keeps_no_message_argument_output_reason_secret_or_token() {
    let mut f = Fixture::new();
    f.local_shell();
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set_for_bot("grokbot", "CLE_PIEGE", "Authentifier", TRAPS[3])
        .unwrap();
    std::fs::write(f.root().join("sortie.txt"), TRAPS[2]).unwrap();
    let absent = f.root().join(format!("{}-absent.txt", TRAPS[1]));
    f.turn(
        "pieges",
        &format!("Message secret {}", TRAPS[0]),
        &[
            (
                "sortie",
                "run_shell",
                f.shell(&format!("cat sortie.txt; : {}", TRAPS[1])),
            ),
            ("raison", "read_file", json!({"path":absent})),
            (
                "inconnu",
                &format!("outil_{}", TRAPS[6]),
                json!({"x":TRAPS[1]}),
            ),
            (
                "texte",
                &format!("autre_{}", TRAPS[6]),
                json!("pas un objet"),
            ),
        ],
        json!({"input_tokens":10,"output_tokens":1}),
    );
    f.finish("pieges");
    let outcomes = json!(f.tool_outcomes("pieges")).to_string();
    assert!(
        outcomes.contains(TRAPS[2]) && outcomes.contains(TRAPS[6]),
        "le moteur voit bien la sortie et la raison pièges : {outcomes}"
    );
    let turn = f.logged("pieges");
    assert_eq!(turn["tools"].as_array().unwrap().len(), 4);
    for unknown in [2, 3] {
        assert_eq!(turn["tools"][unknown]["name"], "-");
        assert_eq!(turn["tools"][unknown]["code"], "unknown_tool");
    }
    let day: String = f
        .db()
        .query_row(
            "SELECT date(?1/1000,'unixepoch','localtime')",
            [turn["at_ms"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let summary = f.summary(&day).unwrap();
    assert_eq!(summary["turns"], 1);
    let everything = format!("{}\n{summary}", f.dump());
    for trap in TRAPS {
        assert!(!everything.contains(trap), "le journal contient {trap}");
    }
}

#[test]
fn entries_older_than_thirty_days_are_purged_at_startup() {
    let mut f = Fixture::new();
    let now = now() as i64;
    let rows = [
        entry(now - 31 * DAY_MS, Value::Null, vec![]),
        entry(now - 29 * DAY_MS, Value::Null, vec![]),
    ];
    seed(&f, &rows);
    let other: i64 = f
        .db()
        .query_row("SELECT count(*) FROM wrapper_conversations", [], |r| {
            r.get(0)
        })
        .unwrap();
    f.reopen();
    let kept: Vec<String> = {
        let db = f.db();
        let mut statement = db.prepare("SELECT run_id FROM wrapper_turn_log").unwrap();
        statement
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(kept, ["seed-1"]);
    let after: i64 = f
        .db()
        .query_row("SELECT count(*) FROM wrapper_conversations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(after, other, "les autres tables restent intactes");
}

#[test]
fn the_daily_summary_gives_known_numbers_for_the_local_day_only() {
    let mut f = Fixture::new();
    let day: String = f
        .db()
        .query_row("SELECT date('now','localtime','-2 days')", [], |r| r.get(0))
        .unwrap();
    let next: String = f
        .db()
        .query_row("SELECT date(?1,'+1 day')", [&day], |r| r.get(0))
        .unwrap();
    let before: String = f
        .db()
        .query_row("SELECT date(?1,'-1 day')", [&day], |r| r.get(0))
        .unwrap();
    let tokens = |input, output, cached| json!({"input":input,"output":output,"cached":cached});
    let rows = [
        entry(
            local_ms(&f, &day, "00:00:00"),
            tokens(100, 10, 40),
            vec![
                ("read_file", Some(10), "ok", "ok"),
                ("run_shell", Some(30), "ok", "ok"),
                ("run_shell", Some(50), "erreur", "failed"),
            ],
        ),
        entry(
            local_ms(&f, &day, "12:00:00"),
            Value::Null,
            vec![
                ("run_shell", Some(20), "ok", "ok"),
                ("run_shell", Some(40), "ok", "ok"),
                ("run_shell", Some(5), "erreur", "failed"),
                ("read_file", Some(100), "ok", "ok"),
                ("-", None, "refusé", "unknown_tool"),
            ],
        ),
        entry(
            local_ms(&f, &day, "23:59:59"),
            tokens(200, 20, 0),
            vec![
                ("run_shell", None, "approbation refusée", "approval_denied"),
                ("read_file", Some(70), "ok", "ok"),
            ],
        ),
        entry(
            local_ms(&f, &next, "00:00:00"),
            tokens(7000, 700, 70),
            vec![("run_shell", Some(9999), "ok", "ok")],
        ),
        entry(
            local_ms(&f, &before, "23:59:59"),
            tokens(8000, 800, 80),
            vec![("run_shell", Some(9999), "ok", "ok")],
        ),
    ];
    seed(&f, &rows);
    // run_shell : 6 appels dont 5 chronométrés (5, 20, 30, 40, 50), 2 en erreur ou refusés.
    // read_file : 3 appels chronométrés (10, 70, 100).
    assert_eq!(
        f.summary(&day).unwrap(),
        json!({
            "day": day,
            "turns": 3,
            "tokens": {"input":300,"output":30,"cached":40,"unmeasured_turns":1},
            "most_used": [{"name":"run_shell","calls":6},{"name":"read_file","calls":3},{"name":"-","calls":1}],
            "slowest": [
                {"name":"read_file","calls":3,"median_ms":70,"p95_ms":100},
                {"name":"run_shell","calls":5,"median_ms":30,"p95_ms":50}
            ],
            "errors": [
                {"name":"run_shell","code":"failed","count":2},
                {"name":"-","code":"unknown_tool","count":1},
                {"name":"run_shell","code":"approval_denied","count":1}
            ]
        })
    );
    for invalid in ["hier", "2026-13-45", "2026-3-1", "2026-02-30", ""] {
        assert_eq!(f.summary(invalid), Err(400), "{invalid:?}");
    }
    let empty = f.summary("1999-01-01").unwrap();
    assert_eq!(empty["turns"], 0);
    assert_eq!(empty["most_used"], json!([]));
}

#[test]
fn the_command_line_prints_the_same_summary_as_the_route() {
    let mut f = Fixture::new();
    let day: String = f
        .db()
        .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
        .unwrap();
    let early = local_ms(&f, &day, "00:00:01");
    seed(
        &f,
        &[entry(
            early,
            json!({"input":3,"output":2,"cached":1}),
            vec![("read_file", Some(12), "ok", "ok")],
        )],
    );
    let path = f.path.clone();
    let run = |day: &str| {
        std::process::Command::new(env!("CARGO_BIN_EXE_grokbot-wrapper"))
            .args(["--turns-summary", day, "--db"])
            .arg(&path)
            .output()
            .unwrap()
    };
    let printed = run(&day);
    assert!(printed.status.success());
    let printed: Value = serde_json::from_slice(&printed.stdout).unwrap();
    assert_eq!(printed["turns"], 1);
    assert_eq!(printed, f.summary(&day).unwrap());
    assert!(!run("hier").status.success());
}
