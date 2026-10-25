//! Livrables : une lecture unique `GET /api/livrables` de ce que les bots ont produit
//! (Pages, documents générés, images générées), avec bot, conversation, date, lien
//! d’ouverture et identifiant de la carte du fil. Une création = un livrable : un document
//! est adressé par son contenu, deux créations identiques restent deux livrables.
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
const BROKEN_PAGE: &str =
    "export default function Page() { return (<table><tr><td>Essentiel</tr></table>); }";

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

    /// Le bot de `chat` appelle `tool` et la run se termine.
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

    /// L’état d’une base d’avant la migration : ni table de livrables, ni sa trace de version.
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
fn a_page_a_pdf_and_an_image_of_bot_a_are_listed_with_bot_and_conversation() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", "Bilan des forfaits");
    f.document(&chat, "d1", "Rapport", "pdf");
    f.image(&chat, "i1", "un phare au crépuscule");
    let items = f.list("");
    assert_eq!(items.len(), 3, "{items:?}");
    let mut kinds = types(&items);
    kinds.sort();
    assert_eq!(kinds, ["document", "image", "page"]);
    for item in &items {
        assert_eq!(item["bot_id"], "alpha");
        assert_eq!(item["bot_name"], "Alpha");
        assert_eq!(item["conversation_id"], chat.as_str());
        assert!(item["date"].as_u64().unwrap() > 0);
        assert!(!item["title"].as_str().unwrap().is_empty());
        // La carte du fil existe : c’est l’entrée d’outil de la conversation.
        let card = item["entry_id"].as_str().unwrap();
        let known: bool = f.db().query_row("SELECT EXISTS(SELECT 1 FROM wrapper_messages WHERE conversation_id=?1 AND message_id=?2 AND kind='engine_tool')",[&chat, card],|r|r.get(0)).unwrap();
        assert!(known, "carte {card} absente du fil");
        f.opens(item);
    }
    let page = items.iter().find(|i| i["type"] == "page").unwrap();
    assert_eq!(page["title"], "Bilan des forfaits");
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
    assert_eq!(f.list("bot=alpha").len(), 3);
    assert!(f.list("bot=inconnu").is_empty());
    for query in [
        "couleur=rouge",
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
    // Un ancien document dont plus aucune run ne se souvient : origine inconnue.
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
    // Relancer la migration ne duplique rien.
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

#[test]
fn a_page_from_before_the_migration_gets_the_card_of_its_last_write_back() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", "Bilan");
    f.page(&chat, "p2", "bilan.page.tsx", "Bilan");
    let before = f.list("type=page");
    let card = before[0]["entry_id"].as_str().unwrap().to_owned();
    assert!(card.starts_with(&f.run("p2").run_id), "{card}");
    f.db()
        .execute("UPDATE bot_pages SET entry_id=NULL", [])
        .unwrap();
    f.reopen_without_marker();
    let after = f.list("type=page");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0]["entry_id"], card.as_str());
}

#[test]
fn a_page_never_announces_the_card_of_another_conversation() {
    let mut f = Fixture::new();
    let c1 = f.chat("alpha");
    let c2 = f.chat("alpha");
    f.page(&c1, "p1", "bilan.page.tsx", "Bilan");
    f.call(
        &c2,
        "p2",
        "write_page",
        json!({"name":"bilan.page.tsx","title":"Bilan","source":BROKEN_PAGE}),
    );
    let id = f
        .db()
        .query_row("SELECT id FROM bot_pages", [], |r| r.get::<_, String>(0))
        .unwrap();
    f.post("/api/pages/revert", json!({"id": id}));
    let items = f.list("type=page");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["conversation_id"], c2.as_str());
    if let Some(card) = items[0]["entry_id"].as_str() {
        let known: bool = f.db().query_row("SELECT EXISTS(SELECT 1 FROM wrapper_messages WHERE conversation_id=?1 AND message_id=?2)", [&c2, card], |r| r.get(0)).unwrap();
        assert!(known, "carte {card} absente de la conversation {c2}");
    }
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
        f.page(
            chat,
            &format!("p{tag}"),
            &format!("page{tag}.page.tsx"),
            &format!("Page {tag}"),
        );
        f.document(chat, &format!("d{tag}"), &format!("Doc{tag}"), "md");
        f.image(chat, &format!("i{tag}"), &format!("image {tag}"));
    }
    assert_eq!(f.list("").len(), 9);
    f.post("/api/conversations/delete", json!({"id": a1}));
    let left = f.list("");
    assert_eq!(left.len(), 6, "{left:?}");
    assert!(left.iter().all(|i| i["conversation_id"] != a1.as_str()));
    for item in &left {
        f.opens(item);
    }
    f.post("/api/bots/delete", json!({"id": "alpha"}));
    let left = f.list("");
    assert_eq!(left.len(), 3, "{left:?}");
    assert!(left.iter().all(|i| i["bot_id"] == "beta"));
    for item in &left {
        f.opens(item);
    }
    f.reopen();
    assert_eq!(f.list("").len(), 3);
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
    f.call(
        &chat,
        "bad-page",
        "write_page",
        json!({"name":"cassee.page.tsx","title":"Cassée","source":BROKEN_PAGE}),
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

/// VM instructions of the one read, with `livrables` rows to list and `noise` older run
/// events that are not livrables.
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
    const SECRET: &str = "livrables-secret-4f7a91c3-test";
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", &format!("Bilan {SECRET}"));
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
    assert_eq!(f.list("").len(), 3);
}

const LEAK: &str = "liens-secret-9b2e61d4-test";

/// Un Page et un document dont le titre, le nom et le contenu portent un secret que le coffre
/// n’apprend qu’après leur création.
fn leaky() -> (Fixture, String) {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", &format!("Bilan {LEAK}"));
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
fn opening_a_page_masks_a_secret_registered_since() {
    let (mut f, _) = leaky();
    let url = link(&mut f, "page");
    let text = body(&mut f, "GET", &url, json!(null));
    assert!(text.contains("Bilan"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

#[test]
fn renaming_a_page_masks_a_secret_in_its_answer() {
    let (mut f, _) = leaky();
    let id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
    let text = body(
        &mut f,
        "POST",
        "/api/pages/rename",
        json!({"id": id, "title": format!("Autre {LEAK}")}),
    );
    assert!(text.contains("Autre"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

#[test]
fn reverting_a_page_masks_a_secret_in_its_answer() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "bilan.page.tsx", &format!("Bilan {LEAK}"));
    f.call(
        &chat,
        "p2",
        "write_page",
        json!({"name":"bilan.page.tsx","title":format!("Bilan {LEAK}"),"source":PAGE.replace("Bilan", "Autre")}),
    );
    grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json"))
        .unwrap()
        .set("LIENS_TEST", "Secret de test", LEAK)
        .unwrap();
    let id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
    let text = body(&mut f, "POST", "/api/pages/revert", json!({"id": id}));
    assert!(text.contains("Bilan"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

#[test]
fn the_preview_of_a_document_masks_a_secret_registered_since() {
    let (mut f, _) = leaky();
    let url = link(&mut f, "document");
    let text = body(&mut f, "GET", &url, json!(null));
    assert!(text.contains("<svg"), "{text}");
    assert!(!text.contains(LEAK), "{text}");
}

#[test]
fn the_save_path_of_a_document_masks_a_secret_registered_since() {
    let (mut f, _) = leaky();
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

/// L’aperçu d’un document créé avec `content`, avant puis après que le coffre apprend `secret`.
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
    const SECRET: &str = "p&ss<w\"rd-apercu-5d03";
    let (before, after) = previews("md", &format!("Accès : {SECRET}"), SECRET);
    assert!(
        before.contains("p&amp;ss&lt;w&quot;rd-apercu-5d03"),
        "{before}"
    );
    assert!(
        !after.contains("p&amp;ss&lt;w&quot;rd") && !after.contains(SECRET),
        "{after}"
    );
    assert!(after.contains("Accès : [...]"), "{after}");
}

#[test]
fn the_preview_masks_a_secret_cut_across_two_lines() {
    const SECRET: &str = "apercu-coupe-0123456789abcdef0123456789abcdef0123456789";
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
    const SECRET: &str = "apercu-cellule-a1b2c3d4e5f6a7b8c9d0e1f2";
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
    // Le coffre peut connaître une valeur qui figure aussi dans le balisage du SVG.
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
        json!({"name":"nom-tout-secret-77e1.md","format":"md","content":"Autre"}),
    );
    // Un identifiant de formulaire privé égal au nom du compte macOS.
    vault(
        &f,
        "SITE_COMPTE",
        "Formulaire de site privé",
        &json!({"values":{"identifiant":account}}).to_string(),
    );
    vault(&f, "NOM_TEST", "Secret de test", "nom-tout-secret-77e1.md");
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
    const SECRET: &str = "zq7x-titre-image-3c9a";
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

#[test]
fn a_page_masks_a_secret_written_with_character_references() {
    const SECRET: &str = "p&ss<w\"rd-page-8e41";
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    let source = PAGE.replace(
        "Trois forfaits comparés.",
        "Code p&amp;ss&lt;w&quot;rd-page-8e41",
    );
    f.call(
        &chat,
        "p1",
        "write_page",
        json!({"name":"bilan.page.tsx","title":format!("Bilan {SECRET}"),"source":source}),
    );
    vault(&f, "PAGE_TEST", "Secret de test", SECRET);
    let url = link(&mut f, "page");
    let page = f.app.api("GET", &url, b"").unwrap();
    let text = page.to_string();
    assert!(!text.contains("rd-page-8e41"), "{text}");
    assert_eq!(page["page"]["title"], "Bilan [...]");
    assert!(
        page["page"]["preview"]
            .as_str()
            .unwrap()
            .contains("Code [...]"),
        "{text}"
    );
}
/// VM instructions to find the text a document was made from, with `noise` older run events.
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

fn rendered_page(page: &Value, chosen: Option<&str>) -> String {
    fn click(node: &Value, row: &Value) -> Option<(String, Value)> {
        if let Some(state) = node["click"]["state"].as_str() {
            let value = node["click"]["index"].as_u64().map_or_else(
                || node["click"]["value"].clone(),
                |index| row[index as usize].clone(),
            );
            return Some((state.to_owned(), value));
        }
        if node["type"] == "map" {
            return node["items"]
                .as_array()?
                .iter()
                .find_map(|row| click(&node["node"], row));
        }
        node["children"]
            .as_array()?
            .iter()
            .find_map(|child| click(child, row))
    }
    fn render(node: &Value, states: &Value, row: &Value) -> String {
        match node["type"].as_str().unwrap_or("") {
            "text" => node["text"].as_str().unwrap_or("").to_owned(),
            "state" => states[node["name"].as_str().unwrap()]
                .as_str()
                .unwrap_or("")
                .to_owned(),
            "field" => row[node["index"].as_u64().unwrap() as usize]
                .as_str()
                .unwrap_or("")
                .to_owned(),
            "map" => node["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| render(&node["node"], states, row))
                .collect(),
            "element" => node["children"]
                .as_array()
                .unwrap()
                .iter()
                .map(|child| render(child, states, row))
                .collect(),
            _ => String::new(),
        }
    }
    let mut states = json!({});
    for state in page["document"]["states"].as_array().into_iter().flatten() {
        states[state["name"].as_str().unwrap()] = state["initial"].clone();
    }
    if chosen.is_some() {
        if let Some((name, value)) = click(&page["document"]["view"], &Value::Null) {
            states[&name] = value;
        }
    }
    render(&page["document"]["view"], &states, &Value::Null)
}

#[test]
fn r5_page_fragments_cannot_reassemble_a_private_value_on_any_response() {
    let secret = "synthetic-fragments-281";
    for source in [
        "export default function Page() { return (<article><span>synthetic-</span><span>fragments-281</span></article>); }",
        "export default function Page() { const [choice, setChoice] = useState(\"public\"); const rows = [[\"fragments-281\"]]; return (<article><p>synthetic-{choice}</p>{rows.map((row) => (<button onClick={() => setChoice(row[0])}>Choisir</button>))}</article>); }",
    ] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.call(&chat, "p1", "write_page", json!({"name":"fragments.page.tsx","title":"Fragments","source":source}));
        let item = f.list("type=page")[0].clone();
        let id = item["id"].as_str().unwrap().to_owned();
        f.call(&chat, "p2", "write_page", json!({"name":"fragments.page.tsx","title":"Fragments","source":source.replace("article", "div")}));
        vault(&f, "FRAGMENTS_TEST", "Secret synthétique", secret);
        for (method, path, payload) in [
            ("GET", format!("/api/pages/{id}"), json!(null)),
            ("POST", "/api/pages/rename".into(), json!({"id":id,"title":"Renommée"})),
            ("POST", "/api/pages/revert".into(), json!({"id":id})),
        ] {
            let result = f.app.api(method, &path, payload.to_string().as_bytes()).unwrap();
            for chosen in [None, Some("fragments-281")] {
                let shown = rendered_page(&result["page"], chosen);
                assert!(!shown.contains(secret), "a private value was reconstructed by the Page");
            }
        }
    }
}

#[test]
fn r5_revert_diagnostic_hides_private_source_and_keeps_the_page_unchanged() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "diagnostic.page.tsx", "Diagnostic");
    let id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
    let secret = "privateSyntheticTag281";
    let previous = format!("export default function Page() {{ return (<{secret}>x</{secret}>); }}");
    f.db()
        .execute(
            "UPDATE bot_pages SET previous_source=?1 WHERE id=?2",
            rusqlite::params![previous, id],
        )
        .unwrap();
    vault(&f, "DIAGNOSTIC_TEST", "Secret synthétique", secret);
    let err = f
        .app
        .api(
            "POST",
            "/api/pages/revert",
            json!({"id":id}).to_string().as_bytes(),
        )
        .unwrap_err();
    assert_eq!(err.status, 400);
    assert!(
        !err.message.contains(secret),
        "the failed revert exposed its private tag"
    );
    let after: String = f
        .db()
        .query_row(
            "SELECT previous_source FROM bot_pages WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, previous);
}

#[test]
fn r5_canonical_unicode_is_masked_in_list_page_anchor_and_save_path() {
    for (secret, spelling) in [
        ("synthétique-281", "synthe\u{301}tique-281"),
        ("synthe\u{301}tique-281", "synthétique-281"),
    ] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.call(&chat, "p1", "write_page", json!({"name":"unicode.page.tsx","title":spelling,"source":PAGE.replace("Trois forfaits comparés.",spelling)}));
        f.document(&chat, "d1", spelling, "md");
        let id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
        f.db().execute("INSERT INTO wrapper_page_anchors(page_id,message_id,conversation_id,passage,source_version,state,created_at) VALUES (?1,'anchor-r5',?2,?3,'version','open',1)",rusqlite::params![id,chat,spelling]).unwrap();
        vault(&f, "UNICODE_TEST", "Secret synthétique", secret);
        let list = f.list("");
        let page = body(&mut f, "GET", &format!("/api/pages/{id}"), json!(null));
        let doc = list.iter().find(|item| item["type"] == "document").unwrap();
        let path = body(
            &mut f,
            "GET",
            &format!("/api/generated/{}/save-path", doc["id"].as_str().unwrap()),
            json!(null),
        );
        for shown in [json!(list).to_string(), page, path] {
            assert!(
                !shown.replace("e\u{301}", "é").contains("synthétique-281"),
                "canonically equivalent private text was exposed"
            );
        }
    }
}

#[test]
fn r5_masked_copies_stay_masked_after_vault_removal_rotation_and_restart() {
    for rotate in [false, true] {
        let (mut f, _) = leaky();
        let page_id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
        let doc_id = f.list("type=document")[0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let page_url = format!("/api/pages/{page_id}");
        let save_url = format!("/api/generated/{doc_id}/save-path");
        f.db()
            .execute("UPDATE bot_pages SET previous_source=source", [])
            .unwrap();
        for url in [&page_url, &save_url] {
            assert!(!body(&mut f, "GET", url, json!(null)).contains(LEAK));
        }
        let v = grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json")).unwrap();
        if rotate {
            v.set(
                "LIENS_TEST",
                "Secret synthétique",
                "replacement-synthetic-281",
            )
            .unwrap();
        } else {
            v.remove("LIENS_TEST").unwrap();
        }
        for restarted in [false, true] {
            if restarted {
                f.before_the_migration();
            }
            let list = f.app.api("GET", "/api/livrables", b"").unwrap();
            assert!(
                !list.to_string().contains(LEAK),
                "the list redisclosed a previously masked title"
            );
            for url in [&page_url, &save_url] {
                assert!(
                    !body(&mut f, "GET", url, json!(null)).contains(LEAK),
                    "a previously masked copy reappeared"
                );
            }
            let renamed = f.post(
                "/api/pages/rename",
                json!({"id":page_id,"title":format!("Renommée {LEAK}")}),
            );
            assert!(
                !renamed.to_string().contains(LEAK),
                "rename redisclosed private text"
            );
        }
        let reverted = f.post("/api/pages/revert", json!({"id":page_id}));
        assert!(
            !reverted.to_string().contains(LEAK),
            "revert redisclosed private text"
        );
    }
}

#[test]
fn r5_sensitive_envelope_or_identity_is_refused_without_a_broken_contract() {
    for target in ["livrables", "bot_id", "id", "entry_id", "open_url", "date"] {
        let mut f = Fixture::new();
        let chat = f.chat("alpha");
        f.document(&chat, "d1", "Public", "md");
        let item = f.list("")[0].clone();
        let secret = match target {
            "livrables" => "livrables".to_owned(),
            "bot_id" => {
                f.db()
                    .execute(
                        "UPDATE wrapper_bots SET id='private-bot-281' WHERE id='alpha'",
                        [],
                    )
                    .unwrap();
                f.db()
                    .execute("UPDATE wrapper_livrables SET bot_id='private-bot-281'", [])
                    .unwrap();
                "private-bot-281".into()
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
fn r5_save_path_does_not_recompose_another_private_value() {
    let (mut f, _) = leaky();
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
fn r5_state_and_the_historical_card_mask_a_late_private_value() {
    let (mut f, _) = leaky();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(
        !state["pages"].to_string().contains(LEAK),
        "the alternate Page list exposed a private title"
    );
    assert!(
        !state["conversations"].to_string().contains(LEAK),
        "the historical card exposed a private value"
    );
}

#[test]
fn r5_conversation_mutations_mask_the_same_historical_cards_as_state() {
    let (mut f, chat) = leaky();
    let output = f.post(
        "/api/conversations/update",
        json!({"id":chat,"title":"Renommée"}),
    );
    assert!(!output.to_string().contains(LEAK));
}

#[test]
fn r5_masked_historical_id_cannot_resurrect_a_deleted_document_as_unknown() {
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
fn r5_real_old_page_schema_migrates_without_entry_id() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.page(&chat, "p1", "ancien.page.tsx", "Ancien");
    f.db()
        .execute_batch("ALTER TABLE bot_pages DROP COLUMN entry_id")
        .unwrap();
    f.before_the_migration();
    let items = f.list("");
    assert_eq!(items.len(), 1);
    assert!(items[0]["entry_id"].as_str().is_some());
    f.opens(&items[0]);
}

#[test]
fn r5_private_envelope_whitespace_does_not_escape_title_trim() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    let secret = "  synthetic-spaces-281  ";
    f.page(&chat, "p1", "espaces.page.tsx", secret);
    vault(
        &f,
        "SITE_SPACES_TEST",
        "Formulaire de site privé",
        &json!({"values":{"password":secret}}).to_string(),
    );
    let output = f.app.api("GET", "/api/livrables", b"").unwrap();
    assert!(
        !output.to_string().contains(secret.trim()),
        "title trimming exposed the significant private content"
    );
}

#[test]
fn r5_page_integer_cannot_expose_a_private_value_after_javascript_rounding() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    let source = "export default function Page() { const rows = [[9007199254740993]]; return (<article>{rows.map((row) => (<p>{row[0]}</p>))}</article>); }";
    f.call(
        &chat,
        "p1",
        "write_page",
        json!({"name":"entier.page.tsx","title":"Entier","source":source}),
    );
    let id = f.list("")[0]["id"].as_str().unwrap().to_owned();
    vault(&f, "INTEGER_TEST", "Secret synthétique", "9007199254740992");
    match f.app.api("GET", &format!("/api/pages/{id}"), b"") {
        Err(e) => assert!(!e.message.contains("9007199254740992")),
        Ok(page) => {
            let text = rendered_page(&page["page"]["document"], None);
            assert!(
                !text.contains("9007199254740993"),
                "this integer rounds to a private value in JavaScript"
            );
            assert!(!text.contains("9007199254740992"));
        }
    }
}

#[test]
fn r5_vault_unavailable_after_rename_commit_is_a_closed_error() {
    let (mut f, _) = leaky();
    let id = f.list("type=page")[0]["id"].as_str().unwrap().to_owned();
    let path = f.path.with_extension("secrets.json");
    let saved = f.path.with_extension("secrets.saved");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::create_dir(&path).unwrap();
    let response = f.app.api(
        "POST",
        "/api/pages/rename",
        json!({"id":id,"title":format!("Après {LEAK}")})
            .to_string()
            .as_bytes(),
    );
    let error = response.unwrap_err();
    assert!(!error.message.contains(LEAK));
    let title: String = f
        .db()
        .query_row("SELECT title FROM bot_pages WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        title,
        format!("Après {LEAK}"),
        "the commit happened; the failure concerns emission only"
    );
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&saved, &path).unwrap();
    assert!(!f
        .app
        .api("GET", &format!("/api/pages/{id}"), b"")
        .unwrap()
        .to_string()
        .contains(LEAK));
}

#[test]
fn r5_record_failure_does_not_publish_a_livrable() {
    let mut f = Fixture::new();
    let chat = f.chat("alpha");
    f.db().execute_batch("CREATE TRIGGER reject_livrable BEFORE INSERT ON wrapper_livrables BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
    f.document(&chat, "d1", "Refusé", "md");
    assert!(f.list("").is_empty());
    let statuses: Vec<String> = f.db().prepare("SELECT json_extract(payload,'$.outcome.status') FROM engine_run_events WHERE kind='tool_result'").unwrap().query_map([],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    assert!(!statuses.contains(&"done".to_owned()));
    f.db()
        .execute_batch("DROP TRIGGER reject_livrable")
        .unwrap();
    f.before_the_migration();
    assert!(f.list("").is_empty());
}
