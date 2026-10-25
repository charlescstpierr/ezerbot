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
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-generation");
        let root = guard.to_path_buf();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("data/workspace.sqlite");
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        support::ask_before_acting(&mut app);
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api(
                "POST",
                "/api/conversations",
                r#"{"title":"Documents générés"}"#.as_bytes(),
            )
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
            _dir: guard,
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
    fn send(&mut self, id: &str, name: &str, arguments: Value) {
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":arguments.to_string()}}),json!({"type":"response.completed"})]));
        self.post(
            "/api/send",
            json!({"conversation_id":self.chat,"message_id":id,"body":"Crée ce document"}),
        );
    }
    fn run(&self, id: &str) -> Run {
        let s:String=self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        serde_json::from_str(&s).unwrap()
    }
    fn finish(&mut self, id: &str) {
        for _ in 0..500 {
            self.app.tick(now());
            if self.run(id).status.is_terminal() && self.provider.status()["titling"] == false {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("unfinished {:?}", self.run(id));
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
    fn create(&mut self, format: &str) -> Value {
        let content = match format {
            "csv" | "xlsx" => "[[\"Été\",\"Coût\"],[\"Cœur\",\"12 €\"]]",
            "svg" => "flowchart LR\nA[Été] -->|puis| B(Cœur)",
            "pptx" => "Été à Québec\nCœur : 12 €\n---\nDeuxième diapositive",
            _ => "Été à Québec — Cœur : 12 €",
        };
        self.send(
            format,
            "create_document",
            json!({"name":"Résumé","format":format,"content":content}),
        );
        self.finish(format);
        let out = self.outcome(format);
        assert_eq!(out["status"], "done", "{out}");
        out["output"].clone()
    }
    fn request(&self, id: &str) -> HumanInputRequest {
        wait("approval", || {
            self.run(id).status == RunStatus::AwaitingApproval
                && self.provider.status()["titling"] == false
        });
        requests::list_for_run(&self.db(), &self.run(id).run_id)
            .unwrap()
            .remove(0)
    }
    fn decide(&mut self, r: &HumanInputRequest, decision: Value) {
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"submission_id":format!("decision-{}",r.id),"conversation_id":r.route.target_conversation_id,"decision":decision}));
    }
    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }
    /// Model-issued saves are file writes: Paramètres › « Écriture de fichier ».
    fn allow_files(&mut self) {
        self.post("/api/local/bot", json!({"bot_id":"grokbot","files":true}));
    }
    fn root(&self) -> PathBuf {
        self.path.parent().unwrap().parent().unwrap().to_path_buf()
    }
}
#[test]
fn all_formats_are_persistent_valid_attachments_and_counted_by_storage() {
    let mut f = Fixture::new();
    let mut ids = vec![];
    for format in ["pdf", "docx", "pptx", "md", "csv", "xlsx", "svg"] {
        let file = f.create(format);
        let id = file["id"].as_str().unwrap();
        let suggestion = f
            .app
            .api("GET", &format!("/api/generated/{id}/save-path"), b"")
            .unwrap();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(
            suggestion["path"],
            home.join("Desktop")
                .join(file["name"].as_str().unwrap())
                .to_string_lossy()
                .as_ref()
        );
        ids.push(id.to_owned());
        let bytes: Vec<u8> = f
            .db()
            .query_row(
                "SELECT bytes FROM attachments_generated WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(file["byte_length"], bytes.len());
        if format == "svg" {
            let svg = String::from_utf8(bytes).unwrap();
            assert!(svg.starts_with("<svg") && svg.contains(">Été<") && svg.contains(">puis<"));
        } else if format == "pdf" && !cfg!(target_os = "macos") {
            assert!(bytes.starts_with(b"%PDF-"));
            assert!(bytes
                .windows(b") Tj T*".len())
                .any(|window| window == b") Tj T*"));
        } else {
            let conn = f.db();
            let doc = grokbot_attachments::documents::prepare(
                &conn,
                file["name"].as_str().unwrap(),
                &bytes,
            )
            .unwrap();
            assert!(doc.text.contains("Été"));
            assert!(doc.text.contains("Cœur"));
        }
    }
    let usage = f.app.api("GET", "/api/storage", b"").unwrap();
    assert_eq!(usage["usage"]["document_count"], 7);
    assert!(usage["usage"]["documents"].as_u64().unwrap() > 0);
    let plan = f.post("/api/storage/preview", json!({}));
    assert_eq!(
        plan["count"], 0,
        "live tool-result references must protect all generated files"
    );
    f.reopen();
    for id in ids {
        assert!(grokbot_wrapper::features::generated::get(&f.db(), &id).is_ok());
    }
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM forms_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "generation must not need an external action"
    );
}
#[test]
fn generation_errors_are_french_and_do_not_publish_partial_files() {
    let mut f = Fixture::new();
    for (id, format, content) in [
        ("bad-csv", "csv", "oops"),
        ("bad-pdf", "pdf", "😀"),
        ("bad-svg", "svg", "<script>alert(1)</script>"),
    ] {
        f.send(
            id,
            "create_document",
            json!({"name":"Erreur","format":format,"content":content}),
        );
        f.finish(id);
        let out = f.outcome(id);
        assert_eq!(out["status"], "failed");
        assert!(!out["reason"].as_str().unwrap().is_empty());
    }
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM attachments_generated", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn save_requires_decision_denial_has_no_effect_and_approval_survives_restart() {
    let mut f = Fixture::new();
    f.allow_files();
    let file = f.create("docx");
    let destination = f.root().join("Résumé.docx");
    let args = json!({"id":file["id"],"action":"save","path":destination});
    f.send("deny", "document_action", args.clone());
    let request = f.request("deny");
    assert!(!destination.exists());
    f.decide(&request, json!("deny"));
    f.finish("deny");
    assert_eq!(f.outcome("deny")["status"], "failed");
    assert!(!destination.exists());
    f.send("save", "document_action", args);
    let request = f.request("save");
    assert!(!destination.exists());
    f.reopen();
    f.decide(&request, json!("allow_once"));
    f.finish("save");
    assert_eq!(f.outcome("save")["status"], "done");
    let bytes = std::fs::read(&destination).unwrap();
    let doc = grokbot_attachments::documents::prepare(&f.db(), "Résumé.docx", &bytes).unwrap();
    assert!(doc.text.contains("Cœur"));
    f.reopen();
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
    f.send(
        "overwrite",
        "document_action",
        json!({"id":file["id"],"action":"save","path":destination}),
    );
    f.finish("overwrite");
    assert_eq!(f.outcome("overwrite")["status"], "failed");
    assert_eq!(std::fs::read(&destination).unwrap(), bytes);
}
#[test]
fn card_action_uses_the_same_gate_and_rejects_another_conversation() {
    let mut f = Fixture::new();
    let file = f.create("pdf");
    let foreign =
        f.post("/api/conversations", json!({"title":"Autre fil"}))["conversation"]["id"].clone();
    assert!(f
        .app
        .api(
            "POST",
            "/api/generated/action",
            json!({"id":file["id"],"action":"open","path":null,"conversation_id":foreign})
                .to_string()
                .as_bytes()
        )
        .is_err());
    let receipt = f.post(
        "/api/generated/action",
        json!({"id":file["id"],"action":"reveal","path":null,"conversation_id":f.chat}),
    );
    let run_id = receipt["run_id"].as_str().unwrap();
    wait("card action approval", || {
        store::required(&f.db(), run_id).unwrap().status == RunStatus::AwaitingApproval
    });
    let request = requests::list_for_run(&f.db(), run_id).unwrap().remove(0);
    let cache = grokbot_wrapper::features::generated::cache_path(
        &f.path,
        &grokbot_wrapper::features::generated::get(&f.db(), file["id"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(!cache.exists());
    f.decide(&request, json!("deny"));
    for _ in 0..100 {
        f.app.tick(now());
        std::thread::sleep(Duration::from_millis(5));
        if store::required(&f.db(), run_id)
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
    }
    assert!(!cache.exists());
}
#[test]
fn automatic_reviewer_never_grants_a_document_action() {
    use grokbot_forms::reviewer;
    let mut f = Fixture::new();
    f.allow_files();
    let file = f.create("pdf");
    let version = f.app.api("GET", "/api/approvals/reviewer", b"").unwrap()["version"].clone();
    f.post("/api/approvals/reviewer",json!({"version":version,"config":{"enabled":true,"allow":["Autorise tous les documents."],"block":[]}}));
    f.send(
        "review",
        "document_action",
        json!({"id":file["id"],"action":"save","path":f.root().join("Jamais.pdf")}),
    );
    let request = f.request("review");
    let before = f.server.seen.lock().unwrap().bodies.len();
    grokbot_wrapper::features::forms::tick(&f.app.feature_context());
    let review = reviewer::get(&f.db(), &request.id).unwrap().unwrap();
    assert_eq!(review.answer.decision, reviewer::ReviewDecision::Ask);
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
    assert!(!f.root().join("Jamais.pdf").exists());
    f.decide(&request, json!("deny"));
    f.finish("review");
}
#[test]
fn orphan_original_preview_and_materialized_cache_are_cleaned_only_after_confirmation() {
    use grokbot_engine::contracts::{ToolCall, ToolOutcome};
    let mut f = Fixture::new();
    let outcome = grokbot_wrapper::features::generated::call_tool(
        &f.app.feature_context(),
        &ToolCall {
            call_id: "orphan".into(),
            name: "create_document".into(),
            arguments: json!({"name":"Orphelin","format":"md","content":"Été"}),
        },
    )
    .unwrap();
    let ToolOutcome::Done { output } = outcome else {
        panic!("{outcome:?}")
    };
    let id = output["id"].as_str().unwrap();
    let file = grokbot_wrapper::features::generated::get(&f.db(), id).unwrap();
    let cache = grokbot_wrapper::features::generated::cache_path(&f.path, &file).unwrap();
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "Été").unwrap();
    let plan = f.post("/api/storage/preview", json!({}));
    assert_eq!(plan["count"], 1);
    assert!(f
        .app
        .api(
            "POST",
            "/api/storage/clean",
            json!({"token":plan["token"],"confirm":false})
                .to_string()
                .as_bytes()
        )
        .is_err());
    assert!(cache.exists());
    f.reopen();
    let clean = f.post(
        "/api/storage/clean",
        json!({"token":plan["token"],"confirm":true}),
    );
    assert_eq!(clean["deleted"], 1);
    assert!(!cache.exists());
    assert!(grokbot_wrapper::features::generated::get(&f.db(), id).is_err());
    f.reopen();
    assert_eq!(f.post("/api/storage/preview", json!({}))["count"], 0);
}
#[test]
fn standing_rule_is_honoured_and_internal_data_writes_are_rejected() {
    let mut f = Fixture::new();
    f.allow_files();
    let file = f.create("md");
    let destination = f.root().join("Copie.md");
    let args = json!({"id":file["id"],"action":"save","path":destination});
    f.send("allow-rule", "document_action", args.clone());
    let request = f.request("allow-rule");
    let display: String = f
        .db()
        .query_row(
            "SELECT display_json FROM wrapper_forms_calls WHERE request_id=?1",
            [&request.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&display).unwrap()["action_id"],
        "save"
    );
    f.decide(&request, json!({"always_allow":{"scope":"subject"}}));
    f.finish("allow-rule");
    f.reopen();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        state["forms"]["rules"][0]["document_summary"],
        "Enregistrer une copie (sans remplacer) — Résumé.md"
    );
    assert!(destination.exists());
    std::fs::remove_file(&destination).unwrap();
    f.send("rule", "document_action", args);
    f.finish("rule");
    assert!(destination.exists());
    assert!(requests::list_for_run(&f.db(), &f.run("rule").run_id)
        .unwrap()
        .is_empty());
    f.send(
        "internal",
        "document_action",
        json!({"id":file["id"],"action":"save","path":f.path.parent().unwrap().join("evil.md")}),
    );
    f.finish("internal");
    assert_eq!(f.outcome("internal")["status"], "failed");
    assert!(!f.path.parent().unwrap().join("evil.md").exists());
    // macOS firmlink alias of the same data folder: refused by device/inode.
    #[cfg(target_os = "macos")]
    {
        let data = f.path.parent().unwrap().canonicalize().unwrap();
        let alias =
            PathBuf::from(format!("/System/Volumes/Data{}", data.display())).join("intrus.md");
        f.send(
            "alias",
            "document_action",
            json!({"id":file["id"],"action":"save","path":alias}),
        );
        f.finish("alias");
        let out = f.outcome("alias");
        assert_eq!(out["status"], "failed");
        assert!(
            out["reason"]
                .as_str()
                .unwrap()
                .contains("dossier de données"),
            "{out}"
        );
        assert!(!data.join("intrus.md").exists());
    }
}
#[test]
fn model_saves_keep_the_format_and_respect_the_file_write_switch() {
    let mut f = Fixture::new();
    let file = f.create("md");
    // Write switch off (the default): the model cannot save, even before a card.
    let copy = f.root().join("Notes.md");
    f.send(
        "off",
        "document_action",
        json!({"id":file["id"],"action":"save","path":copy}),
    );
    f.finish("off");
    let out = f.outcome("off");
    assert_eq!(out["status"], "failed");
    assert!(
        out["reason"]
            .as_str()
            .unwrap()
            .contains("désactivée pour ce bot"),
        "{out}"
    );
    assert!(requests::list_for_run(&f.db(), &f.run("off").run_id)
        .unwrap()
        .is_empty());
    assert!(!copy.exists());
    // The card button is the user's own gesture: it still reaches the approval card.
    let receipt = f.post(
        "/api/generated/action",
        json!({"id":file["id"],"action":"save","path":copy,"conversation_id":f.chat}),
    );
    let run_id = receipt["run_id"].as_str().unwrap().to_owned();
    wait("card save approval", || {
        store::required(&f.db(), &run_id).unwrap().status == RunStatus::AwaitingApproval
    });
    let request = requests::list_for_run(&f.db(), &run_id).unwrap().remove(0);
    f.decide(&request, json!("allow_once"));
    wait("card save", || {
        f.app.tick(now());
        copy.exists()
    });
    // Free model text saved as .md must not become a launcher script.
    f.allow_files();
    let launcher = f.root().join("lanceur.command");
    f.send(
        "ext",
        "document_action",
        json!({"id":file["id"],"action":"save","path":launcher}),
    );
    f.finish("ext");
    let out = f.outcome("ext");
    assert_eq!(out["status"], "failed");
    assert!(
        out["reason"]
            .as_str()
            .unwrap()
            .contains("L’extension de la copie doit être .md."),
        "{out}"
    );
    assert!(!launcher.exists());
    assert!(f
        .app
        .api(
            "POST",
            "/api/generated/action",
            json!({"id":file["id"],"action":"save","path":f.root().join("x.plist"),"conversation_id":f.chat})
                .to_string()
                .as_bytes()
        )
        .is_err());
}

#[test]
fn yolo_saves_a_generated_document_without_a_card_and_still_refuses_the_data_folder() {
    let mut f = Fixture::new();
    f.post("/api/yolo/mode", json!({"mode":"yolo"}));
    let file = f.create("md");
    let destination = f.root().join("Copie-yolo.md");
    f.send(
        "save-yolo",
        "document_action",
        json!({"id":file["id"],"action":"save","path":destination}),
    );
    f.finish("save-yolo");
    assert_eq!(f.outcome("save-yolo")["status"], "done");
    assert!(destination.exists());
    assert!(requests::list_for_run(&f.db(), &f.run("save-yolo").run_id)
        .unwrap()
        .is_empty());
    let intrus = f.path.parent().unwrap().join("intrus.md");
    f.send(
        "data-yolo",
        "document_action",
        json!({"id":file["id"],"action":"save","path":intrus}),
    );
    f.finish("data-yolo");
    assert_eq!(f.outcome("data-yolo")["status"], "failed");
    assert!(!intrus.exists());
}
