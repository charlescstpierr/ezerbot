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
        atomic::{AtomicBool, AtomicUsize, Ordering},
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
    // A display name cannot turn a membership event into a legacy limit card.
    f.post(
        "/api/bots/update",
        json!({"id":"b","name":"Limite de chaîne atteinte"}),
    );
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
    assert_eq!(
        &grouped["messages"].as_array().unwrap()[..2],
        old["messages"].as_array().unwrap()
    );
    assert_eq!(grouped["messages"].as_array().unwrap().len(), 3);
    assert_eq!(grouped["messages"][2]["kind"], "group_event");
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM wrapper_group_limits", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(grouped["messages"][2]["body"]
        .as_str()
        .unwrap()
        .contains("a rejoint le fil"));
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
fn interrupted_group_tool_call_is_recorded_without_another_model_call() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let conversation = f.group(&["a", "b"]);
    let db = f.db();
    with_tx(&db, |conn| -> grokbot_engine::contracts::Result<()> {
        let run = model::Run {
            run_id: "groupe-interrompu".into(),
            bot_id: "a".into(),
            conversation_id: conversation,
            trigger: Trigger::UserMessage {
                message_id: "message-groupe".into(),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Queued,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 1,
        };
        model::start(conn, &run)?;
        model::set_running_at(conn, &run.run_id, 2)?;
        store::append_event(
            conn,
            &run.run_id,
            RunEventKind::ToolCall,
            &json!({"call_id":"appel","name":"remember","arguments":{}}),
        )?;
        let mut current = store::required(conn, &run.run_id)?;
        current.status = RunStatus::Interrupted;
        store::save(conn, &current)?;
        store::append_event(
            conn,
            &run.run_id,
            RunEventKind::Interrupted,
            &json!({"reason":"process_restart"}),
        )?;
        Ok(())
    })
    .unwrap();
    drop(db);
    let before = f.server.seen.lock().unwrap().bodies.len();
    f.app.tick(0);
    let db = f.db();
    assert_eq!(
        store::required(&db, "groupe-interrompu").unwrap().status,
        RunStatus::Interrupted
    );
    let events = store::events(&db, "groupe-interrompu", 0).unwrap();
    assert!(events.iter().any(|event| {
        event.kind == RunEventKind::ToolResult && event.payload["call_id"] == "appel"
    }));
    let replay = grokbot_engine::run_loop::replay_items(&events).unwrap();
    assert!(serde_json::to_string(&replay).unwrap().contains("appel"));
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), before);
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
        if index == 2 {
            // Merely reserving a bot's third slot is not a refused delivery.
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM wrapper_group_limits", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            tx.query_row("SELECT COUNT(*) FROM wrapper_group_notices", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
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
    let current = f.conversation(&c);
    let terminal = current["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(terminal["kind"], "group_notice");
    assert_eq!(terminal["payload"]["terminal"], true);
    assert_eq!(
        current["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["kind"] == "group_notice")
            .count(),
        1
    );
    assert!(db.query_row("SELECT (SELECT ordinal FROM wrapper_messages WHERE message_id=?1) > (SELECT MAX(m.ordinal) FROM wrapper_messages m JOIN wrapper_group_turns t ON t.id=m.message_id WHERE t.root_id='limite' AND m.kind='group_reply')",[terminal["id"].as_str().unwrap()],|r|r.get::<_,bool>(0)).unwrap());
    // Upgrade a prototype database containing two early limit cards. Keep their
    // IDs durably, but project only one corrected card after the final reply.
    let content_ids = current["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] != "group_notice")
        .map(|m| m["id"].clone())
        .collect::<Vec<_>>();
    db.execute("DELETE FROM wrapper_messages WHERE kind='group_notice'", [])
        .unwrap();
    db.execute("DELETE FROM wrapper_group_notices", []).unwrap();
    db.execute("DELETE FROM wrapper_group_limits", []).unwrap();
    db.execute("UPDATE wrapper_messages SET ordinal=ordinal*10+10000", [])
        .unwrap();
    for (id, body, ordinal) in [
        (
            "prototype-a",
            "Limite de chaîne atteinte : 3 tours par bot.",
            10025,
        ),
        (
            "prototype-b",
            "Limite de chaîne atteinte : 12 tours.",
            10035,
        ),
    ] {
        db.execute(
            "INSERT INTO wrapper_group_notices VALUES (?1,?2,'limite',?3)",
            rusqlite::params![id, c, body],
        )
        .unwrap();
        grokbot_wrapper::features::conversations::append_entry(&db, &c, "group_notice", id, 0)
            .unwrap();
        db.execute(
            "UPDATE wrapper_messages SET ordinal=?2 WHERE message_id=?1",
            rusqlite::params![id, ordinal],
        )
        .unwrap();
    }
    f.restart();
    f.drain();
    let migrated = f.conversation(&c);
    let messages = migrated["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["kind"] != "group_notice")
            .map(|m| m["id"].clone())
            .collect::<Vec<_>>(),
        content_ids
    );
    assert_eq!(messages.last().unwrap()["id"], "prototype-a");
    assert_eq!(
        messages
            .iter()
            .filter(|m| m["kind"] == "group_notice")
            .count(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM wrapper_group_notices", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    f.restart();
    f.drain();
    assert_eq!(
        f.conversation(&c)["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["id"],
        "prototype-a"
    );
}

#[test]
fn connected_queued_turn_waits_for_delayed_catalogue_after_restart() {
    restart_with_delayed_catalogue(false);
}

#[test]
fn connected_already_created_run_waits_for_delayed_catalogue_after_restart() {
    restart_with_delayed_catalogue(true);
}

fn restart_with_delayed_catalogue(already_created: bool) {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    let response_gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().response_gate = Some((1, response_gate.clone()));
    f.send(&c, "ancienne-requete", json!([mention("a")]));
    wait("requête distante en vol", || {
        f.server.seen.lock().unwrap().bodies.len() == 1
    });
    f.send(&c, "reprise-unique", json!([mention("b")]));
    assert_eq!(
        f.db()
            .query_row(
                "SELECT status FROM wrapper_group_turns WHERE root_id='reprise-unique'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "queued"
    );
    // A consistent crash image avoids keeping the old provider's worker alive
    // on the restarted database (which would not happen after a real process kill).
    let recovered = f.path.with_file_name("recovered.sqlite");
    f.db()
        .execute("VACUUM INTO ?1", [recovered.to_str().unwrap()])
        .unwrap();
    std::fs::copy(
        f.path.with_extension("chatgpt-auth.json"),
        recovered.with_extension("chatgpt-auth.json"),
    )
    .unwrap();
    f.post(
        "/api/stop",
        json!({"conversation_id":c,"message_id":"groupe"}),
    );
    response_gate.store(true, Ordering::SeqCst);
    if already_created {
        let mut db = Connection::open(&recovered).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute(
            "UPDATE engine_runs SET status='interrupted' WHERE status='running'",
            [],
        )
        .unwrap();
        tx.execute(
            "UPDATE wrapper_group_turns SET status='failed' WHERE root_id='ancienne-requete'",
            [],
        )
        .unwrap();
        let (turn, prompt): (String, String) = tx
            .query_row(
                "SELECT id,prompt FROM wrapper_group_turns WHERE root_id='reprise-unique'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let run = model::Run {
            run_id: format!("group-{turn}"),
            bot_id: "b".into(),
            conversation_id: c.clone(),
            trigger: Trigger::UserMessage {
                message_id: format!("{c}/{turn}"),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Queued,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 0,
        };
        model::start(&tx, &run).unwrap();
        let input = HistoryItem {
            entry_id: turn.clone(),
            role: Role::User,
            text: prompt,
            images: vec![],
            tool_items: vec![],
        };
        tx.execute(
            "INSERT INTO wrapper_engine_runs(run_id,input_json,live,model) VALUES (?1,?2,1,'')",
            rusqlite::params![run.run_id, serde_json::to_string(&input).unwrap()],
        )
        .unwrap();
        tx.execute(
            "UPDATE wrapper_group_turns SET status='running',run_id=?2,snapshot='[]' WHERE id=?1",
            rusqlite::params![turn, run.run_id],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    let models_gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().models_gate = Some(models_gate.clone());
    f.path = recovered;
    f.provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, f.provider.clone()).unwrap();
    assert_eq!(f.provider.status()["models_ready"], false);
    wait("catalogue retenu", || {
        f.server.seen.lock().unwrap().model_requests == 2
    });
    for _ in 0..3 {
        f.app.tick(0);
    }
    let pending = f.conversation(&c)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["payload"]["root_id"] == "reprise-unique")
        .unwrap()
        .clone();
    assert_eq!(pending["payload"]["turn_status"], "queued");
    assert_ne!(pending["state"], "failed");
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 1);
    models_gate.store(true, Ordering::SeqCst);
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT status FROM wrapper_group_turns WHERE root_id='reprise-unique'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "completed"
    );
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(
        seen.bodies
            .iter()
            .filter(|b| b.to_string().contains("reprise-unique"))
            .count(),
        1
    );
    assert_eq!(seen.bodies.len(), 2); // uncertain old request was never replayed
}

#[test]
fn refused_limit_is_published_once_after_all_real_calls_and_replies() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    f.post("/api/bots/update", json!({"id":"a","name":"Aurore"}));
    f.post("/api/bots/update", json!({"id":"b","name":"Boréal"}));
    let c = f.group(&["a", "b"]);
    for (index, bot) in ["b", "a", "b", "b", "a", "a"].into_iter().enumerate() {
        f.script(call(&format!("chaine-{index}"), bot));
        f.script(text("Tour terminé."));
    }
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().response_gate = Some((8, gate.clone()));
    f.send(&c, "chaine", json!([mention("a")]));
    wait("refus avec deux tours encore en attente", || {
        f.app.tick(0);
        f.server.seen.lock().unwrap().bodies.len() == 8
    });
    assert!(f
        .db()
        .query_row(
            "SELECT COUNT(*)>0 FROM wrapper_group_limits WHERE root_id='chaine' AND published=0",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert!(f
        .db()
        .query_row(
            "SELECT COUNT(*)>0 FROM wrapper_group_turns WHERE root_id='chaine' AND status='queued'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert!(!f.conversation(&c)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["kind"] == "group_notice"));
    gate.store(true, Ordering::SeqCst);
    f.drain();
    let current = f.conversation(&c);
    let terminal = current["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(terminal["kind"], "group_notice");
    assert!(terminal["body"]
        .as_str()
        .unwrap()
        .contains("Aurore, Boréal"));
    assert!(f.db().query_row("SELECT (SELECT ordinal FROM wrapper_messages WHERE message_id=?1) > (SELECT MAX(ordinal) FROM wrapper_messages WHERE conversation_id=?2 AND kind<>'group_notice')",rusqlite::params![terminal["id"].as_str().unwrap(),c],|r|r.get::<_,bool>(0)).unwrap());
    f.restart();
    f.drain();
    assert_eq!(
        f.conversation(&c)["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["kind"] == "group_notice")
            .count(),
        1
    );
    assert_eq!(f.server.seen.lock().unwrap().bodies.len(), 12);
}

#[test]
fn deleting_members_excludes_them_from_everyone_and_replaces_the_coordinator() {
    let mut f = Fixture::new(false);
    for bot in ["a", "b", "c"] {
        f.bot(bot);
    }
    let c = f.group(&["a", "b", "c"]);
    f.send(&c, "historique", json!([mention("a")]));
    f.drain();
    f.post("/api/bots/delete", json!({"id":"c"}));
    f.post("/api/bots/delete", json!({"id":"c"}));
    f.send(
        &c,
        "tous-vivants",
        json!([{"id":"@everyone","label":"everyone"}]),
    );
    f.drain();
    assert_eq!(f.db().query_row("SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='tous-vivants' AND bot_id='c'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    f.post("/api/bots/delete", json!({"id":"a"}));
    f.restart();
    assert_eq!(f.conversation(&c)["bot_id"], "b");
    assert_eq!(f.conversation(&c)["member_ids"], json!(["b"]));
    assert!(f.conversation(&c)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["payload"]["bot_id"] == "a"));
    f.send(&c, "repli", json!([]));
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT bot_id FROM wrapper_group_turns WHERE root_id='repli'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "b"
    );
    f.post("/api/bots/delete", json!({"id":"b"}));
    assert_eq!(f.conversation(&c)["member_ids"], json!([]));
    assert_eq!(
        f.bad(
            "/api/send",
            json!({"conversation_id":c,"message_id":"sans-membre","body":"Bonjour"})
        ),
        409
    );
    f.post(
        "/api/conversations/members/add",
        json!({"conversation_id":c,"bot_id":"grokbot","command_id":"remplacer"}),
    );
    assert_eq!(f.conversation(&c)["bot_id"], "grokbot");
    f.send(&c, "reprise", json!([]));
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT bot_id FROM wrapper_group_turns WHERE root_id='reprise'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "grokbot"
    );
}

#[test]
fn migration_retires_previously_deleted_group_coordinators() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    f.db()
        .execute("UPDATE wrapper_bots SET deleted=1 WHERE id='a'", [])
        .unwrap();
    f.restart();
    assert_eq!(f.conversation(&c)["member_ids"], json!(["b"]));
    assert_eq!(f.conversation(&c)["bot_id"], "b");
    f.send(
        &c,
        "migration",
        json!([{"id":"@everyone","label":"everyone"}]),
    );
    f.drain();
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='migration' AND bot_id='a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn mention_offsets_are_validated_and_survive_reload_without_name_matching() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    let c = f.group(&["a", "b"]);
    let body = "😀 @Même nom tapé, puis @Même nom choisi";
    let offset = body[..body.rfind("@Même nom").unwrap()]
        .encode_utf16()
        .count();
    let mention = json!({"id":"b","label":"Même nom","offset":offset});
    let input = json!({"conversation_id":c,"message_id":"puce","body":body,"mentions":[mention]});
    f.post("/api/send", input.clone());
    f.drain();
    f.restart();
    f.post("/api/send", input);
    f.drain();
    let current = f.conversation(&c);
    assert_eq!(current["messages"][0]["body"], body);
    assert_eq!(
        current["messages"][0]["payload"]["mentions"][0]["offset"],
        offset
    );
    assert_eq!(
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE root_id='puce'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    for mentions in [
        json!([{"id":"a","label":"Même nom","offset":1}]),
        json!([mention.clone(), mention]),
    ] {
        assert_eq!(f.bad("/api/send",json!({"conversation_id":c,"message_id":"position-invalide","body":body,"mentions":mentions})),400);
    }
}

#[test]
fn first_completed_group_turn_generates_one_title_from_shared_visible_context() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.post("/api/conversations", json!({"bot_ids":["a","b"]}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.script(text("Bonjour au groupe."));
    f.script(text("Conversation de groupe"));
    f.send(&c, "titre", json!([mention("a")]));
    wait("titre du groupe", || {
        f.app.tick(0);
        f.conversation(&c)["title"] == "Conversation de groupe"
    });
    f.drain();
    f.send(&c, "suite", json!([mention("b")]));
    f.drain();
    wait("fin de titrage", || f.provider.status()["titling"] == false);
    let seen = f.server.seen.lock().unwrap();
    let titles = seen
        .bodies
        .iter()
        .filter(|b| {
            b["instructions"]
                .as_str()
                .unwrap_or_default()
                .contains("titre")
        })
        .collect::<Vec<_>>();
    assert_eq!(titles.len(), 1);
    assert!(titles[0].to_string().contains("Bonjour au groupe"));
    assert!(!titles[0].to_string().contains("secret-prive"));
}

#[test]
fn group_documents_remain_shared_once_without_private_files_after_restart() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let private = f.group(&["a"]);
    let private_doc = f.post(
        "/api/documents",
        json!({"name":"prive.txt","data":STANDARD.encode("DOCUMENT_PRIVE_HORS_GROUPE")}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.post("/api/send",json!({"conversation_id":private,"message_id":"prive","body":"Garde ce fichier dans ce fil.","attachments":[private_doc]}));
    wait("réponse privée terminée", || {
        f.conversation(&private)["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["state"]
            == "echoed"
    });
    let before = f.server.seen.lock().unwrap().bodies.len();
    let c = f.group(&["a", "b"]);
    let document = f.post(
        "/api/documents",
        json!({"name":"notes-partagees.txt","data":STANDARD.encode("DOCUMENT_PARTAGE_DU_GROUPE")}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.script(call("lecture-pair", "b"));
    f.script(text("Demande livrée en file."));
    f.script(text("Document lu par le pair."));
    f.script(text("Avis du pair reçu."));
    f.post("/api/send",json!({"conversation_id":c,"message_id":"document-groupe","body":"Lis les notes puis demande un avis.","mentions":[mention("a")],"attachments":[document]}));
    f.drain();
    f.restart();
    f.script(text("Notes relues après redémarrage."));
    f.send(&c, "relecture", json!([mention("b")]));
    f.drain();
    let seen = f.server.seen.lock().unwrap();
    assert_eq!(seen.bodies.len() - before, 5);
    for request in &seen.bodies[before..] {
        let input = request["input"].to_string();
        assert_eq!(input.matches("DOCUMENT_PARTAGE_DU_GROUPE").count(), 1);
        assert!(!input.contains("DOCUMENT_PRIVE_HORS_GROUPE"));
        assert!(!request["instructions"]
            .as_str()
            .unwrap()
            .contains("DOCUMENT_PARTAGE_DU_GROUPE"));
    }
}

#[test]
fn a_failed_initial_group_turn_does_not_prevent_the_first_completed_title() {
    let mut f = Fixture::new(true);
    f.bot("a");
    f.bot("b");
    let c = f.post("/api/conversations", json!({"bot_ids":["a","b"]}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            500,
            json!({"error":{"message":"Indisponible"}}),
        ));
    f.send(&c, "echec-initial", json!([mention("a")]));
    f.drain();
    f.script(text("La discussion peut reprendre."));
    f.script(text("Un nouveau départ"));
    f.send(&c, "premier-succes", json!([mention("b")]));
    wait("premier titre après échec", || {
        f.app.tick(0);
        f.conversation(&c)["title"] == "Un nouveau départ"
    });
    f.drain();
    wait("fin du titrage", || f.provider.status()["titling"] == false);
}

#[test]
fn group_profile_is_saved_on_create_and_rename() {
    let mut f = Fixture::new(false);
    f.bot("a");
    f.bot("b");
    assert_eq!(
        f.bad(
            "/api/conversations",
            json!({"bot_ids":["a"],"title":"Seul","description":"Pas un groupe"})
        ),
        400
    );
    assert_eq!(
        f.bad(
            "/api/conversations",
            json!({"bot_ids":["a","b","c","d","e","f","g"],"title":"Trop","description":"Sept bots"})
        ),
        400
    );
    let created = f.post(
        "/api/conversations",
        json!({"bot_ids":["a","b"],"title":"Atelier","description":"Croiser les lectures","avatar":{"color":"cyan","mark":"A"}}),
    );
    let id = created["conversation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(created["conversation"]["kind"], "group");
    assert_eq!(created["conversation"]["title"], "Atelier");
    assert_eq!(
        created["conversation"]["description"],
        "Croiser les lectures"
    );
    assert_eq!(created["conversation"]["avatar"]["color"], "cyan");
    assert_eq!(created["conversation"]["avatar"]["mark"], "A");
    assert_eq!(created["conversation"]["member_ids"], json!(["a", "b"]));
    let renamed = f.post(
        "/api/conversations/update",
        json!({"id": id, "title": "Atelier bis", "description": "Nouvelle description", "avatar": {"color": "violet", "mark": "B"}}),
    );
    assert_eq!(renamed["conversation"]["title"], "Atelier bis");
    assert_eq!(
        renamed["conversation"]["description"],
        "Nouvelle description"
    );
    assert_eq!(
        renamed["conversation"]["avatar"],
        json!({"color": "violet", "mark": "B"})
    );
    f.post(
        "/api/conversations/update",
        json!({"id": id, "favorite": true}),
    );
    f.restart();
    let stored = f.conversation(&id);
    assert_eq!(stored["title"], "Atelier bis");
    assert_eq!(stored["description"], "Nouvelle description");
    assert_eq!(stored["avatar"]["mark"], "B");
    assert_eq!(stored["favorite"], true);
    let direct = f.post(
        "/api/conversations",
        json!({"bot_ids":["a"],"title":"Direct"}),
    );
    let direct_id = direct["conversation"]["id"].as_str().unwrap();
    assert_eq!(direct["conversation"]["description"], "");
    assert!(direct["conversation"].get("avatar").is_none());
    assert_eq!(
        f.bad(
            "/api/conversations/update",
            json!({"id": direct_id, "description": "Interdit"})
        ),
        400
    );
}
