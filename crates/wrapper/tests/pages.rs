//! Page locale : le fil demande un tableau interactif, le bot écrit un `.page.tsx`,
//! une compilation cassée est renvoyée avec la ligne et le caractère, le fichier
//! réécrit s’affiche, et il est encore là après redémarrage. Aucune URL publique.
mod support;
use grokbot_engine::{
    contracts::RunEventKind,
    model::{Run, RunStatus},
    store,
};
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
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

const ASK: &str = "Compare mes 3 forfaits dans un tableau interactif";
const BROKEN: &str = r#"export default function Page() {
  return (
    <table>
      <tr>
        <td>Essentiel
      </tr>
    </table>
  );
}
"#;
const FIXED: &str = r#"export default function Page() {
  const [offre, setOffre] = useState("Essentiel");
  const lignes = [
    ["Essentiel", "9 €"],
    ["Confort", "19 €"],
    ["Famille", "29 €"],
  ];
  return (
    <article>
      <h1>Trois forfaits</h1>
      <p>Offre choisie : {offre}</p>
      <table>
        <thead>
          <tr>
            <th>Offre</th>
            <th>Prix</th>
          </tr>
        </thead>
        <tbody>
          {lignes.map((ligne) => (
            <tr>
              <td>{ligne[0]}</td>
              <td>{ligne[1]}</td>
              <td>
                <button onClick={() => setOffre(ligne[0])}>Choisir</button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </article>
  );
}
"#;
const DIAGNOSTIC: &str = "ligne 6, caractère 7 : balise « td » non fermée. Réécrivez le fichier .page.tsx ; Charles ne l’édite pas.";

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
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "ezerbot-pages-{}-{}",
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
        wait("models", || provider.status()["models_ready"] == true);
        let chat = app
            .api(
                "POST",
                "/api/conversations",
                r#"{"title":"Forfaits"}"#.as_bytes(),
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
    fn tool(id: &str, source: &str) -> ScriptedResponse {
        ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":"write_page","arguments":json!({"name":"forfaits.page.tsx","title":"Trois forfaits","source":source}).to_string()}}),
            json!({"type":"response.completed"}),
        ])
    }
    fn ask(&mut self) {
        let mut seen = self.server.seen.lock().unwrap();
        seen.responses.push_back(Self::tool("page-broken", BROKEN));
        seen.responses.push_back(Self::tool("page-fixed", FIXED));
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"La comparaison est prête à côté du fil."}),
            json!({"type":"response.completed"}),
        ]));
        drop(seen);
        self.post(
            "/api/send",
            json!({"conversation_id":self.chat,"message_id":"compare","body":ASK}),
        );
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
    fn outcome(&self, message: &str, id: &str) -> Value {
        store::events(&self.db(), &self.run(message).run_id, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == id)
            .unwrap()
            .payload["outcome"]
            .clone()
    }
    fn file(&self) -> PathBuf {
        self.path
            .parent()
            .unwrap()
            .join("pages/grokbot/forfaits.page.tsx")
    }
    fn reopen(&mut self) {
        assert_eq!(self.provider.status()["titling"], false);
        self.provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
        wait("reopened models", || {
            self.provider.status()["models_ready"] == true
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap().parent().unwrap());
    }
}

fn assert_no_public_address(value: &Value) {
    let text = value.to_string();
    assert!(
        !text.contains("http://") && !text.contains("https://"),
        "{text}"
    );
    fn walk(value: &Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let folded = key.to_ascii_lowercase();
                    assert!(
                        !matches!(
                            folded.as_str(),
                            "url" | "share_url" | "public_url" | "href" | "lien"
                        ),
                        "clé publique {key}"
                    );
                    walk(child);
                }
            }
            Value::Array(items) => items.iter().for_each(walk),
            _ => {}
        }
    }
    walk(value);
}

#[test]
fn interactive_table_becomes_a_local_page_and_survives_restart() {
    let mut f = Fixture::new();
    f.ask();
    f.finish("compare");
    assert_eq!(f.run("compare").status, RunStatus::Finished);

    let broken = f.outcome("compare", "page-broken");
    assert_eq!(broken["status"], "failed", "{broken}");
    assert_eq!(broken["reason"], DIAGNOSTIC);

    let fixed = f.outcome("compare", "page-fixed");
    assert_eq!(fixed["status"], "done", "{fixed}");
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), FIXED);

    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let conversation = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == f.chat)
        .unwrap();
    assert!(conversation["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "user" && m["body"] == ASK));
    let replies: Vec<_> = conversation["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "assistant")
        .map(|m| m["body"].as_str().unwrap_or(""))
        .collect();
    let thread = replies.join("\n");
    assert!(
        thread.contains("à côté du fil"),
        "le fil doit annoncer la Page : {thread}"
    );
    assert!(
        !thread.contains("| ---") && !thread.contains("|---|"),
        "le fil ne doit pas s’arrêter à un tableau Markdown : {thread}"
    );

    let listed = &state["pages"]["by_bot"]["grokbot"][0];
    assert_eq!(listed["title"], "Trois forfaits");
    assert_eq!(listed["filename"], "forfaits.page.tsx");
    assert_eq!(listed["status"], "ready");
    assert_eq!(listed["conversation_id"], f.chat);
    assert_no_public_address(listed);
    let id = listed["id"].as_str().unwrap();
    let detail = f.app.api("GET", &format!("/api/pages/{id}"), b"").unwrap();
    let preview = detail["page"]["preview"].as_str().unwrap();
    for word in [
        "Trois forfaits",
        "Essentiel",
        "Confort",
        "Famille",
        "Choisir",
    ] {
        assert!(preview.contains(word), "{word} absent de {preview}");
    }
    assert_eq!(detail["page"]["status"], "ready");
    assert!(detail["page"]["document"].is_object());
    assert_no_public_address(&detail);

    for path in ["/api/pages/share", "/api/pages/publish"] {
        let error = f.app.api("POST", path, b"{}").unwrap_err();
        assert!(
            error.to_string().contains("n'existe pas"),
            "{path} : {error}"
        );
    }

    f.reopen();
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), FIXED);
    let again = f.app.api("GET", "/api/state", b"").unwrap();
    let page = &again["pages"]["by_bot"]["grokbot"][0];
    assert_eq!(page["id"], id);
    assert_eq!(page["title"], "Trois forfaits");
    assert_eq!(page["status"], "ready");
    let renamed = f.post(
        "/api/pages/rename",
        json!({"id": id, "title": "Mes forfaits"}),
    );
    assert_eq!(renamed["page"]["title"], "Mes forfaits");
    assert_no_public_address(&renamed);
    let listed = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        listed["pages"]["by_bot"]["grokbot"][0]["title"],
        "Mes forfaits"
    );
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), FIXED);
}

impl Fixture {
    fn page(&mut self) -> Value {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        let id = state["pages"]["by_bot"]["grokbot"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.app
            .api("GET", &format!("/api/pages/{id}"), b"")
            .unwrap()["page"]
            .clone()
    }
    fn answer(&self, write: Option<ScriptedResponse>) {
        let mut seen = self.server.seen.lock().unwrap();
        seen.responses.extend(write);
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"C’est modifié."}),
            json!({"type":"response.completed"}),
        ]));
    }
    fn anchor(page: &Value, passage: &str) -> Value {
        json!({"page_id": page["id"], "source_version": page["source_version"], "passage": passage})
    }
    fn send_in(
        &mut self,
        chat: &str,
        message: &str,
        anchor: Option<Value>,
    ) -> Result<Value, grokbot_wrapper::ApiError> {
        let mut body =
            json!({"conversation_id":chat,"message_id":message,"body":"Retouche ce passage"});
        if let Some(anchor) = anchor {
            body["page_anchor"] = anchor;
        }
        self.app
            .api("POST", "/api/send", body.to_string().as_bytes())
    }
    fn has_message(&mut self, chat: &str, message: &str) -> bool {
        let state = self.app.api("GET", "/api/state", b"").unwrap();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["id"] == chat)
            .flat_map(|c| c["messages"].as_array().unwrap())
            .any(|m| m["id"] == message)
    }
    fn instructions_with(&self, needle: &str) -> Vec<String> {
        self.server
            .seen
            .lock()
            .unwrap()
            .bodies
            .iter()
            .filter_map(|body| body["instructions"].as_str())
            .filter(|text| text.contains(needle))
            .map(str::to_owned)
            .collect()
    }
}

fn after_first_page() -> Fixture {
    let mut f = Fixture::new();
    f.ask();
    f.finish("compare");
    f
}

#[test]
fn anchored_message_is_kept_and_the_bot_reads_the_cited_lines() {
    let mut f = after_first_page();
    let page = f.page();
    assert_eq!(page["anchors"], json!([]));
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(
        &chat,
        "retouche",
        Some(Fixture::anchor(&page, "Trois forfaits")),
    )
    .unwrap();
    f.finish("retouche");

    assert_eq!(
        f.page()["anchors"],
        json!([{
            "message_id": "retouche",
            "passage": "Trois forfaits",
            "state": "open",
            "source_version": page["source_version"]
        }])
    );
    let line = FIXED.lines().position(|l| l.contains("<h1>")).unwrap() + 1;
    let source_line = FIXED
        .lines()
        .nth(line - 1)
        .unwrap()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    let blocks = f.instructions_with("Passage cité");
    assert!(!blocks.is_empty());
    for block in blocks {
        assert!(
            block.contains("Passage cité : « Trois forfaits »"),
            "{block}"
        );
        assert!(block.contains(&format!("{line}: {source_line}")), "{block}");
        assert!(block.contains("Change seulement cet endroit"), "{block}");
    }
}

#[test]
fn a_passage_missing_from_the_source_reaches_the_bot_as_a_quote_only() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(
        &chat,
        "prix",
        Some(Fixture::anchor(&page, "Offre choisie : Essentiel")),
    )
    .unwrap();
    f.finish("prix");
    let blocks = f.instructions_with("Passage cité : « Offre choisie : Essentiel »");
    assert!(!blocks.is_empty());
    assert!(blocks
        .iter()
        .all(|b| b.contains("citation seule") && !b.contains("Lignes de la source")));
}

#[test]
fn anchor_is_settled_after_a_successful_write_in_the_same_answer() {
    let mut f = after_first_page();
    let page = f.page();
    let rewritten = FIXED.replace("19 €", "21 €");
    f.answer(Some(Fixture::tool("edit-ok", &rewritten)));
    let chat = f.chat.clone();
    f.send_in(&chat, "retouche", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("retouche");

    let after = f.page();
    assert_ne!(after["source_version"], page["source_version"]);
    assert_eq!(after["anchors"][0]["state"], "done");
    assert_eq!(
        after["anchors"][0]["source_version"],
        after["source_version"]
    );
}

#[test]
fn anchor_stays_open_after_a_compile_failure() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(Some(Fixture::tool("edit-bad", BROKEN)));
    let chat = f.chat.clone();
    f.send_in(&chat, "retouche", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("retouche");

    let failed = f.outcome("retouche", "edit-bad");
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(failed["reason"], DIAGNOSTIC);
    let after = f.page();
    assert_eq!(after["anchors"][0]["state"], "open");
    assert_eq!(
        after["anchors"][0]["source_version"],
        page["source_version"]
    );
}

#[test]
fn refused_anchors_create_no_message() {
    let mut f = after_first_page();
    let page = f.page();
    f.post(
        "/api/bots",
        json!({"id": "autre", "name": "Autre", "color": "orange"}),
    );
    let other = f.post("/api/conversations", json!({"bot_id": "autre"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let chat = f.chat.clone();
    let unknown = json!({"page_id": format!("page_{}", "0".repeat(64)), "source_version": page["source_version"], "passage": "Confort"});
    let cases = [
        (other, Fixture::anchor(&page, "Confort"), "autre bot"),
        (
            chat.clone(),
            Fixture::anchor(&page, "Absent de la Page"),
            "n’apparaît pas",
        ),
        (
            chat.clone(),
            Fixture::anchor(&page, &"a".repeat(501)),
            "500 caractères",
        ),
        (
            chat.clone(),
            Fixture::anchor(&page, "   "),
            "500 caractères",
        ),
        (chat.clone(), unknown, "introuvable"),
    ];
    for (n, (conversation, anchor, reason)) in cases.into_iter().enumerate() {
        let id = format!("refuse-{n}");
        let error = f.send_in(&conversation, &id, Some(anchor)).unwrap_err();
        assert!(error.to_string().contains(reason), "{reason} : {error}");
        assert!(!f.has_message(&conversation, &id), "{reason}");
    }
    assert_eq!(f.page()["anchors"], json!([]));
}

#[test]
fn deleting_the_conversation_deletes_its_anchors() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(&chat, "retouche", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("retouche");
    assert_eq!(f.page()["anchors"].as_array().unwrap().len(), 1);

    f.post("/api/conversations/delete", json!({"id": chat}));
    assert_eq!(f.page()["anchors"], json!([]));
}

#[test]
fn a_message_without_anchor_behaves_as_before() {
    let mut f = after_first_page();
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(&chat, "merci", None).unwrap();
    f.finish("merci");
    assert!(f.has_message(&chat, "merci"));
    assert_eq!(f.page()["anchors"], json!([]));
    assert!(f.instructions_with("Passage cité").is_empty());
}

#[test]
fn retrying_an_accepted_anchored_message_survives_the_edit_that_removed_its_passage() {
    let mut f = after_first_page();
    let page = f.page();
    let rewritten = FIXED.replace("Confort", "Premium");
    f.answer(Some(Fixture::tool("edit-ok", &rewritten)));
    let chat = f.chat.clone();
    let anchor = Fixture::anchor(&page, "Confort");
    f.send_in(&chat, "retouche", Some(anchor.clone())).unwrap();
    f.finish("retouche");

    f.send_in(&chat, "retouche", Some(anchor)).unwrap();
    let anchors = f.page()["anchors"].clone();
    assert_eq!(anchors.as_array().unwrap().len(), 1);
    assert_eq!(anchors[0]["state"], "done");
}

#[test]
fn an_anchor_can_only_address_the_bot_owning_the_page() {
    let mut f = after_first_page();
    let page = f.page();
    f.post(
        "/api/bots",
        json!({"id": "autre", "name": "Voisin", "color": "orange"}),
    );
    let group = f.post(
        "/api/conversations",
        json!({"bot_ids": ["grokbot", "autre"], "title": "Groupe"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let send = |f: &mut Fixture, id: &str, mention: &str, label: &str| {
        let body = json!({"conversation_id": group, "message_id": id, "body": "Retouche ce passage",
            "mentions": [{"id": mention, "label": label}], "page_anchor": Fixture::anchor(&page, "Confort")});
        f.app.api("POST", "/api/send", body.to_string().as_bytes())
    };
    for (id, mention, label) in [
        ("voisin", "autre", "Voisin"),
        ("tous", "@everyone", "everyone"),
    ] {
        let error = send(&mut f, id, mention, label).unwrap_err();
        assert!(error.to_string().contains("propriétaire"), "{error}");
        assert!(!f.has_message(&group, id));
    }

    f.answer(None);
    send(&mut f, "proprio", "grokbot", "Ezerbot").unwrap();
    wait("réponse du propriétaire", || {
        f.app.tick(now());
        !f.instructions_with("Passage cité").is_empty()
    });
    assert!(f.instructions_with("Tu t'appelles Voisin").is_empty());
}

/// Photo d’une Page ou d’un site local : sans session d’écran, pour un bot comme
/// pour un sous-bot, rangée comme une capture d’écran. Le rendu est l’image
/// fictive du mode test (une par thème) ; le rendu natif est un autre ticket.
#[cfg(debug_assertions)]
mod photo {
    use super::*;
    use grokbot_delegation::graph::{self, DelegationStatus};
    use grokbot_wrapper::ApiResponse;

    fn call(id: &str, name: &str, args: Value) -> ScriptedResponse {
        ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}}),
            json!({"type":"response.completed"}),
        ])
    }
    fn shot(id: &str, target: &str, theme: &str) -> ScriptedResponse {
        call(
            id,
            "screenshot_page",
            json!({"target": target, "theme": theme}),
        )
    }
    fn shot_path(conversation: &str, capture: &str) -> String {
        format!(
            "/api/screen/shot?conversation_id={}&id={}",
            conversation.replace(':', "%3A"),
            capture.replace(':', "%3A")
        )
    }
    fn fixture() -> Fixture {
        let mut f = Fixture::new();
        support::ask_before_acting(&mut f.app);
        f
    }

    impl Fixture {
        fn converse(&mut self, id: &str, calls: Vec<ScriptedResponse>) {
            self.server.seen.lock().unwrap().responses.extend(calls);
            self.post(
                "/api/send",
                json!({"conversation_id":self.chat,"message_id":id,"body":"Photographie ma Page"}),
            );
            self.finish(id);
        }
        /// L’identifiant que `write_page` rend pour la Page écrite.
        fn written(&mut self) -> String {
            self.converse("write", vec![Fixture::tool("page", FIXED)]);
            let out = self.outcome("write", "page");
            out["output"]["id"].as_str().unwrap().to_owned()
        }
        fn shots(&self) -> i64 {
            self.db()
                .query_row("SELECT count(*) FROM wrapper_screen_shots", [], |r| {
                    r.get(0)
                })
                .unwrap()
        }
        /// Les vignettes des cartes d’outil du fil, telles que l’interface les reçoit.
        fn thumbnails(&mut self) -> Vec<String> {
            let state = self.app.api("GET", "/api/state", b"").unwrap();
            state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|c| c["messages"].as_array().unwrap())
                .filter_map(|m| m["payload"]["screen_image"].as_str().map(str::to_owned))
                .collect()
        }
        fn fetch(&mut self, path: &str) -> Vec<u8> {
            match self.app.api_response("GET", path, b"").unwrap() {
                ApiResponse::Bytes { body, content_type } => {
                    assert_eq!(content_type, "image/png");
                    assert!(body.starts_with(b"\x89PNG\r\n\x1a\n"), "pas un PNG");
                    body
                }
                ApiResponse::Json(value) => panic!("image non binaire : {value}"),
            }
        }
        fn without_screen_session(&mut self) {
            let state = self.app.api("GET", "/api/state", b"").unwrap();
            assert_eq!(state["screen"]["session"], Value::Null);
            assert_eq!(state["screen"]["bots"], json!([]));
        }
    }

    #[test]
    fn a_page_photo_gives_an_image_and_a_capture_id_usable_in_the_same_thread() {
        let mut f = fixture();
        let page = f.written();
        f.converse("photo", vec![shot("shot", &page, "light")]);
        let out = f.outcome("photo", "shot");
        assert_eq!(out["status"], "done", "{out}");
        assert_eq!(out["output"]["target"], page);
        assert_eq!(out["output"]["width"], 1280);
        assert_eq!(out["output"]["height"], 800);
        let capture = out["output"]["capture_id"].as_str().unwrap();
        let chat = f.chat.clone();
        let image = f.fetch(&shot_path(&chat, capture));

        // La vignette de la carte d’outil du fil montre cette image.
        let thumbnails = f.thumbnails();
        assert_eq!(thumbnails.len(), 1, "{thumbnails:?}");
        assert_eq!(f.fetch(&thumbnails[0]), image);

        // Le modèle reçoit l’image au tour suivant, présentée comme une photo de Page.
        let bodies = f.server.seen.lock().unwrap().bodies.clone();
        let last = bodies.last().unwrap().to_string();
        assert!(last.contains("data:image/png;base64,"), "image absente");
        assert!(last.contains("Photo de la Page ou du site local"));
        assert!(!last.contains("Coordonnées en pixels"));
        f.without_screen_session();
    }

    #[test]
    fn refused_targets_leave_no_capture_and_a_local_site_is_photographed() {
        let mut f = fixture();
        f.converse("broken", vec![Fixture::tool("page", BROKEN)]);
        let broken = f.page()["id"].as_str().unwrap().to_owned();
        let missing = format!("page_{}", "0".repeat(64));
        f.converse(
            "refus",
            vec![
                shot("internet", "https://example.com/", "light"),
                shot("file", "file:///Users/charles/.ssh/id_ed25519", "light"),
                shot("missing", &missing, "light"),
                shot("unready", &broken, "light"),
                shot("bad-theme", "http://localhost:3000/", "sepia"),
            ],
        );
        for call in ["internet", "file"] {
            let out = f.outcome("refus", call);
            assert_eq!(out["status"], "failed", "{call}");
            assert!(out["reason"].as_str().unwrap().contains("localhost"));
        }
        assert_eq!(
            f.outcome("refus", "missing")["reason"],
            "Cette Page est introuvable."
        );
        let unready = f.outcome("refus", "unready");
        assert!(unready["reason"]
            .as_str()
            .unwrap()
            .starts_with("Cette Page n’est pas prête"));
        assert_eq!(f.outcome("refus", "bad-theme")["status"], "failed");
        assert_eq!(f.shots(), 0);

        f.converse(
            "local",
            vec![shot("site", "http://localhost:5173/accueil?x=1", "light")],
        );
        assert_eq!(f.outcome("local", "site")["status"], "done");
        assert_eq!(f.shots(), 1);
    }

    #[test]
    fn light_and_dark_give_two_different_images_and_light_is_the_default() {
        let mut f = fixture();
        let page = f.written();
        f.converse(
            "themes",
            vec![
                shot("clair", &page, "light"),
                shot("sombre", &page, "dark"),
                call("defaut", "screenshot_page", json!({"target": page})),
            ],
        );
        let thumbnails = f.thumbnails();
        assert_eq!(thumbnails.len(), 3, "{thumbnails:?}");
        let images: Vec<_> = thumbnails.iter().map(|path| f.fetch(path)).collect();
        assert_ne!(images[0], images[1]);
        assert_eq!(images[0], images[2]);
    }

    #[test]
    fn a_sub_bot_without_a_screen_session_photographs_a_page_but_not_the_screen() {
        let mut f = Fixture::new();
        f.post(
            "/api/bots",
            json!({"id":"child","name":"child","color":"citron","description":"Worker","instructions":""}),
        );
        let mut seen = f.server.seen.lock().unwrap();
        seen.responses.push_back(call(
            "delegate-call",
            "delegate",
            json!({"child_bot":"child","objective":"Photographier la Page","selected_context":{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}}),
        ));
        // Le sous-bot photographie l’identifiant que son propre write_page a rendu,
        // puis tente l’écran du Mac.
        seen.responses.push_back(Fixture::tool("child-page", FIXED));
        seen.responses
            .push_back(call("child-screen", "computer_screenshot", json!({})));
        seen.responses.push_back(call(
            "finish-child",
            "complete_task",
            json!({"summary":"Page photographiée"}),
        ));
        seen.reply_for = Some(Arc::new(|body: &Value| {
            let outputs = body["input"].as_array()?;
            let last = outputs
                .iter()
                .rfind(|i| i["type"] == "function_call_output")?;
            if last["call_id"] != "child-page" {
                return None;
            }
            let text = last["output"].to_string();
            let id = text.get(text.find("page_")?..)?.get(..69)?;
            Some(call(
                "child-shot",
                "screenshot_page",
                json!({"target":id,"theme":"dark"}),
            ))
        }));
        drop(seen);
        f.post(
            "/api/send",
            json!({"conversation_id":f.chat,"message_id":"parent","body":"Délègue la photo"}),
        );
        wait("parent waits for the child", || {
            f.run("parent").status == RunStatus::AwaitingChildren
        });
        let edge = graph::descendants(&f.db(), &f.run("parent").run_id)
            .unwrap()
            .remove(0);
        f.app.tick(now());
        wait("child done", || {
            matches!(
                graph::get(&f.db(), &edge.id).unwrap().unwrap().status,
                DelegationStatus::Done { .. }
            )
        });
        wait("owner released", || f.provider.status()["titling"] == false);

        let child_thread = edge.conversation_id.clone().unwrap();
        let child_run = graph::get(&f.db(), &edge.id).unwrap().unwrap().child_run_id;
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
        assert_eq!(Some(run), child_run);
        assert!(!f.fetch(&shot_path(&child_thread, &capture)).is_empty());

        // L’écran du Mac reste refusé au sous-bot : aucune session, aucune seconde capture.
        let refused = store::events(&f.db(), child_run.as_deref().unwrap(), 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "child-screen")
            .unwrap()
            .payload["outcome"]
            .clone();
        assert_eq!(refused["status"], "failed", "{refused}");
        assert_eq!(f.shots(), 1);
        f.without_screen_session();
        // La capture du sous-bot ne s’ouvre pas depuis le fil du parent.
        assert!(f
            .app
            .api_response("GET", &shot_path(&f.chat, &capture), b"")
            .is_err());
    }
}
