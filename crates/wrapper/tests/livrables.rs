mod support;
use base64::Engine;
use grokbot_engine::model::Run;
use grokbot_wrapper::{ApiResponse, App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

const ACCOUNT: &str = "account-ezerbot-livrables";
const PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
const PAGE: &str = r#"export default function Bilan() {
  return (
    <article>
      <h1>Bilan</h1>
      <p>Trois forfaits comparés.</p>
    </article>
  );
}
"#;

struct Fixture {
    app: App,
    provider: Provider,
    server: FakeOpenAi,
    path: PathBuf,
    _dir: grokbot_test_support::TestDir,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn session_token(account: &str) -> String {
    format!(
        "e30.{}.e30",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            json!({"https://api.openai.com/auth":{"chatgpt_account_id":account,"chatgpt_data_residency":"eu"}}).to_string()
        )
    )
}

impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-livrables");
        let root = guard.to_path_buf();
        std::fs::create_dir_all(root.join("data")).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join("data/workspace.sqlite");
        std::fs::write(path.with_extension("chatgpt-auth.json"), json!({"host_id":"test-host","client_id":CLIENT,"account_id":ACCOUNT,"access_token":session_token(ACCOUNT),"refresh_token":"fake-refresh","expires_at":now()/1000+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let mut app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        app.api(
            "POST",
            "/api/bots",
            br#"{"id":"alpha","name":"Alpha","color":"cyan"}"#,
        )
        .unwrap();
        app.api(
            "POST",
            "/api/bots",
            br#"{"id":"beta","name":"Beta","color":"citron"}"#,
        )
        .unwrap();
        Self {
            app,
            provider,
            server,
            path,
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

    fn chat(&mut self, bot: &str) -> String {
        self.post("/api/conversations", json!({"bot_id": bot}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }

    fn run(&self, id: &str) -> Run {
        let s: String = self.db().query_row("SELECT record FROM engine_runs JOIN wrapper_engine_runs USING(run_id) WHERE json_extract(input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    fn call(&mut self, chat: &str, id: &str, tool: &str, arguments: Value) {
        self.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":tool,"arguments":arguments.to_string()}}),json!({"type":"response.completed"})]));
        self.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":id,"body":"Crée-le"}),
        );
        for _ in 0..800 {
            self.app.tick(now());
            if self.run(id).status.is_terminal() && self.provider.status()["titling"] == false {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("unfinished {:?}", self.run(id));
    }

    fn page(&mut self, chat: &str, id: &str, name: &str, title: &str) {
        self.call(
            chat,
            id,
            "write_page",
            json!({"name":name,"title":title,"source":PAGE}),
        );
    }

    fn document(&mut self, chat: &str, id: &str, name: &str, format: &str) {
        self.call(
            chat,
            id,
            "create_document",
            json!({"name":name,"format":format,"content":"Été à Québec"}),
        );
    }

    fn image(&mut self, chat: &str, id: &str, description: &str) {
        self.server.seen.lock().unwrap().image_reply.push_back(ScriptedResponse::json(
            200,
            json!({"output":[{"type":"image_generation_call","status":"completed","result":PNG_B64}]}),
        ));
        self.call(
            chat,
            id,
            "generate_image",
            json!({"description":description}),
        );
    }

    fn list(&mut self, query: &str) -> Vec<Value> {
        let path = if query.is_empty() {
            "/api/livrables".into()
        } else {
            format!("/api/livrables?{query}")
        };
        self.app.api("GET", &path, b"").unwrap()["livrables"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn opens(&mut self, item: &Value) {
        let url = item["open_url"].as_str().unwrap().to_owned();
        match self.app.api_response("GET", &url, b"") {
            Ok(ApiResponse::Json(_) | ApiResponse::Bytes { .. }) => {}
            Err(e) => panic!("{url} : {} {}", e.status, e.message),
        }
    }

    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }

    fn before_the_migration(&mut self) {
        let db = self.db();
        db.execute_batch(
            "DROP TABLE wrapper_livrables;
             DELETE FROM component_versions WHERE component='wrapper.livrables';",
        )
        .unwrap();
        drop(db);
        self.reopen();
    }
}

fn types(items: &[Value]) -> Vec<&str> {
    items.iter().map(|i| i["type"].as_str().unwrap()).collect()
}

#[test]
fn documents_and_images_have_provenance_while_a_parallel_page_is_absent() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", "Bilan des forfaits");
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM bot_pages WHERE status='ready'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    f.document(&chat, "d1", "Rapport", "pdf");
    f.image(&chat, "i1", "un phare au crépuscule");
    let items = f.list("");
    assert_eq!(items.len(), 2, "{items:?}");
    let mut kinds = types(&items);
    kinds.sort();
    assert_eq!(kinds, ["document", "image"]);
    for item in &items {
        assert_eq!(item["bot_id"], "alpha");
        assert_eq!(item["bot_name"], "Alpha");
        assert_eq!(item["conversation_id"], chat.as_str());
        assert!(item["date"].as_u64().unwrap() > 0);
        assert!(!item["title"].as_str().unwrap().is_empty());
        let card = item["entry_id"].as_str().unwrap();
        let known: bool = f.db().query_row("SELECT EXISTS(SELECT 1 FROM wrapper_messages WHERE conversation_id=?1 AND message_id=?2 AND kind='engine_tool')",[&chat, card],|r|r.get(0)).unwrap();
        assert!(known, "carte {card} absente du fil");
        f.opens(item);
    }
    assert!(items
        .iter()
        .all(|i| !i["open_url"].as_str().unwrap().contains("/pages/")));
    let by_date: Vec<u64> = items.iter().map(|i| i["date"].as_u64().unwrap()).collect();
    assert!(
        by_date.windows(2).all(|w| w[0] >= w[1]),
        "du plus récent au plus ancien"
    );
}

#[test]
fn an_image_the_user_sent_is_not_a_livrable() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.db().execute_batch("INSERT INTO attachments_staged VALUES ('attachment_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','png',70,1)").unwrap();
    f.db().execute("INSERT INTO attachments_message_links VALUES (?1,'envoi-charles','attachment_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0)", [&chat]).unwrap();
    f.image(&chat, "i1", "un phare");
    let items = f.list("");
    assert_eq!(types(&items), ["image"], "{items:?}");
    f.reopen();
    assert_eq!(f.list("").len(), 1);
}

#[test]
fn type_and_bot_filters_isolate_the_livrables_and_unknown_parameters_are_refused() {
    let mut f = Fixture::new();
    let a = f.chat("alpha");
    let b = f.chat("beta");
    f.page(&a, "p1", "a.page.tsx", "Page de A");
    f.document(&a, "d1", "RapportA", "pdf");
    f.image(&a, "i1", "un phare");
    assert!(f.list("bot=beta").is_empty(), "B n’a encore rien produit");
    f.document(&b, "d2", "RapportB", "docx");
    let documents = f.list("type=document");
    assert_eq!(types(&documents), ["document", "document"]);
    let beta = f.list("bot=beta");
    assert_eq!(beta.len(), 1);
    assert_eq!(beta[0]["bot_id"], "beta");
    assert_eq!(beta[0]["title"], "RapportB.docx");
    let alpha_documents = f.list("bot=alpha&type=document");
    assert_eq!(alpha_documents.len(), 1);
    assert_eq!(alpha_documents[0]["title"], "RapportA.pdf");
    assert_eq!(f.list("bot=alpha").len(), 2);
    assert!(f.list("bot=inconnu").is_empty());
    for query in [
        "couleur=rouge",
        "type=page",
        "type=document&type=page",
        "bot",
        "bot=alpha&x=1",
    ] {
        let e = f
            .app
            .api("GET", &format!("/api/livrables?{query}"), b"")
            .unwrap_err();
        assert_eq!(e.status, 400, "{query}");
    }
}

#[test]
fn two_identical_creations_in_two_conversations_are_two_livrables() {
    let mut f = Fixture::new();
    let a = f.chat("alpha");
    let b = f.chat("beta");
    f.document(&a, "d1", "Rapport", "md");
    f.document(&b, "d2", "Rapport", "md");
    let one_file: i64 = f
        .db()
        .query_row("SELECT COUNT(*) FROM attachments_generated", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(one_file, 1, "le fichier est adressé par son contenu");
    let items = f.list("type=document");
    let mut bots: Vec<&str> = items
        .iter()
        .map(|i| i["bot_id"].as_str().unwrap())
        .collect();
    bots.sort();
    assert_eq!(bots, ["alpha", "beta"]);
    assert_eq!(items[0]["id"], items[1]["id"]);
    assert_ne!(items[0]["entry_id"], items[1]["entry_id"]);
    f.before_the_migration();
    let again = f.list("type=document");
    let mut bots: Vec<&str> = again
        .iter()
        .map(|i| i["bot_id"].as_str().unwrap())
        .collect();
    bots.sort();
    assert_eq!(
        bots,
        ["alpha", "beta"],
        "la reprise retrouve les deux créations"
    );
}

#[test]
fn documents_and_images_from_before_the_migration_are_found_without_error() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.document(&chat, "d1", "Ancien", "pdf");
    f.image(&chat, "i1", "un vieux phare");
    let before = f.list("");
    assert_eq!(before.len(), 2);
    f.db().execute("INSERT INTO attachments_generated VALUES ('generated_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',?1,x'00','<svg/>',?2)", rusqlite::params![json!({"id":"generated_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","name":"Orphelin.pdf","format":"pdf","byte_length":1,"pages":1,"preview_url":"/api/generated/generated_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/preview"}).to_string(), 5]).unwrap();
    f.before_the_migration();
    let after = f.list("");
    assert_eq!(after.len(), 3, "{after:?}");
    let known: Vec<_> = after.iter().filter(|i| !i["bot_id"].is_null()).collect();
    assert_eq!(known.len(), 2);
    for item in &known {
        assert_eq!(item["bot_id"], "alpha");
        assert_eq!(item["conversation_id"], chat.as_str());
        assert!(item["entry_id"].is_string());
        assert!(item["date"].as_u64().unwrap() > 0);
    }
    let unknown = after.iter().find(|i| i["bot_id"].is_null()).unwrap();
    assert_eq!(unknown["type"], "document");
    assert_eq!(unknown["title"], "Orphelin.pdf");
    assert!(unknown["conversation_id"].is_null() && unknown["entry_id"].is_null());
    for item in &after {
        f.opens(item);
    }
    f.db()
        .execute(
            "DELETE FROM component_versions WHERE component='wrapper.livrables'",
            [],
        )
        .unwrap();
    f.reopen();
    assert_eq!(f.list("").len(), 3);
    assert_eq!(f.list("bot=alpha").len(), 2);
}

#[test]
fn an_old_document_without_history_survives_the_startup_cleanup_and_opens() {
    let mut f = Fixture::new();
    f.db().execute("INSERT INTO attachments_generated VALUES ('generated_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',?1,x'00','<svg/>',?2)", rusqlite::params![json!({"id":"generated_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","name":"Vieux.pdf","format":"pdf","byte_length":1,"pages":1,"preview_url":"/api/generated/generated_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc/preview"}).to_string(), 5]).unwrap();
    f.reopen_without_marker();
    let items = f.list("");
    assert_eq!(items.len(), 1, "le nettoyage du démarrage l’a emporté");
    assert!(items[0]["bot_id"].is_null());
    f.opens(&items[0]);
    f.reopen();
    assert_eq!(f.list("").len(), 1);
}

impl Fixture {
    fn reopen_without_marker(&mut self) {
        self.db()
            .execute(
                "DELETE FROM component_versions WHERE component='wrapper.livrables'",
                [],
            )
            .unwrap();
        self.reopen();
    }
}

#[test]
fn a_deleted_conversation_or_bot_leaves_no_broken_link() {
    let mut f = Fixture::new();
    let a1 = f.chat("alpha");
    let a2 = f.chat("alpha");
    let b = f.chat("beta");
    for (chat, tag) in [(&a1, "1"), (&a2, "2"), (&b, "3")] {
        f.document(chat, &format!("d{tag}"), &format!("Doc{tag}"), "md");
        f.image(chat, &format!("i{tag}"), &format!("image {tag}"));
    }
    assert_eq!(f.list("").len(), 6);
    f.post("/api/conversations/delete", json!({"id": a1}));
    let left = f.list("");
    assert_eq!(left.len(), 4, "{left:?}");
    assert!(left.iter().all(|i| i["conversation_id"] != a1.as_str()));
    for item in &left {
        f.opens(item);
    }
    f.post("/api/bots/delete", json!({"id": "alpha"}));
    let left = f.list("");
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left.iter().all(|i| i["bot_id"] == "beta"));
    for item in &left {
        f.opens(item);
    }
    f.reopen();
    assert_eq!(f.list("").len(), 2);
}

#[test]
fn a_failed_generation_creates_no_livrable() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.call(
        &chat,
        "bad-csv",
        "create_document",
        json!({"name":"Erreur","format":"csv","content":"oops"}),
    );
    f.server
        .seen
        .lock()
        .unwrap()
        .image_reply
        .push_back(ScriptedResponse::json(
            400,
            json!({"error":{"code":"content_safety_blocked","message":"blocked"}}),
        ));
    f.call(
        &chat,
        "bad-image",
        "generate_image",
        json!({"description":"x"}),
    );
    assert!(f.list("").is_empty());
    f.reopen();
    assert!(f.list("").is_empty());
    f.before_the_migration();
    assert!(f.list("").is_empty());
}

fn list_cost(livrables: usize, noise: usize) -> (i32, usize) {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    let db = f.db();
    db.execute_batch("BEGIN").unwrap();
    for i in 0..livrables {
        db.execute("INSERT INTO wrapper_livrables VALUES (?1,?1,'document',?2,'Rapport','alpha',?3,'/api/x',?4)", rusqlite::params![format!("run-{i}:2"), format!("generated_{i}"), chat, i]).unwrap();
    }
    db.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES ('ancien','alpha',?1,'t','r',1,'finished','{}')", [&chat]).unwrap();
    for i in 0..noise {
        db.execute("INSERT INTO engine_run_events VALUES ('ancien', ?1, 'tool_result', '{\"call_id\":\"c\",\"outcome\":{\"status\":\"done\",\"output\":{\"id\":\"generated_0\"}}}')", [i + 1]).unwrap();
    }
    db.execute_batch("COMMIT").unwrap();
    let plan: Vec<String> = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            grokbot_wrapper::features::livrables::LIST
        ))
        .unwrap()
        .query_map(
            rusqlite::params!["local-workspace", None::<&str>, None::<&str>],
            |r| r.get(3),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    eprintln!("SQL LIST {livrables} creations, {noise} old events: {plan:?}");
    let mut statement = db
        .prepare(grokbot_wrapper::features::livrables::LIST)
        .unwrap();
    let rows = statement
        .query_map(
            rusqlite::params!["local-workspace", None::<&str>, None::<&str>],
            |_| Ok(()),
        )
        .unwrap()
        .count();
    (
        statement.get_status(rusqlite::StatementStatus::VmStep),
        rows,
    )
}

#[test]
fn the_read_does_not_grow_with_the_run_history() {
    let ((small, rows), (large, _)) = (list_cost(50, 1000), list_cost(50, 10_000));
    assert_eq!(rows, 50);
    eprintln!("VmStep LIST: 1000 events {small}, 10000 events {large}");
    assert!(
        large <= small + small / 10,
        "1000 older events: {small} VM instructions, 10000: {large}"
    );
}

#[test]
fn the_read_grows_linearly_with_the_livrables() {
    let ((small, _), (large, rows)) = (list_cost(200, 0), list_cost(400, 0));
    assert_eq!(rows, 400);
    assert!(
        large <= small * 5 / 2,
        "200 livrables: {small} VM instructions, 400: {large}"
    );
    eprintln!("VmStep LIST : 200 livrables {small}, 400 livrables {large}");
}

#[test]
fn a_secret_registered_after_the_creation_is_masked_in_the_titles() {
    const SECRET: &str = "catalogue-r9-valeur-privee";
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.document(&chat, "d1", &format!("Rapport-{SECRET}"), "md");
    f.image(&chat, "i1", &format!("un phare {SECRET}"));
    assert!(f
        .app
        .api("GET", "/api/livrables", b"")
        .unwrap()
        .to_string()
        .contains(SECRET));
    grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json"))
        .unwrap()
        .set("LIVRABLES_TEST", "Secret de test", SECRET)
        .unwrap();
    let body = f.app.api("GET", "/api/livrables", b"").unwrap().to_string();
    assert!(!body.contains(SECRET), "{body}");
    assert_eq!(f.list("").len(), 2);
}

const LEAK: &str = "catalogue-r9-lien-prive";

fn private_document() -> (Fixture, String) {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.call(
        &chat,
        "d1",
        "create_document",
        json!({"name":format!("Rapport-{LEAK}"),"format":"md","content":format!("Le code est {LEAK}")}),
    );
    grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json"))
        .unwrap()
        .set("LIENS_TEST", "Secret de test", LEAK)
        .unwrap();
    (f, chat)
}

fn body(f: &mut Fixture, method: &str, path: &str, payload: Value) -> String {
    match f
        .app
        .api_response(method, path, payload.to_string().as_bytes())
        .unwrap()
    {
        ApiResponse::Json(v) => v.to_string(),
        ApiResponse::Bytes { body, .. } => String::from_utf8_lossy(&body).into_owned(),
    }
}

fn link(f: &mut Fixture, kind: &str) -> String {
    f.list(&format!("type={kind}"))[0]["open_url"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn the_preview_of_a_document_masks_a_secret_registered_since() {
    let (mut f, _) = private_document();
    let url = link(&mut f, "document");
    let text = body(&mut f, "GET", &url, json!(null));
    assert!(text.contains("<svg"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

#[test]
fn the_save_path_of_a_document_masks_a_secret_registered_since() {
    let (mut f, _) = private_document();
    let id = f.list("type=document")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let text = body(
        &mut f,
        "GET",
        &format!("/api/generated/{id}/save-path"),
        json!(null),
    );
    assert!(text.contains("Desktop"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

fn vault(f: &Fixture, name: &str, purpose: &str, value: &str) {
    grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json"))
        .unwrap()
        .set(name, purpose, value)
        .unwrap();
}

fn previews(format: &str, content: &str, secret: &str) -> (String, String) {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.call(
        &chat,
        "d1",
        "create_document",
        json!({"name":"Rapport","format":format,"content":content}),
    );
    let url = link(&mut f, "document");
    let before = body(&mut f, "GET", &url, json!(null));
    vault(&f, "APERCU_TEST", "Secret de test", secret);
    (before, body(&mut f, "GET", &url, json!(null)))
}

#[test]
fn the_preview_masks_a_secret_whose_characters_are_escaped_in_svg() {
    const SECRET: &str = "texte&r9<avec\"xml-prive";
    let (before, after) = previews("md", &format!("Accès : {SECRET}"), SECRET);
    assert!(
        before.contains("texte&amp;r9&lt;avec&quot;xml-prive"),
        "{before}"
    );
    assert!(
        !after.contains("texte&amp;r9&lt;avec&quot;") && !after.contains(SECRET),
        "{after}"
    );
    assert!(after.contains("Accès : [...]"), "{after}");
}

#[test]
fn the_preview_masks_a_secret_cut_across_two_lines() {
    const SECRET: &str = "catalogue-r9-valeur-longue-aaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let (before, after) = previews("md", &format!("Clé : {SECRET}"), SECRET);
    assert!(
        before.contains(&SECRET[..47])
            && before.contains(&SECRET[47..])
            && !before.contains(SECRET),
        "le secret est réparti sur deux <text> : {before}"
    );
    assert!(
        !after.contains(&SECRET[..12]) && !after.contains(&SECRET[47..]),
        "{after}"
    );
    assert!(after.contains("Clé : [...]"), "{after}");
}

#[test]
fn the_preview_masks_a_secret_cut_by_the_cell_width() {
    const SECRET: &str = "catalogue-r9-cellule-aaaaaaaaaaaaaaaaaa";
    let rows = json!([["Nom", "Jeton"], ["API", SECRET]]).to_string();
    let (before, after) = previews("csv", &rows, SECRET);
    assert!(
        before.contains(&SECRET[..30]) && !before.contains(SECRET),
        "la cellule coupe le secret : {before}"
    );
    assert!(!after.contains(&SECRET[..12]), "{after}");
    assert!(after.contains(">[...]<"), "{after}");
}

#[test]
fn a_secret_absent_from_the_text_leaves_the_preview_untouched() {
    let (before, after) = previews("md", "Été à Québec", "font-family");
    assert!(before.contains("font-family="), "{before}");
    assert_eq!(after, before);
}

#[test]
fn the_save_path_masks_only_the_file_name_never_the_home_folder() {
    let home = std::env::var("HOME").unwrap();
    let account = std::path::Path::new(&home)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        account.chars().count() >= 6,
        "compte trop court : {account}"
    );
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.call(
        &chat,
        "d1",
        "create_document",
        json!({"name":format!("Rapport-{account}"),"format":"md","content":"Bilan"}),
    );
    f.call(
        &chat,
        "d2",
        "create_document",
        json!({"name":"catalogue-r9-nom-entier.md","format":"md","content":"Autre"}),
    );
    vault(
        &f,
        "SITE_COMPTE",
        "Formulaire de site privé",
        &json!({"values":{"identifiant":account}}).to_string(),
    );
    vault(
        &f,
        "NOM_TEST",
        "Secret de test",
        "catalogue-r9-nom-entier.md",
    );
    let desktop = std::path::Path::new(&home).join("Desktop");
    let mut paths: Vec<String> = f
        .list("type=document")
        .iter()
        .map(|item| {
            let url = format!("/api/generated/{}/save-path", item["id"].as_str().unwrap());
            f.app.api("GET", &url, b"").unwrap()["path"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            desktop.join("Rapport-[...].md").to_str().unwrap(),
            desktop.join("[...].md").to_str().unwrap(),
        ]
    );
}

#[test]
fn an_image_title_is_cut_after_masking_never_inside_a_secret() {
    const SECRET: &str = "catalogue-r9-image-privee";
    let filler: String = "Un phare au bord de la mer, "
        .repeat(3)
        .chars()
        .take(76)
        .collect();
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.image(&chat, "i1", &format!("{filler}{SECRET}"));
    vault(&f, "TITRE_TEST", "Secret de test", SECRET);
    let expected = format!("{filler}[...");
    assert_eq!(f.list("type=image")[0]["title"], expected.as_str());
    f.before_the_migration();
    assert_eq!(f.list("type=image")[0]["title"], expected.as_str());
}

fn source_cost(noise: usize) -> i32 {
    let f = Fixture::new();
    let db = f.db();
    db.execute_batch("BEGIN").unwrap();
    db.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES ('ancien','alpha','c','t','r',1,'finished','{}')", []).unwrap();
    for i in 0..noise {
        db.execute("INSERT INTO engine_run_events VALUES ('ancien', ?1, 'tool_call', '{\"call_id\":\"c\",\"arguments\":{}}')", [i + 1]).unwrap();
        db.execute("INSERT INTO wrapper_livrables VALUES (?1,?1,'document',?2,'Rapport','alpha','c','/api/x',?3)", rusqlite::params![format!("ancien:{}", i + 1), format!("generated_{i}"), i]).unwrap();
    }
    db.execute_batch("COMMIT").unwrap();
    let mut steps = 0;
    let mut card = db
        .prepare(grokbot_wrapper::features::generated::SOURCE_CARD)
        .unwrap();
    let entry: String = card.query_row(["generated_7"], |r| r.get(0)).unwrap();
    steps += card.get_status(rusqlite::StatementStatus::VmStep);
    let mut call = db
        .prepare(grokbot_wrapper::features::generated::SOURCE_CALL)
        .unwrap();
    let (run, seq) = entry.rsplit_once(':').unwrap();
    let arguments: Option<String> = call
        .query_row(rusqlite::params![run, seq.parse::<i64>().unwrap()], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(arguments.as_deref(), Some("{}"));
    steps + call.get_status(rusqlite::StatementStatus::VmStep)
}

#[test]
fn finding_the_text_of_a_preview_does_not_grow_with_the_history() {
    let (small, large) = (source_cost(1000), source_cost(10_000));
    assert!(
        large <= small + small / 10,
        "1000 events: {small} VM instructions, 10000: {large}"
    );
    eprintln!("VmStep source de l’aperçu : 1000 événements {small}, 10000 {large}");
}

#[test]
fn catalogue_sensitive_envelope_or_identity_is_refused_without_a_broken_contract() {
    for target in [
        "livrables",
        "bot_id",
        "id",
        "entry_id",
        "open_url",
        "date",
        "conversation_id",
    ] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.document(&chat, "d1", "Public", "md");
        let item = f.list("")[0].clone();
        let secret = match target {
            "livrables" => "livrables".to_owned(),
            "bot_id" => {
                f.db()
                    .execute(
                        "UPDATE wrapper_bots SET id='catalogue-r9-bot-prive' WHERE id='alpha'",
                        [],
                    )
                    .unwrap();
                f.db()
                    .execute(
                        "UPDATE wrapper_livrables SET bot_id='catalogue-r9-bot-prive'",
                        [],
                    )
                    .unwrap();
                "catalogue-r9-bot-prive".into()
            }
            "date" => item["date"].to_string(),
            _ => item[target].as_str().unwrap().to_owned(),
        };
        vault(&f, "CONTRACT_TEST", "Secret synthétique", &secret);
        match f.app.api("GET", "/api/livrables", b"") {
            Err(e) => assert!(
                !e.message.contains(&secret),
                "the refusal exposed private text"
            ),
            Ok(value) => {
                assert!(
                    !value.to_string().contains(&secret),
                    "post-processing recreated private text"
                );
                assert!(
                    value["livrables"].is_array(),
                    "the list contract was corrupted"
                );
                for item in value["livrables"].as_array().unwrap() {
                    assert!(item["date"].is_number());
                    f.opens(item);
                }
            }
        }
    }
}

#[test]
fn catalogue_save_path_does_not_recompose_another_private_value() {
    let (mut f, _) = private_document();
    vault(&f, "SECOND_PATH_TEST", "Secret synthétique", "[...].md");
    let id = f.list("type=document")[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let response = f
        .app
        .api("GET", &format!("/api/generated/{id}/save-path"), b"");
    match response {
        Ok(value) => {
            let path = std::path::Path::new(value["path"].as_str().unwrap());
            assert!(
                !path
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("[...].md"),
                "the extension reconstructed another private value"
            );
            assert_eq!(
                path.parent().unwrap(),
                std::path::Path::new(&std::env::var("HOME").unwrap()).join("Desktop")
            );
        }
        Err(e) => assert!(!e.message.contains("[...].md")),
    }
}

#[test]
fn catalogue_masked_historical_id_cannot_resurrect_a_deleted_document_as_unknown() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.document(&chat, "d1", "Ancien", "md");
    f.db().execute("UPDATE engine_run_events SET payload=json_set(payload,'$.outcome.output.id','[...]') WHERE kind='tool_result' AND json_extract(payload,'$.outcome.output.id') IS NOT NULL", []).unwrap();
    f.post("/api/conversations/delete", json!({"id":chat}));
    f.before_the_migration();
    assert!(
        f.list("").is_empty(),
        "migration lost the deleted origin of a masked result"
    );
    let kept: i64 = f
        .db()
        .query_row("SELECT count(*) FROM attachments_generated", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        kept, 1,
        "uncertain provenance must not authorize erasing the file"
    );
}

#[test]
fn catalogue_record_failure_does_not_publish_a_livrable() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.db().execute_batch("CREATE TRIGGER reject_livrable BEFORE INSERT ON wrapper_livrables BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
    f.document(&chat, "d1", "Refusé", "md");
    f.image(&chat, "i1", "Création refusée");
    assert!(f.list("").is_empty());
    let statuses: Vec<String> = f.db().prepare("SELECT json_extract(payload,'$.outcome.status') FROM engine_run_events WHERE kind='tool_result'").unwrap().query_map([],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    assert!(!statuses.contains(&"done".to_owned()));
    f.db()
        .execute_batch("DROP TRIGGER reject_livrable")
        .unwrap();
    f.before_the_migration();
    assert!(f.list("").is_empty());
}

#[test]
fn names_stay_masked_after_rotation_removal_and_restart() {
    for rotate in [false, true] {
        let (mut f, chat) = private_document();
        f.db()
            .execute("UPDATE wrapper_bots SET name=?1 WHERE id='alpha'", [LEAK])
            .unwrap();
        f.db()
            .execute(
                "UPDATE wrapper_conversations SET title=?1 WHERE id=?2",
                [LEAK, &chat],
            )
            .unwrap();
        let item = f.list("")[0].clone();
        for field in ["title", "bot_name", "conversation_title"] {
            assert!(item[field].as_str().unwrap().contains("[...]"));
        }
        let save_url = format!("/api/generated/{}/save-path", item["id"].as_str().unwrap());
        assert!(!body(&mut f, "GET", &save_url, json!(null)).contains(LEAK));
        let v = grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json")).unwrap();
        if rotate {
            v.set(
                "LIENS_TEST",
                "Valeur synthétique",
                "catalogue-r9-remplacement",
            )
            .unwrap();
        } else {
            v.remove("LIENS_TEST").unwrap();
        }
        for restart in [false, true] {
            if restart {
                f.reopen();
            }
            assert!(!json!(f.list("")).to_string().contains(LEAK));
            assert!(!body(&mut f, "GET", &save_url, json!(null)).contains(LEAK));
        }
    }
}

#[test]
fn canonical_unicode_and_trimmed_form_values_are_masked() {
    for (private, spelling) in [
        ("catalogue-r9-étoile", "catalogue-r9-e\u{301}toile"),
        ("catalogue-r9-e\u{301}toile", "catalogue-r9-étoile"),
        ("  catalogue-r9-formulaire  ", "catalogue-r9-formulaire"),
    ] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.document(&chat, "d1", spelling, "md");
        f.image(&chat, "i1", spelling);
        vault(
            &f,
            "SITE_CATALOGUE_FORM",
            "Formulaire de site privé",
            &json!({"values":{"identifiant":private}}).to_string(),
        );
        let items = f.list("");
        assert_eq!(items.len(), 2);
        for item in &items {
            assert!(item["title"].as_str().unwrap().contains("[...]"));
            if item["type"] == "document" {
                let url = format!("/api/generated/{}/save-path", item["id"].as_str().unwrap());
                assert!(body(&mut f, "GET", &url, json!(null)).contains("[...].md"));
            }
        }
    }
}

#[test]
fn an_unreadable_vault_refuses_catalogue_and_save_path() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.document(&chat, "d1", "Public", "md");
    let item = f.list("")[0].clone();
    std::fs::write(
        f.path.with_extension("secrets.json"),
        b"invalid catalogue fixture",
    )
    .unwrap();
    for url in [
        "/api/livrables".to_owned(),
        format!("/api/generated/{}/save-path", item["id"].as_str().unwrap()),
    ] {
        assert_eq!(f.app.api("GET", &url, b"").unwrap_err().status, 409);
    }
}

#[test]
fn invalid_filters_do_not_echo_private_diagnostic_words() {
    let mut f = Fixture::new();
    vault(&f, "R9_DIAGNOSTIC", "Valeur synthétique", "document");
    let error = f
        .app
        .api("GET", "/api/livrables?type=page", b"")
        .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(!error.message.contains("document"));
}

#[test]
fn a_preview_with_no_recoverable_source_is_hidden() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.document(&chat, "d1", "Source perdue", "pdf");
    let url = link(&mut f, "document");
    f.db().execute("UPDATE engine_run_events SET payload=json_remove(payload,'$.arguments') WHERE kind='tool_call'", []).unwrap();
    let shown = body(&mut f, "GET", &url, json!(null));
    assert!(shown.contains("Aperçu indisponible"));
    assert!(!shown.contains("Québec"));
}

#[test]
fn preview_retirement_survives_removal_and_reconnection() {
    persistent_previews(false);
}

#[test]
fn preview_retirement_survives_rotation_and_reconnection() {
    persistent_previews(true);
}

fn persistent_previews(rotate: bool) {
    for (format, private, content, fallback) in [
        (
            "md",
            "apercu-persistant-retrait",
            "Public apercu-persistant-retrait",
            false,
        ),
        (
            "md",
            "apercu-persistant-privé",
            "Public apercu-persistant-prive\u{301}",
            false,
        ),
        (
            "pdf",
            "apercu&persistant<privé",
            "Public apercu&persistant<privé",
            false,
        ),
        (
            "md",
            "apercu-persistant-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "Public apercu-persistant-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            false,
        ),
        (
            "csv",
            "apercu-persistant-aaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            r#"[["Public","apercu-persistant-aaaaaaaaaaaaaaaaaaaaaaaaaaaa"]]"#,
            false,
        ),
        (
            "md",
            "apercu-persistant-fallback",
            "Public apercu-persistant-fallback",
            true,
        ),
    ] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.call(
            &chat,
            "d1",
            "create_document",
            json!({"name":"Rapport","format":format,"content":content}),
        );
        let url = link(&mut f, "document");
        if fallback {
            f.db().execute("UPDATE engine_run_events SET payload=json_remove(payload,'$.arguments') WHERE kind='tool_call'", []).unwrap();
        }
        let before = body(&mut f, "GET", &url, json!(null));
        assert!(
            before.contains("apercu"),
            "fixture must contain unmasked text"
        );
        vault(&f, "PERSISTENT_PREVIEW", "Valeur synthétique", private);
        for phase in 0..3 {
            if phase == 1 {
                let v = grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json"))
                    .unwrap();
                if rotate {
                    v.set(
                        "PERSISTENT_PREVIEW",
                        "Valeur synthétique",
                        "apercu-remplacement-public",
                    )
                    .unwrap();
                } else {
                    v.remove("PERSISTENT_PREVIEW").unwrap();
                }
            } else if phase == 2 {
                f.reopen();
            }
            let shown = body(&mut f, "GET", &url, json!(null));
            assert!(
                shown.contains("Public"),
                "public document text must remain visible"
            );
            assert!(
                shown.contains("[...]"),
                "preview must remain masked in phase {phase}"
            );
            assert!(
                !shown.contains("apercu") && !shown.contains("aaaaaaaa"),
                "private preview text was emitted in phase {phase}"
            );
        }
    }
}

#[test]
fn preview_refuses_an_unavailable_vault_or_ledger() {
    for failure in ["vault", "read", "write"] {
        let (mut f, _) = private_document();
        let url: String = f
            .db()
            .query_row(
                "SELECT json_extract(record,'$.preview_url') FROM attachments_generated",
                [],
                |r| r.get(0),
            )
            .unwrap();
        match failure {
            "vault" => {
                std::fs::write(f.path.with_extension("secrets.json"), b"invalid fixture").unwrap()
            }
            "read" => {
                f.db()
                    .execute_batch("DROP TABLE wrapper_disclosure_patterns")
                    .unwrap();
            }
            "write" => {
                f.db().execute_batch("CREATE TRIGGER refuse_preview_pattern BEFORE INSERT ON wrapper_disclosure_patterns BEGIN SELECT RAISE(ABORT,'registry unavailable'); END").unwrap();
            }
            _ => unreachable!(),
        }
        let result = f.app.api_response("GET", &url, b"");
        assert!(result.is_err(), "preview must refuse {failure} failure");
        assert!(!result.err().unwrap().message.contains(LEAK));
    }
}

#[test]
fn preview_rechecks_vault_changes_during_source_read() {
    use std::{ffi::c_void, os::raw::c_uint};
    struct Change {
        action: Option<Box<dyn FnOnce()>>,
        succeeded: bool,
    }
    unsafe extern "C" fn change(
        _: c_uint,
        context: *mut c_void,
        statement: *mut c_void,
        _: *mut c_void,
    ) -> i32 {
        let sql = std::ffi::CStr::from_ptr(rusqlite::ffi::sqlite3_sql(statement.cast()));
        if sql.to_bytes() == grokbot_wrapper::features::generated::SOURCE_CALL.as_bytes() {
            let change = &mut *context.cast::<Change>();
            if let Some(action) = change.action.take() {
                change.succeeded =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)).is_ok();
            }
        }
        0
    }
    struct Trace(*mut rusqlite::ffi::sqlite3, Box<Change>);
    impl Drop for Trace {
        fn drop(&mut self) {
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(self.0, 0, None, std::ptr::null_mut());
            }
        }
    }
    for mutation in ["add", "remove", "rotate", "corrupt"] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        let private = "apercu-course-synthetique";
        f.call(
            &chat,
            "d1",
            "create_document",
            json!({"name":"Rapport","format":"md","content":format!("Public {private}")}),
        );
        let url = link(&mut f, "document");
        if mutation != "add" {
            vault(&f, "PREVIEW_RACE", "Valeur synthétique", private);
        }
        let path = f.path.with_extension("secrets.json");
        let mut trace = Trace(
            unsafe { f.app.feature_context().connection.handle() },
            Box::new(Change {
                action: Some(Box::new(move || {
                    let vault = grokbot_forms::vault::Vault::open(&path).unwrap();
                    match mutation {
                        "add" => {
                            vault
                                .set("PREVIEW_RACE", "Valeur synthétique", private)
                                .unwrap();
                        }
                        "remove" => {
                            vault.remove("PREVIEW_RACE").unwrap();
                        }
                        "rotate" => {
                            vault
                                .set("PREVIEW_RACE", "Valeur synthétique", "apercu-autre-valeur")
                                .unwrap();
                        }
                        "corrupt" => std::fs::write(&path, b"invalid fixture").unwrap(),
                        _ => unreachable!(),
                    }
                })),
                succeeded: false,
            }),
        );
        assert_eq!(
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(
                    trace.0,
                    rusqlite::ffi::SQLITE_TRACE_STMT as u32,
                    Some(change),
                    (&mut *trace.1 as *mut Change).cast(),
                )
            },
            rusqlite::ffi::SQLITE_OK
        );
        let result = f.app.api_response("GET", &url, b"");
        assert!(
            trace.1.succeeded,
            "the vault mutation must run during the source read"
        );
        drop(trace);
        if mutation == "corrupt" {
            assert!(
                result.is_err(),
                "a vault failure during rendering must refuse the preview"
            );
        } else {
            let ApiResponse::Bytes { body, .. } = result.unwrap() else {
                panic!("expected SVG");
            };
            let shown = String::from_utf8(body).unwrap();
            assert!(
                shown.contains("Public") && shown.contains("[...]"),
                "the preview must retain public text and mask private text"
            );
            assert!(
                !shown.contains(private),
                "a concurrent vault change exposed private text"
            );
        }
    }
}

#[test]
fn shared_files_survive_cleanup_until_the_last_conversation_is_deleted() {
    let mut f = Fixture::new();
    let a = f.chat("alpha");
    let b = f.chat("beta");
    for (chat, suffix) in [(&a, "a"), (&b, "b")] {
        f.document(chat, &format!("d{suffix}"), "Partagé", "md");
        f.image(chat, &format!("i{suffix}"), "Un phare partagé");
    }
    let items = f.list("");
    assert_eq!(items.len(), 4);
    let images = f.list("type=image");
    assert_eq!(images[0]["id"], images[1]["id"]);
    assert_ne!(images[0]["entry_id"], images[1]["entry_id"]);
    f.post("/api/conversations/delete", json!({"id":a}));
    let plan = f.post("/api/storage/preview", json!({}));
    assert_eq!(plan["count"], 0);
    f.post(
        "/api/storage/clean",
        json!({"token":plan["token"],"confirm":true}),
    );
    let remaining = f.list("");
    assert_eq!(remaining.len(), 2);
    for item in remaining {
        f.opens(&item);
    }
    f.post("/api/conversations/delete", json!({"id":b}));
    assert!(f.list("").is_empty());
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM attachments_staged", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM attachments_generated", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let plan = f.post("/api/storage/preview", json!({}));
    assert_eq!(plan["count"], 1);
    f.post(
        "/api/storage/clean",
        json!({"token":plan["token"],"confirm":true}),
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM attachments_generated", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    f.reopen();
    assert!(f.list("").is_empty());
}
