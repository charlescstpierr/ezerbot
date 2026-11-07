//! Real Responses boundary, durable queue/cards/settings, no account or app data.
mod support;
use grokbot_memory::facts::{self, FactStatus, Scope, Source};
use grokbot_wrapper::{App, Provider};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};

struct Fixture {
    app: App,
    server: FakeOpenAi,
    path: PathBuf,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        Self::with_small_model(true)
    }
    fn with_small_model(memory_model: bool) -> Self {
        let guard = grokbot_test_support::TestDir::new("memory-auto");
        let path = guard.to_path_buf().join("workspace.sqlite");
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
        Self {
            app,
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
fn undo_cannot_publish_a_private_predecessor_after_the_update_is_shared() {
    for edited in [false, true] {
        let mut f = Fixture::new();
        f.bot();
        let c = f.conv("grokbot");
        let original = "Je préfère le thé vert.";
        f.script(json!([fact(original, None)]));
        f.send(&c, "one", original);
        f.settle();
        let old = f.facts("grokbot")[0].clone();
        let changed = "Je préfère maintenant le café.";
        f.script(json!([fact(changed, Some(&old.fact_id))]));
        f.send(&c, "two", changed);
        f.settle();
        let card = f.cards(&c).pop().unwrap();
        if edited {
            f.post("/api/memory/correct", json!({"bot_id":"grokbot","fact_id":card["payload"]["fact_id"],"text":"Je préfère le café avec du lait."}));
        }
        let head = f.facts("grokbot")[0].clone();
        f.post(
            "/api/memory/promote",
            json!({"bot_id":"grokbot","fact_ids":[head.fact_id]}),
        );
        let blocks: i64 = f
            .db()
            .query_row("SELECT COUNT(*) FROM memory_auto_blocks", [], |r| r.get(0))
            .unwrap();
        let error = f
            .app
            .api(
                "POST",
                "/api/memory/automatic/undo",
                json!({"card_id":card["id"]}).to_string().as_bytes(),
            )
            .unwrap_err();
        assert_eq!(error.status, 409);
        assert!(error.message.contains("changé de portée"));
        assert_eq!(
            f.db()
                .query_row("SELECT COUNT(*) FROM memory_auto_blocks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            blocks
        );
        assert_eq!(
            facts::get(&f.db(), &old.fact_id).unwrap().unwrap().scope,
            Scope::Private
        );
        let visible = f.facts("other");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].text, head.text);
        assert_eq!(visible[0].scope, Scope::Team);
        assert!(!visible.iter().any(|v| v.text == original));
        f.reopen();
        let projected = f.cards(&c).pop().unwrap();
        assert_eq!(projected["payload"]["scope_changed"], true);
        assert_eq!(projected["payload"]["active"], false);
        assert_eq!(projected["payload"]["can_undo"], false);
        assert_eq!(projected["payload"]["text"], "");
        assert_eq!(projected["payload"]["cancelled"], false);
    }
}

#[test]
fn edited_automatic_update_restores_predecessor_without_deleting_or_duplicating_it() {
    for declared in [false, true] {
        let mut f = Fixture::new();
        let c = f.conv("grokbot");
        let original = "Je préfère le thé vert.";
        if declared {
            facts::remember(
                &f.db(),
                &facts::Fact {
                    fact_id: String::new(),
                    bot_id: "grokbot".into(),
                    scope: Scope::Private,
                    text: original.into(),
                    provenance: facts::Provenance {
                        source: Source::UserStated,
                        conversation_id: None,
                        entry_id: None,
                    },
                    learned_at: 1700000000000,
                    status: FactStatus::Active,
                    sensitive: false,
                },
            )
            .unwrap();
        } else {
            f.script(json!([fact(original, None)]));
            f.send(&c, "creation", original);
            f.settle();
        }
        let old = f.facts("grokbot")[0].clone();
        let changed = "Je préfère maintenant le café noir.";
        f.script(json!([fact(changed, Some(&old.fact_id))]));
        f.send(&c, "update", changed);
        f.settle();
        let update = f.cards(&c).last().unwrap().clone();
        let edited = "Je préfère maintenant le café noir avec du lait.";
        f.post(
            "/api/memory/correct",
            json!({"bot_id":"grokbot","fact_id":update["payload"]["fact_id"],"text":edited}),
        );
        let cards = f.cards(&c);
        assert_eq!(
            cards
                .iter()
                .filter(|c| c["payload"]["active"] == true)
                .count(),
            1
        );
        if !declared {
            assert_eq!(cards[0]["payload"]["active"], false);
            assert_eq!(cards[0]["payload"]["updated"], true);
            assert!(f
                .app
                .api(
                    "POST",
                    "/api/memory/automatic/undo",
                    json!({"card_id":cards[0]["id"]}).to_string().as_bytes()
                )
                .is_err());
        }
        f.post(
            "/api/memory/automatic/undo",
            json!({"card_id":update["id"]}),
        );
        let restored = f.facts("grokbot");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].text, original);
        assert_eq!(restored[0].provenance, old.provenance);
        assert_eq!(restored[0].learned_at, old.learned_at);
        assert_ne!(
            facts::get(&f.db(), &old.fact_id).unwrap().unwrap().status,
            FactStatus::Forgotten
        );
        let cards = f.cards(&c);
        assert_eq!(
            cards
                .iter()
                .filter(|c| c["payload"]["active"] == true)
                .count(),
            usize::from(!declared)
        );
        assert_eq!(cards.last().unwrap()["payload"]["restored_active"], true);
        f.send(&c, "repeat-edited", edited);
        f.settle();
        assert_eq!(f.calls().len(), if declared { 1 } else { 2 });
        assert_eq!(f.facts("grokbot")[0].text, original);
        f.reopen();
        assert_eq!(f.facts("grokbot")[0].provenance, old.provenance);
        let later=f.post("/api/memory/correct",json!({"bot_id":"grokbot","fact_id":restored[0].fact_id,"text":"Je préfère le thé vert avec du miel."}));
        assert_eq!(
            f.cards(&c).last().unwrap()["payload"]["restored_active"],
            false
        );
        f.post(
            "/api/memory/forget",
            json!({"bot_id":"grokbot","fact_id":later["fact"]["fact_id"]}),
        );
        assert_eq!(
            f.cards(&c).last().unwrap()["payload"]["restored_active"],
            false
        );
    }
}

#[test]
fn realistic_token_sse_envelope_retains_three_facts_and_transport_stays_bounded() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let texts = [
        "Je préfère travailler ",
        "Je suis organisatrice de projets ",
        "Je travaille sur le projet Boréal ",
    ]
    .map(|prefix| {
        format!(
            "{prefix}{}.",
            "avec des listes lisibles et des étapes faciles à suivre ".repeat(2)
        )
    });
    let output =
        json!({"facts":texts.iter().map(|text|fact(text,None)).collect::<Vec<_>>()}).to_string();
    let reply =
        ScriptedResponse::openai_text(&output, &"Consignes d’extraction bornée. ".repeat(60));
    assert!(reply.body.len() > 30000);
    assert!(reply.body.len() < 512 * 1024);
    f.script_response(reply);
    f.send(&c, "realistic", &texts.join(" "));
    f.settle();
    assert_eq!(f.facts("grokbot").len(), 3);
    assert_eq!(f.cards(&c).len(), 3);
    let mut oversized = response(json!([fact("Je préfère le thé vert.", None)]));
    oversized.body = format!("{}{}", ": keepalive\n\n".repeat(50000), oversized.body);
    f.script_response(oversized);
    f.send(&c, "over-envelope", "Je préfère le thé vert.");
    f.settle();
    assert_eq!(f.facts("grokbot").len(), 3);
    assert_eq!(f.cards(&c).len(), 3);
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
    assert!(f
        .cards(&c)
        .iter()
        .all(|card| card["payload"]["memory_root_id"] == "everyone"));
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
fn plain_everyone_in_group_learns_without_changing_structured_routing() {
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
    f.post(
        "/api/send",
        json!({"conversation_id":c,"message_id":"plain-group","body":format!("@everyone {text}")}),
    );
    wait("groupe texte brut terminé", || {
        f.state();
        f.db()
            .query_row(
                "SELECT COUNT(*) FROM wrapper_group_turns WHERE status='completed'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    });
    f.settle();
    // Bare text does not forge structured recipients. The primary bot is the
    // existing default target; it must still extract despite the '@' token.
    assert_eq!(f.cards(&c).len(), 1);
    assert_eq!(f.facts("grokbot").len(), 1);
    assert!(f.facts("other").is_empty());
    assert_eq!(f.calls().len(), 1);
    assert!(f
        .calls()
        .iter()
        .all(|b| !b["input"].to_string().contains("@everyone")));
}

#[test]
fn responses_round_trip_recalls_an_old_fact_from_five_hundred_before_answering() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let mut old_id = String::new();
    for i in 0..500 {
        let saved = facts::remember(
            &f.db(),
            &facts::Fact {
                fact_id: if i == 499 {
                    "fact_00000000000000000000000000004317".into()
                } else {
                    String::new()
                },
                bot_id: "grokbot".into(),
                scope: Scope::Private,
                text: if i == 0 {
                    "Le projet Boréal utilise le port 4317.".into()
                } else {
                    format!("Le projet récent {i} dispose de sa documentation de développement.")
                },
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
        if i == 0 {
            old_id = saved.fact_id;
        }
    }
    {
        let mut seen = f.server.seen.lock().unwrap();
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_item.done","item":{"id":"fc-recall","type":"function_call","status":"completed","call_id":"recall","name":"recall_memory","arguments":json!({"query":"port Boreal","limit":5}).to_string()}}),
            json!({"type":"response.completed"})]));
        seen.responses.push_back(ScriptedResponse::events(&[
            json!({"type":"response.output_text.delta","delta":"Le projet Boréal utilise le port 4317, selon le souvenir retrouvé."}),json!({"type":"response.completed"})]));
    }
    f.send(&c, "recall-old", "Quel port utilise le projet Boréal ?");
    f.settle();
    let seen = f.server.seen.lock().unwrap();
    let chat: Vec<_> = seen
        .bodies
        .iter()
        .filter(|b| b["tools"].is_array())
        .collect();
    assert_eq!(chat.len(), 2);
    let initial = chat[0].to_string();
    assert!(!initial.contains("port 4317"));
    assert!(!initial.contains(&old_id));
    assert!(chat[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "recall_memory"));
    assert!(chat[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["type"] == "function_call_output"
            && i["output"]
                .as_str()
                .is_some_and(|output| output.contains(&old_id)
                    && output.contains("Le projet Boréal utilise le port 4317."))));
    drop(seen);
    assert!(f
        .messages(&c)
        .iter()
        .any(|m| m["body"].as_str().unwrap_or_default().contains("4317")));
}

fn remember(f: &Fixture, text: &str, learned_at: i64) -> String {
    facts::remember(
        &f.db(),
        &facts::Fact {
            fact_id: String::new(),
            bot_id: "grokbot".into(),
            scope: Scope::Private,
            text: text.into(),
            provenance: facts::Provenance {
                conversation_id: None,
                entry_id: None,
                source: Source::UserStated,
            },
            learned_at,
            status: FactStatus::Active,
            sensitive: false,
        },
    )
    .unwrap()
    .fact_id
}
/// Le modèle factice appelle `recall_memory`, puis répond.
fn script_recall(f: &Fixture, query: &str) {
    let mut seen = f.server.seen.lock().unwrap();
    seen.responses.push_back(ScriptedResponse::events(&[
        json!({"type":"response.output_item.done","item":{"id":"fc-recall","type":"function_call","status":"completed","call_id":"recall","name":"recall_memory","arguments":json!({"query":query,"limit":5}).to_string()}}),
        json!({"type":"response.completed"})]));
    seen.responses.push_back(ScriptedResponse::events(&[
        json!({"type":"response.output_text.delta","delta":"Voilà."}),
        json!({"type":"response.completed"}),
    ]));
}
fn recalled(f: &mut Fixture, c: &str, id: &str) -> Value {
    f.messages(c)
        .into_iter()
        .find(|m| m["id"] == format!("reply-{id}"))
        .unwrap()["payload"]["recalled_fact_ids"]
        .clone()
}

#[test]
fn reply_after_recall_memory_carries_the_ids_of_the_recalled_facts_only() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let port = remember(&f, "Le projet Boréal utilise le port 4317.", 1);
    remember(&f, "Le thé vert est préféré.", 2);
    script_recall(&f, "port Boreal");
    f.send(&c, "ask", "Quel port utilise le projet Boréal ?");
    assert_eq!(recalled(&mut f, &c, "ask"), json!([port]));
}

#[test]
fn facts_only_injected_in_the_bootstrap_are_not_counted_as_recalled() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    remember(&f, "Le projet Boréal utilise le port 4317.", 1);
    f.send(&c, "plain", "Bonjour");
    assert!(f
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .any(|b| b.to_string().contains("port 4317")));
    assert_eq!(recalled(&mut f, &c, "plain"), json!([]));
}

#[test]
fn reply_without_any_memory_has_an_empty_list_even_after_a_recall() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    script_recall(&f, "port Boreal");
    f.send(&c, "empty", "Quel port ?");
    assert_eq!(recalled(&mut f, &c, "empty"), json!([]));
}

#[test]
fn a_recalled_fact_forgotten_afterwards_is_no_longer_listed() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let port = remember(&f, "Le projet Boréal utilise le port 4317.", 1);
    let tea = remember(&f, "Le projet Boréal sert du thé vert.", 2);
    script_recall(&f, "Boreal");
    f.send(&c, "ask", "Parle-moi de Boréal");
    assert_eq!(recalled(&mut f, &c, "ask"), json!([tea, port]));
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":port}),
    );
    assert_eq!(recalled(&mut f, &c, "ask"), json!([tea]));
}

#[test]
fn a_recalled_topic_fact_is_listed_then_dropped_when_charles_forgets_it() {
    let mut f = Fixture::new();
    let topic = f.post("/api/topics", json!({"name":"Déménagement"}))["topic"]["id"].clone();
    let c = f.conv("grokbot");
    f.post(
        "/api/topics/assign",
        json!({"conversation_id":c,"topic_id":topic}),
    );
    let offer = match f
        .app
        .invoke_tool_in(
            "grokbot",
            Some(&c),
            "propose_topic_memory",
            json!({"text":"Le camion est réservé pour le 1er juillet"}),
        )
        .unwrap()
    {
        grokbot_engine::contracts::ToolOutcome::Done { output } => output["offer_id"].clone(),
        _ => panic!("proposition refusée"),
    };
    let fact = f.post("/api/topics/memory/accept", json!({"offer_id":offer}))["fact"]["id"].clone();
    script_recall(&f, "camion");
    f.send(&c, "truck", "Quand est le camion ?");
    assert_eq!(recalled(&mut f, &c, "truck"), json!([fact]));
    f.post("/api/topics/memory/forget", json!({"fact_id":fact}));
    assert_eq!(recalled(&mut f, &c, "truck"), json!([]));
}

#[test]
fn plain_everyone_produces_a_receipt_and_consent_card_without_weakening_email_guard() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je travaille sur le projet Boréal.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "plain", &format!("@everyone {text}"));
    f.settle();
    assert_eq!(f.cards(&c).len(), 1);
    let offer = f
        .messages(&c)
        .into_iter()
        .find(|m| m["kind"] == "memory_share")
        .unwrap();
    assert_eq!(offer["payload"]["facts"][0]["class"], "work");
    assert_eq!(offer["payload"]["facts"][0]["shared"], false);
    assert_eq!(f.facts("grokbot")[0].scope, Scope::Private);
    assert!(!f.calls()[0]["input"].to_string().contains("@everyone"));
    let id = f.facts("grokbot")[0].fact_id.clone();
    f.post(
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":id}),
    );
    let offer = f
        .messages(&c)
        .into_iter()
        .find(|m| m["kind"] == "memory_share")
        .unwrap();
    assert_eq!(offer["payload"]["facts"], json!([]));
    f.send(&c, "address", "@everyone Je préfère x@exemple.fr.");
    f.settle();
    assert_eq!(f.calls().len(), 1);
    f.send(&c, "substring", "@everyoneElse Je préfère le thé vert.");
    f.settle();
    assert_eq!(f.calls().len(), 1);
}

#[test]
fn punctuated_everyone_is_stripped_without_rewriting_addresses_or_longer_names() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je travaille sur le projet Boréal.";
    for (id, body) in [
        ("dot", format!("@everyone. {text}")),
        ("paren", format!("(@everyone) {text}")),
        ("ellipsis", format!("@everyone… {text}")),
    ] {
        f.script(json!([fact(text, None)]));
        f.send(&c, id, &body);
        f.settle();
    }
    let calls = f.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls
        .iter()
        .all(|call| !call["input"].to_string().contains("@everyone")));
    f.send(
        &c,
        "address",
        "Écrivez à x@everyone.com au sujet du projet.",
    );
    f.settle();
    f.send(&c, "longer", "@everyoneElse Je préfère le thé vert.");
    f.settle();
    assert_eq!(f.calls().len(), 3);
}

#[test]
fn share_card_keeps_journal_time_and_the_run_that_offered_it() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    let text = "Je travaille sur le projet Boréal.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "plain", &format!("@everyone {text}"));
    f.settle();
    let messages = f.messages(&c);
    let offer = messages
        .iter()
        .find(|m| m["kind"] == "memory_share")
        .unwrap();
    let reply = messages.iter().find(|m| m["id"] == "reply-plain").unwrap();
    assert!(offer["created_at"].as_u64().unwrap() > 0);
    assert_eq!(
        offer["payload"]["memory_run_id"],
        reply["payload"]["run_id"]
    );
}

fn remember_many(f: &Fixture, count: i64) {
    let conn = f.db();
    for i in 0..count {
        facts::remember(
            &conn,
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
}

fn known_in(request: &Value) -> Vec<Value> {
    let input = request["input"][0]["content"][0]["text"]
        .as_str()
        .or_else(|| request["input"][0]["content"].as_str())
        .unwrap_or_default();
    let body: Value = serde_json::from_str(input).unwrap();
    body["known"].as_array().unwrap().clone()
}

#[test]
fn five_hundred_facts_still_learn_a_new_preference_with_only_close_facts_in_the_prompt() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    remember_many(&f, 500);
    let before = f.facts("grokbot");
    assert_eq!(before.len(), 500);
    assert!(
        before
            .iter()
            .map(|x| x.text.chars().count() + 100)
            .sum::<usize>()
            > 6000
    );
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    assert_eq!(f.facts("grokbot").len(), 501);
    let cards = f.cards(&c);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["payload"]["text"], text);
    let calls = f.calls();
    assert_eq!(calls.len(), 1);
    assert!(known_in(&calls[0]).len() <= 20);
    let status = f.post("/api/memory", json!({"bot_id":"grokbot"}));
    assert!(status.get("automatic").is_none());
}

#[test]
fn five_hundred_facts_do_not_duplicate_a_fact_repeated_in_a_message() {
    let mut f = Fixture::new();
    let c = f.conv("grokbot");
    remember_many(&f, 500);
    let text = "Je préfère le thé vert.";
    f.script(json!([fact(text, None)]));
    f.send(&c, "one", text);
    f.settle();
    assert_eq!(f.facts("grokbot").len(), 501);
    f.script(json!([fact(text, None)]));
    f.send(&c, "two", text);
    f.settle();
    assert_eq!(f.calls().len(), 2);
    let known = known_in(&f.calls()[1]);
    assert!(known.len() <= 20);
    assert!(known.iter().any(|k| k["text"] == text));
    assert_eq!(f.facts("grokbot").len(), 501);
    assert_eq!(f.cards(&c).len(), 1);
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
