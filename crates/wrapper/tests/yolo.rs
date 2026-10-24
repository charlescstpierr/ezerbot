//! Mode YOLO : défaut, migration, surcharge par bot, retour à « Demander » et garde-fous.
//! Tous les effets restent dans des dossiers jetables de test.
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
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const PAYMENTS: &str = "Je ne peux pas faire ça : les paiements sont interdits aux bots, même avec ton accord. Tu peux le faire toi-même.";
const PROTECTED: &str =
    "Mode YOLO : les données et les identifiants d’Ezerbot restent hors de portée des outils.";

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    chat: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-yolo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
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
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api("POST", "/api/conversations", br#"{"title":"YOLO"}"#)
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
    fn root(&self) -> PathBuf {
        self.path.parent().unwrap().parent().unwrap().to_path_buf()
    }
    fn data(&self) -> PathBuf {
        self.path.parent().unwrap().to_path_buf()
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
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn yolo(&mut self) -> Value {
        self.state()["yolo"].clone()
    }
    fn mode(&mut self, mode: &str) {
        self.post("/api/yolo/mode", json!({"mode": mode}));
    }
    fn bot_mode(&mut self, bot: &str, mode: &str) {
        self.post("/api/yolo/bot", json!({"bot_id": bot, "mode": mode}));
    }
    fn local(&mut self, bot: &str, shell: bool, files: bool) {
        self.post(
            "/api/local/bot",
            json!({"bot_id":bot,"shell":shell,"files":files}),
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
    fn send_in(&mut self, chat: &str, id: &str, name: &str, args: Value) {
        self.script(vec![json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":args.to_string()}}),json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":id,"body":"Exécute cet outil"}),
        );
    }
    fn send(&mut self, id: &str, name: &str, args: Value) {
        let chat = self.chat.clone();
        self.send_in(&chat, id, name, args);
    }
    fn run(&self, id: &str) -> Run {
        let text:String=self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        serde_json::from_str(&text).unwrap()
    }
    fn requests(&self, id: &str) -> Vec<HumanInputRequest> {
        requests::list_for_run(&self.db(), &self.run(id).run_id).unwrap()
    }
    fn request(&self, id: &str) -> HumanInputRequest {
        wait("approval and drained owner", || {
            self.run(id).status == RunStatus::AwaitingApproval
                && self.provider.status()["titling"] == false
        });
        self.requests(id).remove(0)
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
    fn outcome(&self, id: &str) -> Value {
        store::events(&self.db(), &self.run(id).run_id, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == id)
            .unwrap()
            .payload["outcome"]
            .clone()
    }
    fn shell(&self, command: &str) -> Value {
        json!({"command":command,"cwd":self.root(),"timeout_ms":2000,"output_limit":1024})
    }
    fn card(&mut self, chat: &str, call_id: &str) -> Value {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == chat)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "tool" && m["payload"]["call"]["call_id"] == call_id)
            .unwrap_or_else(|| panic!("carte absente pour {call_id}"))
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
    /// Un outil lancé sans réglage préalable, terminé sans carte.
    fn runs_without_a_card(&mut self, id: &str, name: &str, args: Value) -> Value {
        self.send(id, name, args);
        self.finish(id);
        assert!(self.requests(id).is_empty(), "{id} a demandé une carte");
        self.outcome(id)
    }
}
impl Fixture {
    /// Un connecteur de caisse fictif, connecté avec la carte que le YOLO laisse en place.
    /// Chaque appel qui l'atteint est noté dans le fichier rendu.
    fn payments_connector(&mut self) -> PathBuf {
        let calls = self.root().join("appels.txt");
        let fixture = format!(
            "{}/tests/fixtures/mcp_server.py",
            env!("CARGO_MANIFEST_DIR")
        );
        let server = self.post("/api/mcp/add", json!({"name":"Caisse","command":"python3","arguments":[fixture,"payments"],"environment":{"CALL_LOG":calls}}))["id"].as_str().unwrap().to_owned();
        self.post(
            "/api/mcp/select",
            json!({"id":server,"bot_id":"grokbot","enabled":true}),
        );
        let run = self.post("/api/mcp/restart", json!({"id":server}))["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        wait("carte de connexion", || {
            !requests::list_for_run(&self.db(), &run).unwrap().is_empty()
        });
        let card = requests::list_for_run(&self.db(), &run).unwrap().remove(0);
        self.post("/api/approvals/resolve", json!({"entry_id":card.entry_id,"conversation_id":card.route.target_conversation_id,"submission_id":"connexion","decision":"allow_once"}));
        self.app.tick(0);
        wait("connecteur prêt", || {
            self.state()["mcp"]["servers"][0]["status"] == "connected"
        });
        calls
    }
    fn connector_tool(&mut self, name: &str) -> String {
        self.state()["mcp"]["servers"][0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["namespace"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn forbidden_log(&mut self) -> Vec<Value> {
        let log = self.yolo()["log"].as_array().unwrap().clone();
        log.into_iter()
            .filter(|e| e["source"] == "forbidden")
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.root());
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", "--", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn a_new_workspace_starts_in_yolo_and_runs_a_command_and_a_file_write_without_a_card() {
    let mut f = Fixture::new();
    let yolo = f.yolo();
    assert_eq!(yolo["mode"], "yolo");
    assert_eq!(yolo["bots"][0]["effective"], "yolo");
    // Aucun réglage Terminal ou Fichiers n'a été touché : le mode les recouvre.
    let out = f.runs_without_a_card("cmd", "run_shell", f.shell("printf bonjour-yolo"));
    assert_eq!(out["status"], "done");
    assert_eq!(out["output"]["stdout"], "bonjour-yolo");
    let target = f.root().join("ecrit-par-yolo.txt");
    let out = f.runs_without_a_card(
        "write",
        "write_file",
        json!({"path":target,"content":"fait automatiquement","expected_sha256":null}),
    );
    assert_eq!(out["status"], "done");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "fait automatiquement"
    );
    let out = f.runs_without_a_card("read", "read_file", json!({"path":target}));
    assert_eq!(out["output"]["content"], "fait automatiquement");
    // La carte du fil dit qui a autorisé, et le journal garde chaque action.
    let chat = f.chat.clone();
    for id in ["cmd", "write", "read"] {
        assert_eq!(
            f.card(&chat, id)["payload"]["authorized_by"],
            json!({"kind":"yolo","source":"global"})
        );
    }
    let log = f.yolo()["log"].clone();
    let tools: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["tool"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["read_file", "write_file", "run_shell"]);
    assert!(log[2]["summary"]
        .as_str()
        .unwrap()
        .contains("printf bonjour-yolo"));
    assert!(log[1]["summary"]
        .as_str()
        .unwrap()
        .contains("ecrit-par-yolo.txt"));
    assert_eq!(log[0]["source"], "global");
}

#[test]
fn the_migration_turns_yolo_on_for_an_existing_workspace_without_touching_its_settings() {
    let mut f = Fixture::new();
    f.local("grokbot", true, false);
    // Une base d'avant le Mode YOLO n'a aucune de ses tables.
    f.db()
        .execute_batch(
            "DROP TABLE wrapper_yolo_settings; DROP TABLE wrapper_yolo_bots; DROP TABLE wrapper_yolo_log;",
        )
        .unwrap();
    f.reopen();
    assert_eq!(f.yolo()["mode"], "yolo");
    let stored: (bool, bool) = f
        .db()
        .query_row(
            "SELECT shell,files FROM wrapper_local_bots WHERE bot_id='grokbot'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (true, false),
        "les réglages d'avant restent intacts"
    );
    // Fichiers était désactivé : le mode l'active sans carte.
    let target = f.root().join("lu.txt");
    std::fs::write(&target, "contenu").unwrap();
    let out = f.runs_without_a_card("lecture", "read_file", json!({"path":target}));
    assert_eq!(out["output"]["content"], "contenu");
    // Annuler : un clic, et les réglages d'avant reprennent (Fichiers redevient désactivé).
    f.mode("ask");
    f.send("lecture-2", "read_file", json!({"path":target}));
    f.finish("lecture-2");
    assert_eq!(f.outcome("lecture-2")["status"], "failed");
    assert!(f.outcome("lecture-2")["reason"]
        .as_str()
        .unwrap()
        .contains("désactivé"));
    // Le choix « Demander avant d'agir » survit au redémarrage : la migration ne le rejoue pas.
    f.reopen();
    assert_eq!(f.yolo()["mode"], "ask");
    assert_eq!(f.yolo()["bots"][0]["effective"], "ask");
}

#[test]
fn switching_back_to_ask_brings_the_cards_back_and_one_click_returns_to_yolo() {
    let mut f = Fixture::new();
    f.mode("ask");
    f.local("grokbot", true, true);
    f.send("ask-1", "run_shell", f.shell("printf demande"));
    let request = f.request("ask-1");
    assert_eq!(request.status, requests::RequestStatus::Pending);
    f.post("/api/approvals/resolve", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":request.route.target_conversation_id,"decision":"deny"}));
    f.finish("ask-1");
    assert_eq!(
        f.yolo()["log"],
        json!([]),
        "rien n'est auto-approuvé en mode Demander"
    );
    f.mode("yolo");
    let out = f.runs_without_a_card("yolo-1", "run_shell", f.shell("printf auto"));
    assert_eq!(out["output"]["stdout"], "auto");
    assert_eq!(f.yolo()["log"].as_array().unwrap().len(), 1);
}

#[test]
fn a_bot_override_beats_the_global_switch_in_both_directions() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"pair","name":"Pair","color":"cyan"}),
    );
    let pair_chat = f.post(
        "/api/conversations",
        json!({"bot_id":"pair","title":"Pair"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.local("pair", true, true);
    // Global YOLO, Pair demande : seule Pair affiche une carte.
    f.bot_mode("pair", "ask");
    let yolo = f.yolo();
    let effective = |yolo: &Value, bot: &str| {
        yolo["bots"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["bot_id"] == bot)
            .unwrap()["effective"]
            .clone()
    };
    assert_eq!(effective(&yolo, "grokbot"), "yolo");
    assert_eq!(effective(&yolo, "pair"), "ask");
    f.send_in(&pair_chat, "pair-ask", "run_shell", f.shell("printf pair"));
    let request = f.request("pair-ask");
    f.post("/api/approvals/resolve", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":request.route.target_conversation_id,"decision":"deny"}));
    f.finish("pair-ask");
    let out = f.runs_without_a_card("main-auto", "run_shell", f.shell("printf principal"));
    assert_eq!(out["output"]["stdout"], "principal");
    assert_eq!(
        f.card(&f.chat.clone(), "main-auto")["payload"]["authorized_by"]["source"],
        "global"
    );
    // Global « Demander », Pair en YOLO : le contraire.
    f.mode("ask");
    f.bot_mode("pair", "yolo");
    f.local("grokbot", true, true);
    f.send_in(
        &pair_chat,
        "pair-auto",
        "run_shell",
        f.shell("printf pair-auto"),
    );
    f.finish("pair-auto");
    assert!(f.requests("pair-auto").is_empty());
    assert_eq!(
        f.card(&pair_chat, "pair-auto")["payload"]["authorized_by"],
        json!({"kind":"yolo","source":"bot"})
    );
    f.send("main-ask", "run_shell", f.shell("printf demande"));
    let request = f.request("main-ask");
    f.post("/api/approvals/resolve", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":request.route.target_conversation_id,"decision":"deny"}));
    f.finish("main-ask");
    // « Hériter » retire la surcharge.
    f.bot_mode("pair", "inherit");
    assert_eq!(effective(&f.yolo(), "pair"), "ask");
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM wrapper_yolo_bots", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn ezerbot_data_and_credentials_stay_out_of_reach_even_in_yolo() {
    let mut f = Fixture::new();
    let auth = f.path.with_extension("chatgpt-auth.json");
    let alias = f.root().join("copie-dure.json");
    std::fs::hard_link(&auth, &alias).unwrap();
    let data = f.data();
    let secret_target = data.join("jamais-ecrit.txt");
    let attempts = [
        ("lire-jeton", "read_file", json!({"path":auth})),
        ("lire-lien-dur", "read_file", json!({"path":alias})),
        ("lister-donnees", "list_directory", json!({"path":data})),
        (
            "ecrire-donnees",
            "write_file",
            json!({"path":secret_target,"content":"non","expected_sha256":null}),
        ),
        (
            "cat-base",
            "run_shell",
            f.shell(&format!("cat '{}'", f.path.display())),
        ),
        (
            "cat-jeton-nu",
            "run_shell",
            f.shell("cat data/workspace.chatgpt-auth.json"),
        ),
        (
            "support-applicatif",
            "run_shell",
            f.shell("cd ~/Library/\"Application Support\" && ls Grokbot"),
        ),
        (
            "trousseau",
            "run_shell",
            f.shell("security find-generic-password -s ezerbot -w"),
        ),
        (
            "trousseau-chemin",
            "run_shell",
            f.shell("/usr/bin/security dump-keychain"),
        ),
    ];
    for (id, tool, args) in attempts {
        f.send(id, tool, args);
        f.finish(id);
        assert!(
            f.requests(id).is_empty(),
            "{id} ne doit pas ouvrir de carte"
        );
        let out = f.outcome(id);
        assert_eq!(out["status"], "failed", "{id}: {out}");
        assert_eq!(out["reason"], PROTECTED, "{id}");
        assert!(
            !out.to_string().contains("fake-access"),
            "{id} a laissé fuiter un jeton"
        );
    }
    let mut cwd_in_data = f.shell("ls");
    cwd_in_data["cwd"] = json!(data);
    f.send("cwd-donnees", "run_shell", cwd_in_data);
    f.finish("cwd-donnees");
    assert_eq!(f.outcome("cwd-donnees")["reason"], PROTECTED);
    assert!(!secret_target.exists());
    assert_eq!(
        f.yolo()["log"],
        json!([]),
        "un refus du garde-fou n'est pas une action faite automatiquement"
    );
    // Le garde-fou ne bloque pas le travail ordinaire à côté.
    let out = f.runs_without_a_card("ordinaire", "run_shell", f.shell("printf utile"));
    assert_eq!(out["output"]["stdout"], "utile");
}

#[test]
fn relative_paths_are_judged_where_they_lead_not_by_their_spelling() {
    let mut f = Fixture::new();
    // Le dossier de travail est le parent des données : « data/… » désigne la base.
    let attempts = [
        ("relatif", "cat data/anything.txt"),
        ("cd-donnees", "cd data && ls"),
        ("point-point", "ls ./data/../data"),
        ("majuscules", "ls DATA/x"),
        ("redirection", "echo x > data/intrus.txt"),
        ("option", "tail --file=data/journal.log"),
        ("joker", "cat da*/anything.txt"),
        ("joker-un", "ls d?ta"),
        ("joker-classe", "ls [d]ata/x"),
        ("joker-jeton", "cat d*/workspace.chatgpt-auth.j*n"),
        ("joker-accolades", "ls dat{a,o}"),
    ];
    for (id, command) in attempts {
        f.send(id, "run_shell", f.shell(command));
        f.finish(id);
        let out = f.outcome(id);
        assert_eq!(out["reason"], PROTECTED, "{id}: {out}");
    }
    assert!(!f.data().join("intrus.txt").exists());
    // Le même mot relatif, juste à côté des données, reste permis.
    let out = f.runs_without_a_card(
        "voisin",
        "run_shell",
        f.shell("ls . && cat ../hors-donnees"),
    );
    assert_eq!(out["status"], "done", "{out}");
    let out = f.runs_without_a_card("joker-voisin", "run_shell", f.shell("ls *.txt x?"));
    assert_eq!(out["status"], "done", "{out}");
}

#[test]
fn a_command_that_reaches_the_data_without_naming_it_reads_nothing() {
    let mut f = Fixture::new();
    // Le dossier de travail est le parent des données.
    let attempts = [
        ("grep-recursif", "grep -r refresh ."),
        ("tar-parent", "tar cf - . | strings"),
        (
            "variable-joker",
            "D=da; cat ${D}*/workspace.chatgpt-auth.json",
        ),
    ];
    for (id, command) in attempts {
        let out = f.runs_without_a_card(id, "run_shell", f.shell(command));
        assert!(
            !out["output"].to_string().contains("fake-refresh"),
            "{id} a laissé fuiter un jeton : {out}"
        );
    }
    std::fs::write(f.root().join("voisin.txt"), "utile").unwrap();
    let out = f.runs_without_a_card("voisin", "run_shell", f.shell("grep -r utile ."));
    assert!(
        out["output"]["stdout"].to_string().contains("utile"),
        "{out}"
    );
}

/// Le trousseau est refusé par le noyau au shell YOLO, même quand le texte de la
/// commande ne nomme jamais l'exécutable (`S=security; "$S" …`).
#[cfg(target_os = "macos")]
#[test]
fn the_keychain_is_refused_by_the_kernel_however_the_command_is_spelled() {
    let mut f = Fixture::new();
    // Hors YOLO le même texte lit le trousseau de l'utilisateur : c'est la référence.
    let plain = std::process::Command::new("/bin/sh")
        .args(["-c", r#"S=security; "$S" default-keychain"#])
        .output()
        .unwrap();
    if !plain.status.success() {
        return; // aucun trousseau lisible ici (ex. session sans trousseau) : rien à comparer
    }
    assert!(String::from_utf8_lossy(&plain.stdout).contains("keychain"));
    for (id, command) in [
        ("var", r#"S=security; "$S" default-keychain"#),
        (
            "script",
            r#"printf '#!/bin/sh\nexec /usr/bin/security list-keychains\n' > lk.sh; sh lk.sh"#,
        ),
        (
            "lien",
            r#"ln -s /usr/bin/security sec; ./sec default-keychain"#,
        ),
    ] {
        let out = f.runs_without_a_card(id, "run_shell", f.shell(command));
        assert_eq!(out["status"], "done", "{id}: {out}");
        let shown = out["output"]["stdout"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        assert!(
            !shown.contains("login.keychain") && !shown.contains("system.keychain"),
            "{id} a lu le trousseau : {out}"
        );
    }
}

/// Un auditeur local qui compte les connexions reçues.
fn listener() -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = hits.clone();
    std::thread::spawn(move || {
        for stream in socket.incoming() {
            if stream.is_ok() {
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
    (port, hits)
}

#[test]
fn a_command_that_names_the_wrapper_port_is_refused_before_it_runs() {
    let mut f = Fixture::new();
    let (port, hits) = listener();
    grokbot_wrapper::features::yolo::register_control_port(port);
    for (id, command) in [
        (
            "curl",
            format!("curl -s --max-time 2 http://127.0.0.1:{port}/api/secrets"),
        ),
        ("nc", format!("nc -z localhost {port}")),
        (
            "script",
            format!("printf 'GET /' > req.txt; cat req.txt | nc 127.0.0.1 {port}"),
        ),
    ] {
        f.send(id, "run_shell", f.shell(&command));
        f.finish(id);
        assert!(f.requests(id).is_empty());
        assert_eq!(f.outcome(id)["reason"], PROTECTED, "{id}");
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    // Un autre nombre qui contient les mêmes chiffres n'est pas ce port.
    let out = f.runs_without_a_card("autre", "run_shell", f.shell(&format!("printf {port}0")));
    assert_eq!(out["status"], "done", "{out}");
}

/// Le texte ne nomme pas le port (calculé par le shell) : c'est le noyau qui refuse
/// toute connexion locale au shell YOLO et à ses enfants.
#[cfg(target_os = "macos")]
#[test]
fn the_kernel_refuses_a_local_connection_even_when_the_port_is_computed() {
    let mut f = Fixture::new();
    let (port, hits) = listener();
    let low = port - 1000;
    for (id, command) in [
        (
            "calcule-nc",
            format!("P=$(({low}+1000)); nc -z 127.0.0.1 $P; echo nc=$?"),
        ),
        (
            "calcule-curl",
            format!("P=$(({low}+1000)); curl -s --max-time 2 http://localhost:$P/; echo curl=$?"),
        ),
    ] {
        let out = f.runs_without_a_card(id, "run_shell", f.shell(&command));
        assert_eq!(out["status"], "done", "{id}: {out}");
        let shown = out["output"]["stdout"].as_str().unwrap_or("");
        assert!(
            !shown.contains("nc=0") && !shown.contains("curl=0"),
            "{id}: {out}"
        );
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "le shell a atteint un service local"
    );
    // Sans le sandbox (hors YOLO), la même connexion aboutit : la référence.
    let plain = std::process::Command::new("/usr/bin/nc")
        .args(["-z", "127.0.0.1", &port.to_string()])
        .status()
        .unwrap();
    assert!(plain.success());
}

#[test]
fn a_trapped_wildcard_in_a_command_does_not_hold_the_admission_for_long() {
    let mut f = Fixture::new();
    let trap = format!("ls \"{}x/*/*/z\"", "*".repeat(100));
    let started = std::time::Instant::now();
    f.send("piege", "run_shell", f.shell(&trap));
    f.finish("piege");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(f.requests("piege").is_empty());
}

#[test]
fn a_global_refusal_of_the_shell_also_stops_commands_no_always_rule_could_cover() {
    let mut f = Fixture::new();
    f.db()
        .execute(
            "INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,'run_shell','','\"global\"',0)",
            params![f.chat],
        )
        .unwrap();
    // Une commande composée n'est jamais mémorisable, mais le refus permanent la couvre.
    for (id, command) in [("composee", "pwd && ls"), ("simple", "printf non")] {
        f.send(id, "run_shell", f.shell(command));
        f.finish(id);
        assert!(f.requests(id).is_empty());
        assert!(
            f.outcome(id)["reason"]
                .as_str()
                .unwrap()
                .contains("autorisation permanente"),
            "{id}"
        );
    }
}

#[test]
fn a_permanent_refusal_written_by_the_owner_still_wins_in_yolo() {
    let mut f = Fixture::new();
    f.db()
        .execute(
            "INSERT INTO forms_approval_rules VALUES ('grokbot','grokbot',?1,'write_file','','\"global\"',0)",
            params![f.chat],
        )
        .unwrap();
    let target = f.root().join("refuse.txt");
    f.send(
        "refus",
        "write_file",
        json!({"path":target,"content":"non","expected_sha256":null}),
    );
    f.finish("refus");
    assert!(f.requests("refus").is_empty());
    assert!(f.outcome("refus")["reason"]
        .as_str()
        .unwrap()
        .contains("autorisation permanente"));
    assert!(!target.exists());
}

#[test]
fn stop_still_interrupts_a_running_command_and_a_background_task_in_yolo() {
    let mut f = Fixture::new();
    f.send(
        "premier-plan",
        "run_shell",
        f.shell("(sleep 1; touch fuite-premier-plan) & echo ready; wait"),
    );
    for _ in 0..100 {
        f.app.tick(now());
        let started: i64 = f
            .db()
            .query_row(
                "SELECT count(*) FROM engine_external_tools WHERE call_id='premier-plan'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if started > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let begun = std::time::Instant::now();
    f.post(
        "/api/stop",
        json!({"conversation_id":f.chat,"message_id":"premier-plan"}),
    );
    f.finish("premier-plan");
    assert!(begun.elapsed() < Duration::from_secs(1));
    let mut background = f.shell("echo $$ > fond.pid; sleep 20; touch fuite-fond");
    background["background"] = json!(true);
    f.send("fond", "run_shell", background);
    f.finish("fond");
    let pid_file = f.root().join("fond.pid");
    wait("pid de la tâche de fond", || pid_file.exists());
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(process_alive(pid));
    f.post(
        "/api/local/tasks/stop",
        json!({"run_id": f.run("fond").run_id, "call_id": "fond"}),
    );
    wait("tâche de fond arrêtée", || !process_alive(pid));
    std::thread::sleep(Duration::from_millis(1100));
    assert!(!f.root().join("fuite-premier-plan").exists());
    assert!(!f.root().join("fuite-fond").exists());
}

#[test]
fn screen_control_still_needs_a_session_the_owner_started_even_in_yolo() {
    let mut f = Fixture::new();
    f.send("capture", "computer_screenshot", json!({}));
    f.finish("capture");
    assert!(f.requests("capture").is_empty());
    let out = f.outcome("capture");
    assert_eq!(out["status"], "failed");
    assert!(out["reason"]
        .as_str()
        .unwrap()
        .contains("Aucune session active"));
}

#[test]
fn a_screen_session_granted_in_yolo_takes_gestures_without_a_card_and_ends_when_ask_returns() {
    let mut f = Fixture::new();
    let chat = f.chat.clone();
    // Aucun réglage « Écran du Mac » n'a été touché : le Mode YOLO ouvre l'écran, la session reste à accorder.
    f.post(
        "/api/screen/grant",
        json!({"bot_id":"grokbot","conversation_id":chat,"minutes":1}),
    );
    f.app.tick(now());
    assert!(!f.state()["screen"]["session"].is_null());
    let out = f.runs_without_a_card("ecran", "computer_screenshot", json!({}));
    assert_eq!(out["status"], "done", "{out}");
    // Le geste est lié à la session vivante à l'admission, pas à la suivante.
    let authority = f.card(&chat, "ecran")["payload"]["authorized_by"].clone();
    assert_eq!(authority["kind"], "yolo");
    assert_eq!(authority["source"], "global");
    assert!(authority["session"].is_u64(), "{authority}");
    f.mode("ask");
    f.app.tick(now());
    assert!(
        f.state()["screen"]["session"].is_null(),
        "quitter le Mode YOLO révoque la session accordée sans réglage"
    );
}

#[test]
fn settings_cards_and_drafts_keep_their_card_in_yolo() {
    use grokbot_wrapper::features::yolo::covered;
    for tool in [
        "propose_skill",
        "mcp_start_github",
        "mcp_remove_github",
        "recall_join",
    ] {
        assert!(!covered(tool), "{tool} doit garder sa carte");
    }
    for tool in [
        "run_shell",
        "write_file",
        "delegate",
        "computer_click",
        "create_document",
    ] {
        assert!(covered(tool), "{tool} doit être couvert");
    }
}

#[test]
fn a_payment_tool_is_refused_in_both_modes_with_no_card_and_never_reaches_the_connector() {
    let mut f = Fixture::new();
    let calls = f.payments_connector();
    for (mode, ids) in [
        ("yolo", ["paie-yolo", "vire-yolo"]),
        ("ask", ["paie-ask", "vire-ask"]),
    ] {
        f.mode(mode);
        for (id, tool) in ids.into_iter().zip(["create_payment", "transfer_funds"]) {
            let name = f.connector_tool(tool);
            f.send(id, &name, json!({}));
            f.finish(id);
            assert!(f.requests(id).is_empty(), "{id} a demandé une carte");
            assert_eq!(f.outcome(id)["reason"], PAYMENTS, "{id}");
        }
    }
    assert!(std::fs::read_to_string(&calls)
        .unwrap_or_default()
        .is_empty());
    let log = f.forbidden_log();
    assert_eq!(log.len(), 4);
    assert!(log.iter().all(|e| e["summary"]
        .as_str()
        .unwrap()
        .starts_with("Refusé (action interdite : paiement) · Connecteur Caisse")));
}

#[test]
fn reading_payments_and_plain_commands_keep_their_decision_in_both_modes() {
    let mut f = Fixture::new();
    let calls = f.payments_connector();
    let list = f.connector_tool("list_payments");
    f.mode("yolo");
    f.runs_without_a_card("lire-yolo", &list, json!({}));
    f.runs_without_a_card("shell-yolo", "run_shell", f.shell("printf ok"));
    assert_eq!(
        std::fs::read_to_string(&calls).unwrap().trim(),
        "list_payments"
    );
    f.mode("ask");
    f.local("grokbot", true, false);
    let other = f.post("/api/conversations", json!({"title":"Demander"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.send("lire-ask", &list, json!({}));
    f.request("lire-ask");
    f.send_in(&other, "shell-ask", "run_shell", f.shell("printf ok"));
    f.request("shell-ask");
    assert!(f.forbidden_log().is_empty());
}

#[test]
fn the_forbidden_list_is_code_only_with_no_table_route_or_setting() {
    let mut f = Fixture::new();
    let tables: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE lower(name) LIKE '%forbid%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
    for (method, path) in [
        ("GET", "/api/forbidden"),
        ("POST", "/api/forbidden"),
        ("POST", "/api/forbidden/disable"),
    ] {
        let error = f.app.api(method, path, b"{}").unwrap_err();
        assert_eq!(error.status, 404, "{method} {path}");
    }
    assert!(!f.state().to_string().contains("forbid"));
}
