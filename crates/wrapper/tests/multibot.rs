//! Group UI/API contract against disposable databases and scripted Responses.
mod support;
use grokbot_engine::{
    contracts::*,
    model::{self, RunStatus},
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
    time::{SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};
struct Fixture {
    app: App,
    path: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new(live: bool) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "ezerbot-multibot-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        if live {
            std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"fake","client_id":"fake","access_token":"fake","refresh_token":"fake","expires_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        }
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let app = App::open_with(&path, provider.clone()).unwrap();
        if live {
            wait("catalogue", || provider.status()["models_ready"] == true);
        }
        Self {
            app,
            path,
            server,
            provider,
        }
    }
    fn post(&mut self, p: &str, v: Value) -> Value {
        self.app.api("POST", p, v.to_string().as_bytes()).unwrap()
    }
    fn bad(&mut self, p: &str, v: Value) -> u16 {
        self.app
            .api("POST", p, v.to_string().as_bytes())
            .unwrap_err()
            .status
    }
    fn bot(&mut self, id: &str) {
        self.post("/api/bots",json!({"id":id,"name":"Même nom","color":"cyan","instructions":format!("secret-prive-{id}")}));
    }
    fn group(&mut self, ids: &[&str]) -> String {
        self.post(
            "/api/conversations",
            json!({"bot_ids":ids,"title":"Groupe test"}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn send(&mut self, c: &str, m: &str, mentions: Value) -> Value {
        self.post("/api/send",json!({"conversation_id":c,"message_id":m,"body":"Bonjour au groupe","mentions":mentions}))
    }
    fn conversation(&mut self, c: &str) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == c)
            .unwrap()
            .clone()
    }
    fn db(&self) -> Connection {
        let db = Connection::open(&self.path).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(3)).unwrap();
        db
    }
    fn drain(&mut self) {
        wait("tours terminés", || {
            self.app.tick(0);
            self.db().query_row("SELECT COUNT(*)=0 FROM wrapper_group_turns WHERE status IN ('queued','running')",[],|r|r.get::<_,bool>(0)).unwrap()
        });
    }
    fn script(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn restart(&mut self) {
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn function_call(id: &str, name: &str, arguments: Value) -> Vec<Value> {
    vec![
        json!({"type":"response.output_item.done","item":{"id":id,"type":"function_call","call_id":id,"name":name,"arguments":arguments.to_string()}}),
        json!({"type":"response.completed"}),
    ]
}
fn mention(id: &str) -> Value {
    json!({"id":id,"label":"Même nom"})
}
fn text(s: &str) -> Vec<Value> {
    vec![
        json!({"type":"response.output_text.delta","delta":s}),
        json!({"type":"response.completed"}),
    ]
}
fn call(id: &str, bot: &str) -> Vec<Value> {
    vec![
        json!({"type":"response.output_item.done","item":{"id":id,"type":"function_call","call_id":id,"name":"message_bot","arguments":json!({"bot_id":bot,"text":"Donne ton avis"}).to_string()}}),
        json!({"type":"response.completed"}),
    ]
}

#[test]
fn migrate_single_add_remove_preserves_history_and_authors_after_restart() {
    let mut f = Fixture::new(false);
    f.bot("b");
    let c = f.group(&["grokbot"]);
    let old = f.send(&c, "ancien", json!([]))["conversation"].clone();
    assert_eq!(old["messages"][1]["id"], "reply-ancien");
    f.db()
        .execute("DELETE FROM wrapper_members WHERE conversation_id=?1", [&c])
        .unwrap();
    f.restart();
    assert_eq!(f.conversation(&c)["member_ids"], json!(["grokbot"]));
    let command = json!({"conversation_id":c,"bot_id":"b","command_id":"ajout"});
    f.post("/api/conversations/members/add", command.clone());
    f.post("/api/conversations/members/add", command);
    f.restart();
    let grouped = f.conversation(&c);
    assert_eq!(grouped["kind"], "group");
    assert_eq!(grouped["messages"], old["messages"]);
    assert_eq!(grouped["member_ids"], json!(["grokbot", "b"]));
    f.send(&c, "coordonner", json!([]));
    f.drain();
    let count:i64=f.db().query_row("SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='coordonner' AND bot_id='grokbot'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 1);
    f.post(
        "/api/conversations/members/remove",
        json!({"conversation_id":c,"bot_id":"grokbot","command_id":"retirer"}),
    );
    f.restart();
    let current = f.conversation(&c);
    assert_eq!(current["bot_id"], "b");
    assert_eq!(current["kind"], "group");
    assert_eq!(current["messages"][1]["payload"]["bot_id"], "grokbot");
    assert_eq!(
        f.bad(
            "/api/conversations/members/remove",
            json!({"conversation_id":c,"bot_id":"b","command_id":"dernier"})
        ),
        409
    );
}

#[test]
fn structured_mentions_deduplicate_same_names_survive_rename_and_nonce_retry() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.send(
        &c,
        "choix",
        json!([mention("b"), mention("a"), mention("b")]),
    );
    f.drain();
    f.post("/api/bots/update", json!({"id":"b","name":"Nouveau nom"}));
    f.send(
        &c,
        "choix",
        json!([mention("b"), mention("a"), mention("b")]),
    );
    f.drain();
    f.restart();
    let db = f.db();
    let mut s = db
        .prepare("SELECT bot_id FROM wrapper_group_turns WHERE root_id='choix' ORDER BY ordinal")
        .unwrap();
    let bots = s
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(bots, ["b", "a"]);
    let messages = f.conversation(&c)["messages"].as_array().unwrap().clone();
    assert_eq!(
        messages.iter().filter(|m| m["role"] == "assistant").count(),
        2
    );
    assert_eq!(messages[0]["payload"]["mentions"][0]["id"], "b");
    assert_ne!(messages[1]["id"], messages[2]["id"]);
    assert_eq!(f.bad("/api/send",json!({"conversation_id":c,"message_id":"choix","body":"Bonjour au groupe","mentions":[mention("a")]})),409);
    assert_eq!(f.bad("/api/send",json!({"conversation_id":c,"message_id":"absent","body":"Salut","mentions":[mention("grokbot")]})),409);
    assert!(!f.conversation(&c)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "absent"));
}

#[test]
fn everyone_marker_cannot_collide_with_a_real_bot_id() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("everyone");
    let c = f.group(&["a", "everyone"]);
    f.send(&c, "bot-precis", json!([mention("everyone")]));
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='bot-precis'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT bot_id FROM wrapper_group_turns WHERE root_id='bot-precis'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "everyone"
    );
    f.send(&c, "tous", json!([{"id":"@everyone","label":"everyone"}]));
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='tous'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
}

#[test]
fn everyone_snapshots_members_and_coordinator_is_editable() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    f.bot("c");
    let c = f.group(&["a", "b"]);
    f.post("/api/send",json!({"conversation_id":c,"message_id":"perdu","body":"Salut","mentions":[{"id":"@everyone","label":"everyone"}],"simulate_loss":true}));
    f.post(
        "/api/conversations/members/add",
        json!({"conversation_id":c,"bot_id":"c","command_id":"ajout-c"}),
    );
    f.post(
        "/api/reconcile",
        json!({"conversation_id":c,"message_id":"perdu"}),
    );
    f.drain();
    let n: i64 = f
        .db()
        .query_row(
            "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='perdu'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 2);
    f.post(
        "/api/conversations/coordinator",
        json!({"conversation_id":c,"bot_id":"b","command_id":"coord"}),
    );
    f.send(&c, "sans-mention", json!([]));
    f.drain();
    let bot: String = f
        .db()
        .query_row(
            "SELECT bot_id FROM wrapper_group_turns WHERE root_id='sans-mention'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bot, "b");
    assert_eq!(
        f.bad(
            "/api/conversations/coordinator",
            json!({"conversation_id":c,"bot_id":"a","command_id":"coord"})
        ),
        409
    );
}

#[test]
fn peer_delivery_answer_return_share_visible_history_not_private_instructions() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.script(call("avis-b", "b"));
    f.script(text("La demande est livrée."));
    f.script(text("Voici l’avis de B."));
    f.script(text("Je reprends avec son avis."));
    f.send(&c, "racine", json!([mention("a")]));
    f.drain();
    let messages = f.conversation(&c)["messages"].as_array().unwrap().clone();
    let replies = messages
        .iter()
        .filter(|m| m["payload"]["turn_id"].is_string())
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 3);
    assert_eq!(
        replies
            .iter()
            .map(|m| m["payload"]["bot_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "a"]
    );
    assert!(replies[1]["payload"]["parent_turn_id"].is_string());
    assert!(replies[2]["body"].as_str().unwrap().contains("reprends"));
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.bodies.len(), 4);
    for body in &seen.bodies {
        let instructions = body["instructions"].as_str().unwrap();
        assert_ne!(
            instructions.contains("secret-prive-a"),
            instructions.contains("secret-prive-b")
        );
        let input = body["input"].to_string();
        assert!(!input.contains("secret-prive-"));
    }
    assert!(seen.bodies[3]["input"].to_string().contains("avis de B"));
}

#[test]
fn interrupted_group_callback_is_not_silently_reissued_in_the_same_process() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.db().execute_batch("CREATE TRIGGER reject_group_delta BEFORE INSERT ON engine_run_events WHEN NEW.run_id LIKE 'group-%' AND NEW.kind='text_delta' BEGIN SELECT RAISE(FAIL,'injected group callback failure'); END;").unwrap();
    f.script(text("Un texte distant dont la sauvegarde échoue."));
    f.send(&c, "incertain-io", json!([mention("a")]));
    f.drain();
    let run: String = f
        .db()
        .query_row(
            "SELECT run_id FROM wrapper_group_turns WHERE root_id='incertain-io'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        store::required(&f.db(), &run).unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 1);
    f.db()
        .execute_batch("DROP TRIGGER reject_group_delta")
        .unwrap();
    f.app.tick(0);
    f.restart();
    f.app.tick(0);
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 1);
}

#[test]
fn refusal_model_failure_stop_removal_and_uncertain_restart_are_terminal() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.script(call("interdit", "grokbot"));
    f.script(text("Ce bot n’est pas membre."));
    f.send(&c, "refus", json!([mention("a")]));
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='refus'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(f.server.seen.lock().unwrap().bodies[1]["input"]
        .to_string()
        .contains("membre actif"));
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            500,
            json!({"error":{"message":"Échec modèle de test"}}),
        ));
    f.send(&c, "erreur", json!([mention("b")]));
    f.drain();
    assert!(f.conversation(&c)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["state"] == "failed"));
    // Persist an in-flight remote run without ever issuing a network request.
    let db = f.db();
    db.execute("INSERT INTO wrapper_group_turns(account_id,conversation_id,id,root_id,bot_id,cause_id,prompt,priority,status,run_id,snapshot) VALUES ('local-workspace',?1,'incertain','refus','a','crash','avis',1,'running','crash-run','[]')",[&c]).unwrap();
    let run = model::Run {
        run_id: "crash-run".into(),
        bot_id: "a".into(),
        conversation_id: c.clone(),
        trigger: Trigger::UserMessage {
            message_id: format!("{c}/incertain"),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Queued,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: 0,
    };
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    model::start(&tx, &run).unwrap();
    model::set_running_at(&tx, &run.run_id, 0).unwrap();
    tx.commit().unwrap();
    db.execute("INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES ('crash-run',?1,1,'gpt-test')",[json!({"entry_id":"incertain","role":"user","text":"avis","images":[],"tool_items":[]}).to_string()]).unwrap();
    grokbot_wrapper::features::conversations::append_entry(&db, &c, "group_reply", "incertain", 0)
        .unwrap();
    let before = f.server.seen.lock().unwrap().bodies.len();
    f.restart();
    f.app.tick(0);
    assert_eq!(
        store::required(&db, "crash-run").unwrap().status,
        RunStatus::Interrupted
    );
    assert_eq!(before, f.server.seen.lock().unwrap().bodies.len());
    assert_eq!(
        db.query_row(
            "SELECT status FROM wrapper_group_turns WHERE id='incertain'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "failed"
    );
    // A queued turn survives a restart, but stop/removal must prevent its launch.
    db.execute("INSERT INTO wrapper_group_turns(account_id,conversation_id,id,root_id,bot_id,cause_id,prompt,priority,status) VALUES ('local-workspace',?1,'en-file','refus','b','queued','avis',1,'queued')",[&c]).unwrap();
    f.restart();
    f.post(
        "/api/conversations/members/remove",
        json!({"conversation_id":c,"bot_id":"b","command_id":"retrait"}),
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM wrapper_group_turns WHERE id='en-file'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "cancelled"
    );
    f.post(
        "/api/stop",
        json!({"conversation_id":c,"message_id":"groupe"}),
    );
    assert_eq!(before, f.server.seen.lock().unwrap().bodies.len());
}

#[test]
fn group_approval_uses_the_existing_gate_in_the_displayed_thread_and_denial_stays_denied() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    let db = f.db();
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    let run = model::Run {
        run_id: "autorisation".into(),
        bot_id: "b".into(),
        conversation_id: c.clone(),
        trigger: Trigger::UserMessage {
            message_id: "question".into(),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Running,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: 0,
    };
    model::start(&tx, &run).unwrap();
    let call = ToolCall {
        call_id: "action-protegee".into(),
        name: "qa_write_file".into(),
        arguments: json!({"path":"/tmp/aucune-ecriture","content":"test"}),
    };
    let ctx = grokbot_wrapper::features::Context {
        connection: &tx,
        provider: &f.provider,
        db_path: &f.path,
        now_ms: f.app.feature_context().now_ms,
        bot_id: Some("b"),
        conversation_id: Some(&c),
        run_id: Some(&run.run_id),
    };
    grokbot_wrapper::features::forms::request_tool_approval(&ctx, &run, &call).unwrap();
    tx.commit().unwrap();
    let request = grokbot_forms::requests::list_for_run(&db, &run.run_id)
        .unwrap()
        .remove(0);
    assert_eq!(request.route.target_conversation_id, c);
    assert_eq!(request.route.requesting_bot_id, "b");
    f.post("/api/approvals/resolve",json!({"entry_id":request.entry_id,"submission_id":"refus","conversation_id":c,"decision":"deny"}));
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    let ctx = grokbot_wrapper::features::Context {
        connection: &tx,
        provider: &f.provider,
        db_path: &f.path,
        now_ms: f.app.feature_context().now_ms,
        bot_id: Some("b"),
        conversation_id: Some(&c),
        run_id: Some(&run.run_id),
    };
    assert!(matches!(
        grokbot_wrapper::features::forms::check_tool_approval(&ctx, &run, &call).unwrap(),
        ApprovalCheck::Denied { .. }
    ));
    tx.commit().unwrap();
}

#[test]
fn six_member_cap_and_human_priority_are_enforced_by_the_host() {
    let mut f = Fixture::new(false);
    for id in ["a", "b", "c", "d", "e", "f", "g"] {
        f.bot(id);
    }
    let c = f.group(&["a", "b", "c", "d", "e", "f"]);
    assert_eq!(
        f.bad(
            "/api/conversations/members/add",
            json!({"conversation_id":c,"bot_id":"g","command_id":"sept"})
        ),
        409
    );
    assert_eq!(
        f.bad(
            "/api/conversations",
            json!({"bot_ids":["a","b","c","d","e","f","g"]})
        ),
        400
    );
    f.send(&c, "origine", json!([]));
    f.drain();
    let db = f.db();
    db.execute("INSERT INTO wrapper_group_turns(account_id,conversation_id,id,root_id,bot_id,cause_id,prompt,priority,status) VALUES ('local-workspace',?1,'delegation-en-file','origine','b','delayed','Avis',1,'queued')",[&c]).unwrap();
    grokbot_wrapper::features::conversations::append_entry(
        &db,
        &c,
        "group_reply",
        "delegation-en-file",
        0,
    )
    .unwrap();
    f.send(&c, "nouveau-humain", json!([mention("c")]));
    f.drain();
    let mut s=db.prepare("SELECT t.root_id FROM wrapper_engine_runs w JOIN wrapper_group_turns t ON t.run_id=w.run_id WHERE t.id='delegation-en-file' OR t.root_id='nouveau-humain' ORDER BY w.rowid").unwrap();
    let order = s
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(order, ["nouveau-humain", "origine"]);
}

#[test]
fn group_form_from_non_coordinator_resumes_in_the_same_thread_after_restart() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.script(function_call("formulaire", "request_form", json!({"fields":[{"name":"avis","label":"Votre avis","field_type":{"kind":"text"},"required":true}]})));
    f.send(&c, "question", json!([mention("b")]));
    wait("formulaire du groupe", || {
        f.db()
            .query_row(
                "SELECT COUNT(*)=1 FROM forms_requests WHERE status='pending'",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
    });
    let run_id: String = f
        .db()
        .query_row(
            "SELECT run_id FROM wrapper_group_turns WHERE root_id='question'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let request = grokbot_forms::requests::list_for_run(&f.db(), &run_id)
        .unwrap()
        .remove(0);
    assert_eq!(request.route.target_conversation_id, c);
    assert_eq!(request.route.target_bot_id, "b");
    f.restart();
    f.script(text("Merci pour votre avis."));
    f.post("/api/forms/submit",json!({"entry_id":request.entry_id,"submission_id":"avis-1","conversation_id":c,"values":{"avis":"Continuons ensemble"}}));
    f.drain();
    assert_eq!(
        store::required(&f.db(), &run_id).unwrap().status,
        RunStatus::Finished
    );
    assert!(f
        .conversation(&c)
        .to_string()
        .contains("Merci pour votre avis"));
}

#[test]
fn shared_snapshots_forget_visible_facts_without_touching_peers_private_threads() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    let fact = "Une préférence collective très précise";
    f.script(function_call(
        "retenir",
        "remember",
        json!({"text":fact,"sensitive":false,"source":"model_inferred"}),
    ));
    f.script(text(fact));
    f.send(&c, "memoire", json!([mention("a")]));
    f.drain();
    f.script(text("J’ai lu le fil partagé."));
    f.send(&c, "pair", json!([mention("b")]));
    f.drain();
    assert!(f
        .db()
        .query_row(
            "SELECT snapshot FROM wrapper_group_turns WHERE root_id='pair'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap()
        .contains(fact));
    let private = f.group(&["b"]);
    f.script(text(fact));
    f.send(&private, "prive", json!([]));
    wait("réponse privée", || {
        f.conversation(&private)["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "assistant" && m["state"] == "echoed")
    });
    let id = grokbot_memory::facts::list_active(&f.db(), "a")
        .unwrap()
        .remove(0)
        .fact_id;
    f.post("/api/memory/forget", json!({"bot_id":"a","fact_id":id}));
    assert!(!f
        .db()
        .query_row(
            "SELECT snapshot FROM wrapper_group_turns WHERE root_id='pair'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap()
        .contains(fact));
    assert!(f.conversation(&private).to_string().contains(fact));
}

#[test]
fn context_limit_preserves_transcript_and_reset_starts_a_shorter_projection() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    for n in 0..9 {
        f.post("/api/send",json!({"conversation_id":c,"message_id":format!("long-{n}"),"body":"texte".repeat(3200)}));
        f.drain();
    }
    let conversation = f.conversation(&c);
    assert_eq!(
        conversation["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .count(),
        9
    );
    assert!(conversation.to_string().contains("128 Kio"));
    f.post("/api/conversations/reset", json!({"conversation_id":c}));
    f.send(&c, "apres-reset", json!([]));
    f.drain();
    let snapshot: String = f
        .db()
        .query_row(
            "SELECT snapshot FROM wrapper_group_turns WHERE root_id='apres-reset'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!snapshot.contains("textetexte"));
    assert_eq!(
        f.db()
            .query_row(
                "SELECT status FROM wrapper_group_turns WHERE root_id='apres-reset'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "completed"
    );
}

#[test]
fn queued_offline_turn_does_not_switch_to_a_remote_generation_after_signin() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.send(
        &c,
        "hors-ligne",
        json!([{"id":"@everyone","label":"everyone"}]),
    );
    std::fs::write(f.path.with_extension("chatgpt-auth.json"),json!({"host_id":"fake","client_id":"fake","access_token":"fake","refresh_token":"fake","expires_at":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
    f.provider.refresh_models();
    wait("connexion factice", || {
        f.provider.status()["models_ready"] == true
    });
    f.restart();
    f.drain();
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 0);
    assert_eq!(f.db().query_row("SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='hors-ligne' AND status='completed'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
}

#[test]
fn queued_turn_and_echo_admission_gap_recover_once_after_restart() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.send(&c, "durable", json!([mention("b")]));
    f.drain();
    // Recreate the crash window after echo, before admitting any recipient.
    let db = f.db();
    db.execute(
        "DELETE FROM wrapper_group_turns WHERE root_id='durable'",
        [],
    )
    .unwrap();
    db.execute(
        "DELETE FROM wrapper_messages WHERE conversation_id=?1 AND kind='group_reply'",
        [&c],
    )
    .unwrap();
    f.restart();
    f.drain();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='durable'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // A committed queued recipient remains the same logical turn across restart.
    db.execute("UPDATE wrapper_group_turns SET status='queued',run_id=NULL,snapshot=NULL WHERE root_id='durable'",[]).unwrap();
    f.restart();
    f.drain();
    f.restart();
    f.drain();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='durable'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn per_bot_and_total_chain_budgets_are_durable_and_visible() {
    let mut f = Fixture::new(false);
    for id in ["a", "b", "c", "d", "e"] {
        f.bot(id);
    }
    let c = f.group(&["a", "b", "c", "d", "e"]);
    f.send(&c, "limite", json!([mention("a")]));
    f.drain();
    // Admission is tested through the real model tool hook in its caller transaction.
    let db = f.db();
    db.execute(
        "UPDATE wrapper_group_turns SET status='running' WHERE root_id='limite'",
        [],
    )
    .unwrap();
    let run: String = db
        .query_row(
            "SELECT run_id FROM wrapper_group_turns WHERE root_id='limite'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    let ctx = grokbot_wrapper::features::Context {
        connection: &tx,
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("a"),
        conversation_id: Some(&c),
        run_id: Some(&run),
    };
    for (index, bot) in ["b", "b", "b", "b", "c", "c", "c", "d", "d", "d", "e", "e"]
        .iter()
        .enumerate()
    {
        let call = ToolCall {
            call_id: format!("budget-{index}"),
            name: "message_bot".into(),
            arguments: json!({"bot_id":bot,"text":"Avis"}),
        };
        let result = grokbot_wrapper::features::multibot::call_tool(&ctx, &call).unwrap();
        if index == 3 {
            assert!(matches!(result, ToolOutcome::Failed { .. }));
        }
    }
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='limite'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 12);
    let call = ToolCall {
        call_id: "treize".into(),
        name: "message_bot".into(),
        arguments: json!({"bot_id":"e","text":"Avis"}),
    };
    assert!(matches!(
        grokbot_wrapper::features::multibot::call_tool(&ctx, &call),
        Some(ToolOutcome::Failed { .. })
    ));
    tx.commit().unwrap();
    f.post(
        "/api/stop",
        json!({"conversation_id":c,"message_id":"groupe"}),
    );
    f.restart();
    let text = f.conversation(&c).to_string();
    assert!(text.contains("12 tours"));
    assert!(text.contains("3 tours par bot"));
}
