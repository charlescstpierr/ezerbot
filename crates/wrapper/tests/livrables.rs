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
