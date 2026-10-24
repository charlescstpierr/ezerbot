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
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("run inachevée {:?}", self.run(id));
    }
    fn outcome(&self, id: &str) -> Value {
        self.outcome_in("compare", id)
    }
    fn outcome_in(&self, message: &str, id: &str) -> Value {
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

    let broken = f.outcome("page-broken");
    assert_eq!(broken["status"], "failed", "{broken}");
    assert_eq!(broken["reason"], DIAGNOSTIC);

    let fixed = f.outcome("page-fixed");
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

const BASE: &str = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 9 €</p>\n      <p>Confort : 19 €</p>\n    </article>\n  );\n}\n";
const EDITED: &str = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 9 €</p>\n      <p>Confort : 21 €</p>\n    </article>\n  );\n}\n";

impl Fixture {
    /// One turn of `chat`: the bot makes these calls in order, then answers.
    fn turn(&mut self, chat: &str, message: &str, calls: &[(&str, &str, Value)]) {
        let mut seen = self.server.seen.lock().unwrap();
        for (id, name, arguments) in calls {
            seen.responses.push_back(ScriptedResponse::events(&[
                json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":arguments.to_string()}}),
                json!({"type":"response.completed"}),
            ]));
        }
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"Fait."}),
            json!({"type":"response.completed"}),
        ]));
        drop(seen);
        self.post(
            "/api/send",
            json!({"conversation_id":chat,"message_id":message,"body":"Change la Page"}),
        );
        self.finish(message);
    }
    fn write(&mut self, message: &str, source: &str) -> Value {
        let chat = self.chat.clone();
        self.turn(
            &chat,
            message,
            &[(
                message,
                "write_page",
                json!({"name":"forfaits.page.tsx","title":"Forfaits","source":source}),
            )],
        );
        self.outcome_in(message, message)
    }
    fn edit(&mut self, message: &str, id: &str, before: &str, after: &str) -> Value {
        let chat = self.chat.clone();
        self.turn(
            &chat,
            message,
            &[(
                message,
                "edit_page",
                json!({"id":id,"before":before,"after":after}),
            )],
        );
        self.outcome_in(message, message)
    }
    fn stored(&self, id: &str) -> (String, Option<String>) {
        self.db()
            .query_row(
                "SELECT source, previous_source FROM bot_pages WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }
    fn page(&mut self, id: &str) -> Value {
        self.app
            .api("GET", &format!("/api/pages/{id}"), b"")
            .unwrap()["page"]
            .clone()
    }
}

#[test]
fn edit_replaces_one_passage_and_leaves_every_other_byte() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let outcome = f.edit("modifie", &id, "Confort : 19 €", "Confort : 21 €");
    assert_eq!(outcome["status"], "done", "{outcome}");
    assert_eq!(outcome["output"]["id"], id.as_str());
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), EDITED);
    assert_eq!(f.stored(&id).0, EDITED);
    let page = f.page(&id);
    assert_eq!(page["status"], "ready");
    assert!(page["preview"].as_str().unwrap().contains("Confort : 21 €"));
    assert_eq!(f.page(&id)["has_previous"], true);
}

#[test]
fn edit_refuses_a_passage_that_is_missing_empty_or_not_unique() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    for (message, before, reason) in [
        (
            "absent",
            "Platine : 49 €",
            "Passage introuvable dans la Page.",
        ),
        ("vide", "", "Le passage à remplacer est vide."),
        (
            "deux",
            "<p>",
            "Passage présent 2 fois : ajoute du contexte pour qu’il soit unique.",
        ),
    ] {
        let outcome = f.edit(message, &id, before, "X");
        assert_eq!(outcome["status"], "failed", "{outcome}");
        assert_eq!(outcome["reason"], reason);
        assert_eq!(f.stored(&id), (BASE.to_string(), None), "{message}");
        assert_eq!(std::fs::read_to_string(f.file()).unwrap(), BASE);
    }
    assert_eq!(f.page(&id)["has_previous"], false);
}

#[test]
fn edit_that_breaks_the_compilation_changes_neither_the_row_nor_the_file() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let outcome = f.edit("casse", &id, "<h1>Forfaits</h1>", "<h1>Forfaits</h2>");
    assert_eq!(outcome["status"], "failed", "{outcome}");
    assert_eq!(
        outcome["reason"],
        "ligne 4, caractère 19 : balise « h1 » non fermée. Les lignes comptent après le remplacement ; rien n’a été modifié."
    );
    assert_eq!(f.stored(&id), (BASE.to_string(), None));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), BASE);
    let page = f.page(&id);
    assert_eq!(page["status"], "ready");
    assert_eq!(page["has_previous"], false);
}

#[test]
fn edit_refuses_the_page_of_another_bot() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.post(
        "/api/bots",
        json!({"id":"autre","name":"Autre","color":"cyan"}),
    );
    let other = f.post("/api/conversations", json!({"bot_id":"autre"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.turn(
        &other,
        "intrus",
        &[(
            "intrus",
            "edit_page",
            json!({"id":id,"before":"Confort : 19 €","after":"Confort : 1 €"}),
        )],
    );
    let outcome = f.outcome_in("intrus", "intrus");
    assert_eq!(outcome["status"], "failed", "{outcome}");
    assert_eq!(outcome["reason"], "Cette Page est introuvable.");
    assert_eq!(f.stored(&id), (BASE.to_string(), None));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), BASE);
}

#[test]
fn only_the_version_before_the_last_change_can_be_put_back_and_only_once() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(f.page(&id)["has_previous"], false);
    f.edit("un", &id, "Essentiel : 9 €", "Essentiel : 10 €");
    f.edit("deux", &id, "Confort : 19 €", "Confort : 21 €");
    let second = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 10 €</p>\n      <p>Confort : 21 €</p>\n    </article>\n  );\n}\n";
    let first = second.replace("21 €", "19 €");
    assert_eq!(f.stored(&id), (second.to_string(), Some(first.clone())));

    let back = f.post("/api/pages/revert", json!({"id":id}));
    assert_eq!(back["page"]["has_previous"], false);
    assert_eq!(back["page"]["status"], "ready");
    assert_eq!(f.stored(&id), (first.clone(), None));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), first);

    let again = f.app.api(
        "POST",
        "/api/pages/revert",
        json!({"id":id}).to_string().as_bytes(),
    );
    assert!(again
        .unwrap_err()
        .to_string()
        .contains("pas de version d’avant"));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), first);
}

#[test]
fn a_rewrite_keeps_the_version_before_it_and_a_broken_write_does_not_replace_it() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.write("refonte", EDITED);
    assert_eq!(f.stored(&id), (EDITED.to_string(), Some(BASE.to_string())));
    f.write("casse", BROKEN);
    assert_eq!(f.page(&id)["status"], "error");
    assert_eq!(
        f.stored(&id),
        (BROKEN.to_string(), Some(EDITED.to_string()))
    );
    f.write("repare", FIXED);
    assert_eq!(f.stored(&id), (FIXED.to_string(), Some(EDITED.to_string())));
    f.post("/api/pages/revert", json!({"id":id}));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), EDITED);
}
