//! Real Responses boundary, durable queue/cards/settings, no account or app data.
mod support;
use grokbot_memory::facts::{self, FactStatus, Scope, Source};
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
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};

struct Fixture {
    app: App,
    server: FakeOpenAi,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self::with_small_model(true)
    }
    fn with_small_model(memory_model: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "memory-auto-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":"oaiapp_test","access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen {
            memory_model,
            ..Seen::default()
        })));
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("catalogue", || provider.status()["models_ready"] == true);
        Self { app, server, path }
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn post(&mut self, p: &str, v: Value) -> Value {
        self.app.api("POST", p, v.to_string().as_bytes()).unwrap()
    }
    fn state(&mut self) -> Value {
        self.app.tick(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        );
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn conv(&mut self, bot: &str) -> String {
        self.post(
            "/api/conversations",
            json!({"title":"Test mémoire","bot_id":bot}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn script(&self, facts: Value) {
        self.script_response(response(facts));
    }
    fn script_response(&self, r: ScriptedResponse) {
        self.server
            .seen
            .lock()
            .unwrap()
            .memory_responses
            .push_back(r);
    }
    fn send(&mut self, c: &str, id: &str, text: &str) {
        self.post(
            "/api/send",
            json!({"conversation_id":c,"message_id":id,"body":text}),
        );
        wait("réponse", || {
            self.messages(c)
                .iter()
                .any(|m| m["id"] == format!("reply-{id}") && m["state"] == "echoed")
        });
    }
    fn settle(&mut self) {
        wait("extraction terminée", || {
            self.state()["memory_auto"]["pending"] == false
        });
    }
    fn messages(&mut self, c: &str) -> Vec<Value> {
        self.state()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == c)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn cards(&mut self, c: &str) -> Vec<Value> {
        self.messages(c)
            .into_iter()
            .filter(|m| m["kind"] == "memory_auto")
            .collect()
    }
    fn facts(&self, bot: &str) -> Vec<facts::Fact> {
        facts::list_active(&self.db(), bot).unwrap()
    }
    fn calls(&self) -> Vec<Value> {
        self.server
            .seen
            .lock()
            .unwrap()
            .bodies
            .iter()
            .filter(|b| {
                b["instructions"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("Extraction de mémoire automatique.")
            })
            .cloned()
            .collect()
    }
    fn reopen(&mut self) {
        self.settle();
        let provider = Provider::with_endpoints(&self.path, self.server.endpoints(), false);
        self.app = App::open_with(&self.path, provider.clone()).unwrap();
        wait("catalogue après reprise", || {
            provider.status()["models_ready"] == true
        });
    }
    fn bot(&mut self) {
        self.post(
            "/api/bots",
            json!({"id":"other","name":"Autre","color":"cyan","description":"","instructions":""}),
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn fact(text: &str, replace: Option<&str>) -> Value {
    json!({"text":text,"category":"preference","replace_id":replace})
}
fn response(facts: Value) -> ScriptedResponse {
    ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":json!({"facts":facts}).to_string()}),
        json!({"type":"response.completed"}),
    ])
}

#[test]
fn finished_turn_extracts_bounded_private_facts_and_persistent_cards() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let texts = [
        "Je préfère le thé vert.",
        "Je suis développeur.",
        "Je travaille sur Ezerbot.",
        "Je préfère les réponses courtes.",
    ];
    f.script(json!(texts
        .iter()
        .map(|s| fact(s, None))
        .collect::<Vec<_>>()));
    f.send(&c, "one", &texts.join(" "));
    f.settle();
    let facts = f.facts("grokbot");
    assert_eq!(facts.len(), 3);
    for fact in facts {
        assert_eq!(fact.scope, Scope::Private);
        assert_eq!(fact.provenance.source, Source::Automatic);
        assert_eq!(fact.provenance.entry_id.as_deref(), Some("one"));
    }
    assert_eq!(f.cards(&c).len(), 3);
    let request = f.calls().pop().unwrap();
    assert_eq!(request["max_output_tokens"], 900);
    assert_eq!(request["model"], "gpt-test-mini");
    assert!(request.get("tools").is_none());
    assert!(request.get("service_tier").is_none());
    assert_eq!(request["input"].as_array().unwrap().len(), 1);
    assert!(!request["input"].to_string().contains("Bonjour"));
    f.reopen();
    assert_eq!(f.cards(&c).len(), 3);
    assert_eq!(f.calls().len(), 1);
}

#[test]
fn deduplicates_updates_preserves_team_scope_and_undo_restores_previous() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let original = "Je préfère le thé vert.";
    f.script(json!([fact(original, None)]));
    f.send(&c, "one", original);
    f.settle();
    let old = f.facts("grokbot")[0].clone();
    f.script(json!([fact(original, None)]));
    f.send(&c, "two", original);
    f.settle();
    assert_eq!(f.cards(&c).len(), 1);
    f.post(
        "/api/memory/promote",
        json!({"bot_id":"grokbot","fact_ids":[old.fact_id]}),
    );
    let team = f
        .facts("grokbot")
        .into_iter()
        .find(|v| v.scope == Scope::Team)
        .unwrap();
    let corrected = "Je préfère maintenant le café.";
    f.script(json!([fact(corrected, Some(&team.fact_id))]));
    f.send(&c, "three", corrected);
    f.settle();
    let new = f
        .facts("grokbot")
        .into_iter()
        .find(|v| v.text == corrected)
        .unwrap();
    assert_eq!(new.scope, Scope::Team);
    let card = f.cards(&c).pop().unwrap();
    assert_eq!(card["payload"]["replaced"], true);
    f.post("/api/memory/automatic/undo", json!({"card_id":card["id"]}));
    let restored = f
        .facts("grokbot")
        .into_iter()
        .find(|v| v.text == original)
        .unwrap();
    assert_eq!(restored.provenance, old.provenance);
    assert_eq!(restored.learned_at, old.learned_at);
    assert_eq!(f.cards(&c)[0]["payload"]["can_undo"], true);
    assert!(!f.facts("grokbot").iter().any(|v| v.text == corrected));
    assert!(f
        .facts("grokbot")
        .iter()
        .any(|v| v.text == original && v.scope == Scope::Team));
    assert_eq!(f.cards(&c).last().unwrap()["payload"]["cancelled"], true);
}

#[test]
fn undo_new_fact_and_forget_block_identical_source_after_restart() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère les explications détaillées.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    let card = f.cards(&c).pop().unwrap();
    f.post("/api/memory/automatic/undo", json!({"card_id":card["id"]}));
    assert!(f.facts("grokbot").is_empty());
    assert_eq!(f.cards(&c)[0]["payload"]["text"], "");
    f.reopen();
    f.send(&c, "two", text.trim_end_matches('.'));
    f.settle();
    assert_eq!(f.calls().len(), 1);
    assert!(f.facts("grokbot").is_empty());
    let other = "Je travaille sur un projet de jardin.";
    f.script(json!([fact(other, None)]));
    f.send(&c, "three", other);
    f.settle();
    let id = f.facts("grokbot")[0].fact_id.clone();
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":id}),
    );
    f.reopen();
    f.send(&c, "four", other);
    f.settle();
    assert_eq!(f.calls().len(), 2);
    assert!(f.facts("grokbot").is_empty());
}

#[test]
fn off_switch_is_per_bot_persisted_and_inflight_disable_discards_results() {
    let mut f = Fixture::new();
    f.bot();
    let c = f.conv("grokbot");
    let other = f.conv("other");
    f.post(
        "/api/memory/automatic",
        json!({"bot_id":"grokbot","enabled":false}),
    );
    f.reopen();
    assert_eq!(f.state()["memory_auto"]["settings"][0]["enabled"], false);
    f.send(&c, "one", "Je préfère le thé.");
    f.settle();
    assert!(f.calls().is_empty());
    let text = "Je préfère le café.";
    f.server.seen.lock().unwrap().memory_delay = Duration::from_millis(500);
    f.script_response(response(json!([fact(text, None)])));
    f.send(&other, "two", text);
    wait("appel en vol", || {
        f.state();
        f.calls().len() == 1
    });
    // The completed reply is already visible while the extractor is waiting.
    assert!(f
        .messages(&other)
        .iter()
        .any(|m| m["id"] == "reply-two" && m["state"] == "echoed"));
    f.post(
        "/api/memory/automatic",
        json!({"bot_id":"other","enabled":false}),
    );
    f.settle();
    assert!(f.facts("other").is_empty());
}

#[test]
fn provider_errors_refusal_malformed_or_ungrounded_facts_are_silent() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let responses = [
        ScriptedResponse::json(500, json!({"error":"bad"})),
        ScriptedResponse::events(&[
            json!({"type":"response.refusal.delta","delta":"non"}),
            json!({"type":"response.completed"}),
        ]),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"not json"}),
            json!({"type":"response.completed"}),
        ]),
        response(
            json!([fact("Je préfère une invention.", None), {"text":"Je préfère le café.","category":"transient","replace_id":null}, fact("Je préfère le café.",Some("fact_00000000000000000000000000000000"))]),
        ),
        ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"x".repeat(4097)}),
            json!({"type":"response.completed"}),
        ]),
    ];
    for (i, r) in responses.into_iter().enumerate() {
        f.script_response(r);
        f.send(&c, &format!("m{i}"), "Je préfère le café.");
        f.settle();
    }
    assert!(f.facts("grokbot").is_empty());
    assert!(f.cards(&c).is_empty());
    for message in f.messages(&c) {
        assert!(message["error"].is_null());
    }
    f.reopen();
    assert_eq!(f.calls().len(), 5); // no retry of failed/claimed calls
}

#[test]
fn rejects_secret_form_transient_and_over_budget_turns_before_model_call() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    f.db().execute("INSERT INTO forms_resolutions(resolution_id,entry_id,submission_id,values_json) VALUES ('resolution','form','submit',?1)",[json!({"city":"Fausseville"}).to_string()]).unwrap();
    let texts = [
        "Mon secret est secret-factice.",
        "Je préfère utiliser le jeton aaa.",
        "Je travaille à Fausseville.",
        "Je préfère mon exemple de code ```rust```.",
        "Je préfère acheter du pain demain.",
    ];
    for (i, text) in texts.iter().enumerate() {
        f.send(&c, &format!("m{i}"), text);
        f.settle();
    }
    f.send(&c, "large", &"é".repeat(4001));
    f.settle();
    assert!(f.calls().is_empty());
    assert!(f.facts("grokbot").is_empty());
}

#[test]
fn late_extraction_cannot_revive_forgotten_fact_and_cannot_update_other_owner() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    let old = f.facts("grokbot")[0].clone();
    let changed = "Je préfère le café noir.";
    f.server.seen.lock().unwrap().memory_delay = Duration::from_millis(500);
    f.script_response(response(json!([fact(changed, Some(&old.fact_id))])));
    f.send(&c, "two", changed);
    wait("appel en vol", || {
        f.state();
        f.calls().len() == 2
    });
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":old.fact_id}),
    );
    f.settle();
    assert!(f.facts("grokbot").is_empty());
    assert_eq!(
        facts::get(&f.db(), &old.fact_id).unwrap().unwrap().status,
        FactStatus::Forgotten
    );
    // A shared fact owned by another bot may be seen but never corrected.
    f.bot();
    let other = f.conv("other");
    f.script(json!([fact(text, None)]));
    f.send(&other, "three", text);
    f.settle();
    let owned = f.facts("other")[0].clone();
    f.post(
        "/api/memory/promote",
        json!({"bot_id":"other","fact_ids":[owned.fact_id]}),
    );
    let team = f.facts("grokbot").pop().unwrap();
    f.script(json!([fact(changed, Some(&team.fact_id))]));
    f.send(&c, "four", changed);
    f.settle();
    assert_eq!(f.facts("grokbot")[0].text, text);
}

#[test]
fn manual_card_edit_uses_correction_and_forgotten_lineage_hides_all_card_text() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    let old = f.facts("grokbot")[0].clone();
    let edited = f.post(
        "/api/memory/correct",
        json!({"bot_id":"grokbot","fact_id":old.fact_id,"text":"Je préfère le café."}),
    );
    assert_eq!(f.cards(&c)[0]["payload"]["text"], "Je préfère le café.");
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":edited["fact"]["fact_id"]}),
    );
    assert_eq!(f.cards(&c)[0]["payload"]["text"], "");
    f.reopen();
    assert_eq!(f.cards(&c)[0]["payload"]["active"], false);
}

#[test]
fn edited_receipt_can_still_undo_and_forgets_the_current_lineage_head() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    let card = f.cards(&c).pop().unwrap();
    f.post("/api/memory/correct",json!({"bot_id":"grokbot","fact_id":card["payload"]["fact_id"],"text":"Je préfère le thé avec du miel."}));
    assert_eq!(f.cards(&c)[0]["payload"]["can_undo"], true);
    f.post("/api/memory/automatic/undo", json!({"card_id":card["id"]}));
    assert!(f.facts("grokbot").is_empty());
    assert_eq!(f.cards(&c)[0]["payload"]["text"], "");
    f.reopen();
    assert_eq!(f.cards(&c)[0]["payload"]["active"], false);
}

#[test]
fn successful_retry_extracts_once_per_human_message_across_further_regenerations() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            400,
            json!({"error":{"code":"invalid_request_error","message":"Échec de test"}}),
        ));
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"retry","body":text}),
    );
    wait("premier tour en échec", || {
        f.messages(&c)
            .iter()
            .any(|m| m["id"] == "reply-retry" && m["state"] == "failed")
    });
    assert!(f.calls().is_empty());
    f.script(json!([fact(text, None)]));
    f.post(
        "/api/regenerate",
        json!({"conversation_id":c,"message_id":"retry"}),
    );
    wait("mémoire après relance", || {
        f.state();
        f.facts("grokbot").len() == 1
    });
    f.settle();
    assert_eq!(f.calls().len(), 1);
    assert_eq!(f.cards(&c).len(), 1);
    f.reopen();
    f.post(
        "/api/regenerate",
        json!({"conversation_id":c,"message_id":"retry"}),
    );
    wait("nouvelle régénération terminée", || {
        f.state();
        f.db().query_row("SELECT r.status FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')='retry' ORDER BY r.rowid DESC LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap()=="finished"
    });
    f.settle();
    assert_eq!(f.calls().len(), 1);
    assert_eq!(f.cards(&c).len(), 1);
}

#[test]
fn disconnected_queue_never_requests_perpetual_ui_polling() {
    let mut f = Fixture::new();
    f.post("/api/provider/signout", json!({}));
    f.db()
        .execute(
            "INSERT INTO memory_auto_jobs VALUES ('offline','queued',1)",
            [],
        )
        .unwrap();
    assert_eq!(f.state()["memory_auto"]["pending"], false);
    assert_eq!(f.state()["memory_auto"]["available"], false);
}

#[test]
fn cancelled_form_and_failed_turn_never_feed_extraction() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    f.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[
        json!({"type":"response.output_item.done","item":{"id":"fc-form","type":"function_call","status":"completed","call_id":"form","name":"request_form","arguments":json!({"fields":[{"name":"city","label":"Ville","field_type":{"kind":"text"},"required":true}]}).to_string()}}),
        json!({"type":"response.completed"}),
    ]));
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"form-turn","body":"Je préfère le thé vert."}),
    );
    wait("formulaire", || {
        f.messages(&c).iter().any(|m| m["kind"] == "form")
    });
    let card = f
        .messages(&c)
        .into_iter()
        .find(|m| m["kind"] == "form")
        .unwrap();
    f.post("/api/forms/cancel",json!({"entry_id":card["payload"]["entry_id"],"submission_id":"cancel","conversation_id":c}));
    // A suspended reply is also projected as `echoed`. Wait for the durable
    // run to finish, otherwise the resumed form consumes the next scripted error.
    wait("reprise terminée après annulation", || {
        f.state();
        f.db().query_row("SELECT r.status FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')='form-turn'",[],|r|r.get::<_,String>(0)).unwrap()=="finished"
    });
    assert!(f
        .messages(&c)
        .iter()
        .any(|m| m["id"] == "reply-form-turn" && m["state"] == "echoed"));
    f.settle();
    assert!(f.calls().is_empty());
    f.server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            400,
            json!({"error":{"code":"invalid_request_error","message":"Requête refusée"}}),
        ));
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"failed","body":"Je préfère le café."}),
    );
    wait("échec du tour", || {
        f.messages(&c)
            .iter()
            .any(|m| m["id"] == "reply-failed" && m["state"] == "failed")
    });
    f.settle();
    assert!(f.calls().is_empty());
    assert!(f.facts("grokbot").is_empty());
}

#[test]
fn queued_job_survives_restart_but_claimed_job_is_never_replayed() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"one","body":text}),
    );
    // No host tick: finish/queue commits, without starting a network worker.
    wait("mise en attente durable", || {
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM memory_auto_jobs WHERE state='queued'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    let provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("catalogue", || provider.status()["models_ready"] == true);
    f.settle();
    assert_eq!(f.facts("grokbot").len(), 1);
    assert_eq!(f.calls().len(), 1);
    f.db()
        .execute("UPDATE memory_auto_jobs SET state='running'", [])
        .unwrap();
    let provider = Provider::with_endpoints(&f.path, f.server.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    wait("catalogue", || provider.status()["models_ready"] == true);
    f.settle();
    assert_eq!(f.calls().len(), 1);
}

#[test]
fn legacy_forgotten_tool_fact_bootstraps_its_human_source_tombstone() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.post(
        "/api/memory/automatic",
        json!({"bot_id":"grokbot","enabled":false}),
    );
    f.server.seen.lock().unwrap().responses.push_back(ScriptedResponse::events(&[
        json!({"type":"response.output_item.done","item":{"id":"fc-remember","type":"function_call","status":"completed","call_id":"remember","name":"remember","arguments":json!({"text":text,"source":"user_stated","sensitive":false}).to_string()}}),json!({"type":"response.completed"}),
    ]));
    f.send(&c, "legacy", text);
    let id = f.facts("grokbot")[0].fact_id.clone();
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":id}),
    );
    // Simulate a pre-automatic-memory database: only the existing erased fact
    // and its tool provenance remain, no automatic suppression records.
    f.db().execute_batch("DELETE FROM memory_auto_blocks; DELETE FROM component_versions WHERE component='memory.automatic';").unwrap();
    f.post(
        "/api/memory/automatic",
        json!({"bot_id":"grokbot","enabled":true}),
    );
    f.reopen();
    assert!(
        f.db()
            .query_row("SELECT COUNT(*) FROM memory_auto_blocks", [], |r| r
                .get::<_, i64>(0))
            .unwrap()
            > 0
    );
    f.send(&c, "repeat", text);
    f.settle();
    assert!(f.calls().is_empty());
    assert!(f.facts("grokbot").is_empty());
}

#[test]
fn group_root_extracts_only_the_original_human_text_into_private_memory() {
    let mut f = Fixture::new();
    f.bot();
    let c = f.conv("grokbot");
    let text = "Je préfère le thé vert.";
    f.post(
        "/api/conversations/members/add",
        json!({"conversation_id":c,"bot_id":"other","command_id":"add-other"}),
    );
    f.script(json!([fact(text, None)]));
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"human-root","body":text}),
    );
    wait("tour humain du groupe terminé", || {
        f.state();
        f.db()
            .query_row("SELECT COUNT(*) FROM memory_auto_jobs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            > 0
    });
    f.settle();
    let facts = f.facts("grokbot");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].scope, Scope::Private);
    assert_eq!(facts[0].provenance.entry_id.as_deref(), Some("human-root"));
    assert!(f.facts("other").is_empty());
    let request = f.calls().pop().unwrap();
    let input: Value =
        serde_json::from_str(request["input"][0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["message"], text);
    assert_eq!(f.cards(&c).len(), 1);
}

#[test]
fn rich_group_mentions_are_removed_but_email_values_remain_excluded() {
    let mut f = Fixture::new();
    f.bot();
    let c = f.conv("grokbot");
    f.post(
        "/api/conversations/members/add",
        json!({"conversation_id":c,"bot_id":"other","command_id":"member"}),
    );
    let text = "Je préfère les réponses structurées.";
    f.script(json!([fact(text, None)]));
    f.script(json!([fact(text, None)]));
    f.post("/api/send",json!({"conversation_id":c,"message_id":"everyone","body":format!("@everyone {text}"),"mentions":[{"id":"@everyone","label":"everyone","offset":0}]}));
    wait("deux destinataires terminés", || {
        f.state();
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE status='completed'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 2
    });
    f.settle();
    assert_eq!(f.calls().len(), 2);
    assert_eq!(f.facts("grokbot").len(), 1);
    assert_eq!(f.facts("other").len(), 1);
    for call in f.calls() {
        let data: Value =
            serde_json::from_str(call["input"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(data["message"], text);
    }
    let project = "Je travaille sur le projet Boréal.";
    f.script(json!([fact(project, None)]));
    f.post("/api/send",json!({"conversation_id":c,"message_id":"other-root","body":format!("🦀 @Autre {project}"),"mentions":[{"id":"other","label":"Autre","offset":3}]}));
    wait("mention UTF-16 terminée", || {
        f.state();
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE status='completed'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 3
    });
    f.settle();
    assert_eq!(f.calls().len(), 3);
    assert_eq!(f.facts("other").len(), 2);
    let data: Value = serde_json::from_str(
        f.calls().last().unwrap()["input"][0]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(!data["message"].as_str().unwrap().contains('@'));
    assert!(data["message"].as_str().unwrap().contains('🦀'));
    f.post("/api/send",json!({"conversation_id":c,"message_id":"email","body":"@everyone Je préfère l’adresse x@exemple.fr.","mentions":[{"id":"@everyone","label":"everyone","offset":0}]}));
    wait("tour avec adresse terminé", || {
        f.state();
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE status='completed'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 5
    });
    f.settle();
    assert_eq!(f.calls().len(), 3);
}

#[test]
fn full_deduplication_catalogue_skips_instead_of_duplicating_unseen_facts() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    for i in 0..30 {
        facts::remember(
            &f.db(),
            &facts::Fact {
                fact_id: String::new(),
                bot_id: "grokbot".into(),
                scope: Scope::Private,
                text: format!("Préférence {i} {}", "texte ".repeat(30)),
                provenance: facts::Provenance {
                    conversation_id: None,
                    entry_id: None,
                    source: Source::UserStated,
                },
                learned_at: i,
                status: FactStatus::Active,
                sensitive: false,
            },
        )
        .unwrap();
    }
    f.send(&c, "one", "Je préfère le thé vert.");
    f.settle();
    assert!(f.calls().is_empty());
    assert_eq!(f.facts("grokbot").len(), 30);
    assert!(f.cards(&c).is_empty());
}

#[test]
fn no_small_model_skips_the_call_without_charging_a_large_model() {
    let mut f = Fixture::with_small_model(false);
    assert_eq!(f.state()["memory_auto"]["available"], false);
    let c = f.conv("grokbot");
    f.send(&c, "one", "Je préfère le thé vert.");
    f.settle();
    assert!(f.calls().is_empty());
    assert!(f.facts("grokbot").is_empty());
    assert!(f.cards(&c).is_empty());
}

#[test]
fn known_vault_values_are_excluded_even_without_secret_keywords() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let vault = grokbot_forms::vault::Vault::open(&f.path.with_extension("secrets.json")).unwrap();
    vault
        .set("TEST_VALUE", "Essai", "valeur-factice-inconnue")
        .unwrap();
    f.send(&c, "one", "Je préfère utiliser valeur-factice-inconnue.");
    f.settle();
    assert!(f.calls().is_empty());
    assert!(f.facts("grokbot").is_empty());
}
