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
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    /// Contenu complet des tables que `/api/send` écrit.
    fn snapshot(&self) -> Vec<(&'static str, Vec<String>)> {
        let db = self.db();
        [
            "submissions",
            "wrapper_remote",
            "wrapper_messages",
            "wrapper_group_inputs",
            "wrapper_skill_launches",
            "wrapper_page_anchors",
            "attachments_message_links",
            "wrapper_conversations",
        ]
        .map(|table| {
            let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = statement.column_count();
            let mut rows: Vec<String> = statement
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (table, rows)
        })
        .into()
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn tool(id: &str, source: &str) -> ScriptedResponse {
        Self::call(
            id,
            "write_page",
            json!({"name":"forfaits.page.tsx","title":"Trois forfaits","source":source}),
        )
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
fn the_thread_carries_each_messages_anchor_for_the_bubble_chip() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(
        &chat,
        "citee",
        Some(Fixture::anchor(&page, "Trois forfaits")),
    )
    .unwrap();
    f.finish("citee");
    f.answer(None);
    f.send_in(&chat, "libre", None).unwrap();
    f.finish("libre");

    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let messages = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == chat.as_str())
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .clone();
    let payload = |id: &str| messages.iter().find(|m| m["id"] == id).unwrap()["payload"].clone();
    assert_eq!(
        payload("citee")["page_anchor"],
        json!({"page_id": page["id"], "title": page["title"], "passage": "Trois forfaits"})
    );
    assert_eq!(payload("libre")["page_anchor"], Value::Null);
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

    let settled = f.page()["anchors"].clone();
    let padded = Fixture::anchor(&page, "  Confort ");
    f.send_in(&chat, "retouche", Some(padded)).unwrap();
    let anchors = f.page()["anchors"].clone();
    assert_eq!(anchors.as_array().unwrap().len(), 1);
    assert_eq!(anchors[0]["state"], "done");
    assert_eq!(anchors, settled);
}

#[test]
fn a_resend_cannot_add_remove_or_change_the_anchor_of_the_first_send() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(&chat, "sans-ancre", None).unwrap();
    f.finish("sans-ancre");
    f.send_in(&chat, "ancre", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("ancre");
    let skill = f.post(
        "/api/skills",
        json!({"bot_id": "grokbot", "name": "Résumé", "body": "Résume.", "description": "Test"}),
    )["skill"]["skill_id"]
        .clone();
    let before = f.snapshot();
    assert!(before
        .iter()
        .filter(|(table, _)| *table == "wrapper_page_anchors" || *table == "wrapper_group_inputs")
        .all(|(_, rows)| !rows.is_empty()));

    let elsewhere = json!({"page_id": format!("page_{}", "0".repeat(64)), "source_version": page["source_version"], "passage": "Confort"});
    let cases = [
        ("sans-ancre", Some(Fixture::anchor(&page, "Confort"))),
        ("ancre", None),
        ("ancre", Some(Fixture::anchor(&page, "Trois forfaits"))),
        ("ancre", Some(elsewhere)),
    ];
    for (id, anchor) in cases {
        let mut body = json!({"conversation_id": chat, "message_id": id, "body": "Retouche ce passage", "apply_skill_id": skill});
        if let Some(anchor) = anchor {
            body["page_anchor"] = anchor;
        }
        let error = f
            .app
            .api("POST", "/api/send", body.to_string().as_bytes())
            .unwrap_err();
        assert_eq!(error.status, 409, "{id} : {error}");
        assert!(error.to_string().contains("autre ancre"), "{error}");
        assert_eq!(f.snapshot(), before, "{id}");
    }
}

#[test]
fn a_resend_is_judged_while_the_first_answer_is_still_running() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    let gate = Arc::new(AtomicBool::new(false));
    let at = f.server.seen.lock().unwrap().bodies.len() + 1;
    f.server.seen.lock().unwrap().response_gate = Some((at, gate.clone()));
    let chat = f.chat.clone();
    f.send_in(&chat, "ancre", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    wait("réponse retenue", || {
        f.server.seen.lock().unwrap().bodies.len() >= at
    });
    assert!(!f.run("ancre").status.is_terminal());
    let kept = f.page()["anchors"].clone();
    assert_eq!(kept.as_array().unwrap().len(), 1);

    for anchor in [None, Some(Fixture::anchor(&page, "Famille"))] {
        let error = f.send_in(&chat, "ancre", anchor).unwrap_err();
        assert_eq!(error.status, 409, "{error}");
        assert_eq!(f.page()["anchors"], kept);
    }
    f.send_in(&chat, "ancre", Some(Fixture::anchor(&page, " Confort ")))
        .unwrap();
    assert_eq!(f.page()["anchors"], kept);
    gate.store(true, Ordering::SeqCst);
    f.finish("ancre");
    assert_eq!(f.page()["anchors"], kept);
}

#[test]
fn the_same_message_id_in_another_conversation_is_a_first_send() {
    let mut f = after_first_page();
    let page = f.page();
    f.answer(None);
    f.answer(None);
    let chat = f.chat.clone();
    f.send_in(&chat, "ancre", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("ancre");
    let other = f.post("/api/conversations", json!({"title": "Autre"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.send_in(&other, "ancre", Some(Fixture::anchor(&page, "Famille")))
        .unwrap();
    let mut passages: Vec<_> = f.page()["anchors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|anchor| anchor["passage"].as_str().unwrap().to_owned())
        .collect();
    passages.sort();
    assert_eq!(passages, ["Confort", "Famille"]);
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

const BASE: &str = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 9 €</p>\n      <p>Confort : 19 €</p>\n    </article>\n  );\n}\n";
const EDITED: &str = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 9 €</p>\n      <p>Confort : 21 €</p>\n    </article>\n  );\n}\n";

impl Fixture {
    fn call(id: &str, name: &str, arguments: Value) -> ScriptedResponse {
        ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":arguments.to_string()}}),
            json!({"type":"response.completed"}),
        ])
    }
    fn change(&mut self, message: &str, call: ScriptedResponse) -> Value {
        let chat = self.chat.clone();
        self.answer(Some(call));
        self.send_in(&chat, message, None).unwrap();
        self.finish(message);
        self.outcome(message, message)
    }
    fn write(&mut self, message: &str, source: &str) -> Value {
        self.change(message, Self::tool(message, source))
    }
    fn edit(&mut self, message: &str, id: &str, before: &str, after: &str) -> Value {
        let call = Self::call(
            message,
            "edit_page",
            json!({"id":id,"before":before,"after":after}),
        );
        self.change(message, call)
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
    /// Every column of the row, and the file next to it.
    fn record(&self, id: &str) -> (Vec<rusqlite::types::Value>, String) {
        let db = self.db();
        let mut statement = db.prepare("SELECT * FROM bot_pages WHERE id = ?1").unwrap();
        let columns = statement.column_count();
        let row = statement
            .query_row([id], |r| (0..columns).map(|i| r.get(i)).collect())
            .unwrap();
        (row, std::fs::read_to_string(self.file()).unwrap())
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
    let page = f.page();
    assert_eq!(page["status"], "ready");
    assert!(page["preview"].as_str().unwrap().contains("Confort : 21 €"));
    assert_eq!(f.page()["has_previous"], true);
}

#[test]
fn edit_refuses_a_passage_that_is_missing_empty_or_not_unique() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let written = f.record(&id);
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
        assert_eq!(f.record(&id), written, "{message}");
    }
    assert_eq!(f.page()["has_previous"], false);
}

#[test]
fn edit_that_breaks_the_compilation_changes_neither_the_row_nor_the_file() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.edit("retouche", &id, "Confort : 19 €", "Confort : 21 €");
    assert_eq!(f.stored(&id), (EDITED.to_string(), Some(BASE.to_string())));
    let before = f.record(&id);

    let outcome = f.edit("casse", &id, "<h1>Forfaits</h1>", "<h1>Forfaits</h2>");
    assert_eq!(outcome["status"], "failed", "{outcome}");
    assert_eq!(
        outcome["reason"],
        "ligne 4, caractère 19 : balise « h1 » non fermée. Les lignes comptent après le remplacement ; rien n’a été modifié."
    );
    assert_eq!(f.record(&id), before);
    let page = f.page();
    assert_eq!(page["status"], "ready");
    assert_eq!(page["has_previous"], true);
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
    f.answer(Some(Fixture::call(
        "intrus",
        "edit_page",
        json!({"id":id,"before":"Confort : 19 €","after":"Confort : 1 €"}),
    )));
    f.send_in(&other, "intrus", None).unwrap();
    f.finish("intrus");
    let outcome = f.outcome("intrus", "intrus");
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
    assert_eq!(f.page()["has_previous"], false);
    f.edit("un", &id, "Essentiel : 9 €", "Essentiel : 10 €");
    let one = f.page();
    f.edit("deux", &id, "Confort : 19 €", "Confort : 21 €");
    let two = f.page();
    assert_ne!(one["preview"], two["preview"]);
    assert_ne!(one["document"], two["document"]);
    let second = "export default function Page() {\n  return (\n    <article>\n      <h1>Forfaits</h1>\n      <p>Essentiel : 10 €</p>\n      <p>Confort : 21 €</p>\n    </article>\n  );\n}\n";
    let first = second.replace("21 €", "19 €");
    assert_eq!(f.stored(&id), (second.to_string(), Some(first.clone())));

    let back = f.post("/api/pages/revert", json!({"id":id}));
    assert_eq!(back["page"]["has_previous"], false);
    assert_eq!(back["page"]["status"], "ready");
    for restored in [&back["page"], &f.page()] {
        assert_eq!(restored["document"], one["document"]);
        assert_eq!(restored["preview"], one["preview"]);
        assert_eq!(restored["source_version"], one["source_version"]);
    }
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
fn a_kept_version_that_no_longer_compiles_is_refused_and_nothing_changes() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.edit("retouche", &id, "Confort : 19 €", "Confort : 21 €");
    f.db()
        .execute(
            "UPDATE bot_pages SET previous_source = ?1 WHERE id = ?2",
            rusqlite::params![BROKEN, id],
        )
        .unwrap();
    let kept = f.record(&id);

    let refused = f
        .app
        .api(
            "POST",
            "/api/pages/revert",
            json!({"id":id}).to_string().as_bytes(),
        )
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("ligne 6, caractère 7 : balise « td » non fermée. Rien n’a été modifié."),
        "{refused}"
    );
    assert_eq!(f.record(&id), kept);
    assert_eq!(f.page()["has_previous"], true);
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
    assert_eq!(f.page()["status"], "error");
    assert_eq!(
        f.stored(&id),
        (BROKEN.to_string(), Some(EDITED.to_string()))
    );
    f.write("repare", FIXED);
    assert_eq!(f.stored(&id), (FIXED.to_string(), Some(EDITED.to_string())));
    f.post("/api/pages/revert", json!({"id":id}));
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), EDITED);
}

#[test]
fn a_successful_edit_settles_the_anchor_with_the_new_source_version() {
    let mut f = after_first_page();
    let page = f.page();
    let id = page["id"].as_str().unwrap().to_owned();
    f.answer(Some(ScriptedResponse::events(&[
        json!({"type":"response.output_item.done","item":{"id":"fc-edit-ok","type":"function_call","status":"completed","call_id":"edit-ok","name":"edit_page","arguments":json!({"id":id,"before":"19 €","after":"21 €"}).to_string()}}),
        json!({"type":"response.completed"}),
    ])));
    let chat = f.chat.clone();
    f.send_in(&chat, "retouche", Some(Fixture::anchor(&page, "Confort")))
        .unwrap();
    f.finish("retouche");

    assert_eq!(f.outcome("retouche", "edit-ok")["status"], "done");
    assert_eq!(
        std::fs::read_to_string(f.file()).unwrap(),
        FIXED.replace("19 €", "21 €")
    );
    let after = f.page();
    assert_ne!(after["source_version"], page["source_version"]);
    assert_eq!(after["anchors"][0]["state"], "done");
    assert_eq!(
        after["anchors"][0]["source_version"],
        after["source_version"]
    );
}

#[test]
fn a_refused_edit_leaves_the_anchor_open_with_its_original_version() {
    let mut f = after_first_page();
    let page = f.page();
    let id = page["id"].as_str().unwrap().to_owned();
    let chat = f.chat.clone();
    for (message, before, after, reason) in [
        ("absent", "Platine : 49 €", "X", "Passage introuvable"),
        (
            "casse",
            "<h1>Trois forfaits</h1>",
            "<h1>Trois forfaits</h2>",
            "rien n’a été modifié",
        ),
    ] {
        f.answer(Some(Fixture::call(
            message,
            "edit_page",
            json!({"id":id,"before":before,"after":after}),
        )));
        f.send_in(&chat, message, Some(Fixture::anchor(&page, "Confort")))
            .unwrap();
        f.finish(message);
        let outcome = f.outcome(message, message);
        assert_eq!(outcome["status"], "failed", "{outcome}");
        assert!(outcome["reason"].as_str().unwrap().contains(reason));
    }

    let after = f.page();
    assert_eq!(after["source_version"], page["source_version"]);
    let anchors = after["anchors"].as_array().unwrap();
    assert_eq!(anchors.len(), 2);
    for anchor in anchors {
        assert_eq!(anchor["state"], "open", "{anchor}");
        assert_eq!(anchor["source_version"], page["source_version"]);
    }
}

#[test]
fn the_size_limit_counts_the_bytes_of_the_resulting_source() {
    let mut f = Fixture::new();
    let id = f.write("ecrit", BASE)["output"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = "Confort : 19 €";
    let room = 100_000 - (BASE.len() - before.len()) - "Confort : ".len();
    let sized = |bytes: usize| {
        format!(
            "Confort : {}{}",
            "é".repeat(bytes / 2),
            "x".repeat(bytes % 2)
        )
    };
    let written = f.record(&id);

    let over = sized(room + 1);
    let resulting = BASE.replacen(before, &over, 1);
    assert_eq!(resulting.len(), 100_001);
    assert!(resulting.chars().count() < 100_000);
    let refused = f.edit("trop", &id, before, &over);
    assert_eq!(refused["status"], "failed", "{refused}");
    assert!(
        refused["reason"]
            .as_str()
            .unwrap()
            .contains("dépasse 100 000 octets"),
        "{refused}"
    );
    assert_eq!(f.record(&id), written);

    let exact = sized(room);
    let resulting = BASE.replacen(before, &exact, 1);
    assert_eq!(resulting.len(), 100_000);
    let accepted = f.edit("pile", &id, before, &exact);
    assert_eq!(accepted["status"], "done", "{accepted}");
    assert_eq!(f.stored(&id).0, resulting);
    assert_eq!(std::fs::read_to_string(f.file()).unwrap(), resulting);
    assert_eq!(f.page()["status"], "ready");
}
