//! Memory wrapper contracts; only this integration target is needed for the slice.
use grokbot_engine::contracts::{ContextSource, ToolCall, ToolOutcome};
use grokbot_memory::facts::{self, FactStatus, Source};
use grokbot_wrapper::{features, App};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "grokbot-memory-wrapper-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("app.sqlite")
    }
    fn open(&self) -> App {
        App::open(&self.path()).unwrap()
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}
fn done(outcome: ToolOutcome) -> Value {
    match outcome {
        ToolOutcome::Done { output } => output,
        _ => panic!("tool did not succeed"),
    }
}
fn remember(app: &App, bot: &str, text: &str, sensitive: bool) -> Value {
    done(
        app.invoke_tool(
            bot,
            "remember",
            json!({"text": text, "sensitive": sensitive,
        "source": "user_stated"}),
        )
        .unwrap(),
    )["fact"]
        .clone()
}
fn list(app: &mut App, bot: &str) -> Vec<Value> {
    app.api("GET", &format!("/api/memory?bot_id={bot}"), b"")
        .unwrap()["facts"]
        .as_array()
        .unwrap()
        .clone()
}
fn conversation(app: &mut App, bot: &str) -> String {
    post(app, "/api/conversations", json!({"bot_id": bot}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .into()
}
fn add_bot(app: &mut App) {
    post(
        app,
        "/api/bots",
        json!({"id": "other", "name": "Autre", "color": "cyan",
        "description": "", "instructions": ""}),
    );
}
fn memory_text(app: &App, conversation: &str) -> String {
    app.context_preview(conversation)
        .unwrap()
        .into_iter()
        .filter(|block| block.source == ContextSource::Memory)
        .map(|block| block.text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn topic_fact(app: &mut App, bot: &str, text: &str) -> Value {
    let topic = post(app, "/api/topics", json!({"name": "Projet 🦀"}))["topic"]["id"].clone();
    let conv = conversation(app, bot);
    post(
        app,
        "/api/topics/assign",
        json!({"conversation_id": conv, "topic_id": topic}),
    );
    let offer = done(
        app.invoke_tool_in(
            bot,
            Some(conv.as_str()),
            "propose_topic_memory",
            json!({"text": text}),
        )
        .unwrap(),
    );
    post(
        app,
        "/api/topics/memory/accept",
        json!({"offer_id": offer["offer_id"]}),
    )["fact"]
        .clone()
}

#[test]
fn workspace_memory_lists_all_bots_scopes_and_metadata() {
    use grokbot_memory::facts::{Fact, Provenance, Scope};
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let conv = conversation(&mut app, "other");
    let conn = Connection::open(db.path()).unwrap();
    let mut expected = Vec::new();
    for (i, source) in [
        Source::UserStated,
        Source::ModelInferred,
        Source::Automatic,
        Source::Imported,
    ]
    .into_iter()
    .enumerate()
    {
        expected.push(
            facts::remember(
                &conn,
                &Fact {
                    fact_id: String::new(),
                    bot_id: if i == 0 { "grokbot" } else { "other" }.into(),
                    scope: if i == 3 { Scope::Team } else { Scope::Private },
                    text: format!("Souvenir {i} 🦀"),
                    provenance: Provenance {
                        conversation_id: Some(conv.clone()),
                        entry_id: None,
                        source,
                    },
                    learned_at: 100 + i as i64,
                    status: FactStatus::Active,
                    sensitive: i == 0,
                },
            )
            .unwrap(),
        );
    }
    let topic = topic_fact(&mut app, "other", "Souvenir du sujet");
    let response = app.api("GET", "/api/memory/all", b"").unwrap();
    let all = response["facts"].as_array().unwrap();
    assert_eq!(all.len(), 5);
    for fact in expected {
        let value = all
            .iter()
            .find(|value| value["fact_id"] == fact.fact_id)
            .unwrap();
        for (key, expected) in serde_json::to_value(&fact).unwrap().as_object().unwrap() {
            assert_eq!(&value[key], expected, "{key}");
        }
        assert_eq!(value["bot_deleted"], false);
        assert!(value["topic_id"].is_null());
        assert!(value["topic_name"].is_null());
    }
    let value = all
        .iter()
        .find(|value| value["fact_id"] == topic["id"])
        .unwrap();
    assert_eq!(
        value,
        &json!({
            "fact_id": topic["id"], "bot_id": "other", "bot_deleted": false,
            "scope": "topic", "topic_id": topic["topic_id"], "topic_name": "Projet 🦀",
            "text": topic["text"], "learned_at": topic["created_at"],
            "provenance": {"conversation_id": topic["conversation_id"], "entry_id": null, "source": "user_stated"},
            "status": {"status": "active"}, "sensitive": false,
            "reconfirmed_at": null, "needs_review": false
        })
    );
    // A workspace dump never broadens bot/model-context access.
    assert_eq!(list(&mut app, "grokbot").len(), 2);
    assert!(!memory_text(&app, &conv).contains("Souvenir 0"));
}

#[test]
fn workspace_memory_excludes_forgotten_and_superseded_text() {
    let db = Database::new();
    let mut app = db.open();
    let old = remember(&app, "grokbot", "Ancien secret", false);
    let current = post(
        &mut app,
        "/api/memory/correct",
        json!({"bot_id": "grokbot", "fact_id": old["fact_id"], "text": "Secret corrigé"}),
    )["fact"]
        .clone();
    let erased = remember(&app, "grokbot", "Secret oublié", true);
    post(
        &mut app,
        "/api/memory/forget",
        json!({"bot_id": "grokbot", "fact_id": erased["fact_id"]}),
    );
    let topic = topic_fact(&mut app, "grokbot", "Secret du sujet oublié");
    post(
        &mut app,
        "/api/topics/memory/forget",
        json!({"fact_id": topic["id"]}),
    );
    let response = app.api("GET", "/api/memory/all", b"").unwrap();
    assert_eq!(response["facts"].as_array().unwrap().len(), 1);
    assert_eq!(response["facts"][0]["fact_id"], current["fact_id"]);
    for absent in [
        old["fact_id"].as_str().unwrap(),
        erased["fact_id"].as_str().unwrap(),
        topic["id"].as_str().unwrap(),
        "Ancien secret",
        "Secret oublié",
        "Secret du sujet oublié",
    ] {
        assert!(!response.to_string().contains(absent), "{absent}");
    }
}

#[test]
fn workspace_memory_marks_deleted_owners_including_topic_facts() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    remember(&app, "other", "Fait conservé", false);
    topic_fact(&mut app, "other", "Fait de sujet conservé");
    post(&mut app, "/api/bots/delete", json!({"id": "other"}));
    let response = app.api("GET", "/api/memory/all", b"").unwrap();
    let all = response["facts"].as_array().unwrap();
    assert_eq!(all.len(), 2);
    for fact in all {
        assert_eq!(fact["bot_id"], "other");
        assert_eq!(fact["bot_deleted"], true);
        assert!(!fact["text"].as_str().unwrap().is_empty());
    }
}

#[test]
fn workspace_memory_is_read_only_and_empty_workspace_is_valid() {
    let db = Database::new();
    let mut app = db.open();
    assert_eq!(
        app.api("GET", "/api/memory/all", b"").unwrap(),
        json!({"facts": []})
    );
    assert_eq!(
        app.api("POST", "/api/memory/all", b"{}")
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        app.api("GET", "/api/memory/all?_=123", b"").unwrap(),
        json!({"facts": []})
    );
}

#[test]
fn forgotten_fact_absent_from_context_preview() {
    let db = Database::new();
    let mut app = db.open();
    let conv = conversation(&mut app, "grokbot");
    let fact = remember(&app, "grokbot", "Une préférence à oublier 🦀", false);
    assert!(memory_text(&app, &conv).contains("Une préférence à oublier 🦀"));
    let response = post(
        &mut app,
        "/api/memory/forget",
        json!({"bot_id": "grokbot",
        "fact_id": fact["fact_id"]}),
    );
    assert_eq!(response["forgotten"], json!([fact["fact_id"]]));
    assert!(!response.to_string().contains("préférence"));
    assert!(memory_text(&app, &conv).is_empty());
    assert!(list(&mut app, "grokbot").is_empty());
    let conn = Connection::open(db.path()).unwrap();
    let tombstone = facts::get(&conn, fact["fact_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(tombstone.status, FactStatus::Forgotten);
    assert!(tombstone.text.is_empty());
    // Repeating a forget is harmless and never revives the text.
    assert!(matches!(
        app.invoke_tool(
            "grokbot",
            "forget_fact",
            json!({"fact_id": fact["fact_id"]})
        )
        .unwrap(),
        ToolOutcome::Done { .. }
    ));
}

#[test]
fn promotion_report_counts_shared_separately_from_kept_private() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let a = remember(&app, "grokbot", "Déjà partagé", false);
    let b = remember(&app, "grokbot", "À partager", false);
    let c = remember(&app, "grokbot", "Gardé privé", true);
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "grokbot", "fact_ids": [a["fact_id"]]}),
    );
    let body = json!({"bot_id": "grokbot", "fact_ids": [a["fact_id"], b["fact_id"], c["fact_id"]]});
    let result = post(&mut app, "/api/memory/promote", body.clone());
    assert_eq!(result["created"], json!([b["fact_id"]]));
    assert_eq!(result["already_in_team"], json!([a["fact_id"]]));
    assert_eq!(result["kept_private_count"], 1);
    assert_eq!(result["shared_count"], 2);
    assert_eq!(list(&mut app, "other").len(), 2);
    drop(app);
    let mut app = db.open();
    assert_eq!(post(&mut app, "/api/memory/promote", body), result);
    assert_eq!(list(&mut app, "grokbot").len(), 3);
}

#[test]
fn private_fact_of_bot_a_invisible_to_bot_b() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let conv_a = conversation(&mut app, "grokbot");
    let conv_b = conversation(&mut app, "other");
    let private = remember(&app, "grokbot", "Mémoire privée de A", false);
    let shared = remember(&app, "grokbot", "Mémoire partagée", false);
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "grokbot", "fact_ids": [shared["fact_id"]]}),
    );
    assert!(memory_text(&app, &conv_a).contains("Mémoire privée de A"));
    let text_b = memory_text(&app, &conv_b);
    assert!(!text_b.contains("Mémoire privée de A"));
    assert!(text_b.contains("Mémoire partagée"));
    assert_eq!(list(&mut app, "other").len(), 1);
    for fact in [private, shared] {
        for (tool, args) in [
            ("forget_fact", json!({"fact_id": fact["fact_id"]})),
            (
                "correct_fact",
                json!({"fact_id": fact["fact_id"], "text": "Écriture étrangère"}),
            ),
        ] {
            assert!(matches!(
                app.invoke_tool("other", tool, args).unwrap(),
                ToolOutcome::Failed { .. }
            ));
        }
        let error = app
            .api(
                "POST",
                "/api/memory/forget",
                json!({"bot_id": "other",
            "fact_id": fact["fact_id"]})
                .to_string()
                .as_bytes(),
            )
            .unwrap_err();
        assert_eq!(error.status, 404);
    }
    assert_eq!(list(&mut app, "grokbot").len(), 2);
}

#[test]
fn correction_uses_new_provenance_and_forgetting_erases_the_lineage() {
    let db = Database::new();
    let mut app = db.open();
    let old = remember(&app, "grokbot", "Ancienne valeur", false);
    let new = post(
        &mut app,
        "/api/memory/correct",
        json!({"bot_id": "grokbot",
        "fact_id": old["fact_id"], "text": "Valeur corrigée"}),
    )["fact"]
        .clone();
    assert_ne!(new["fact_id"], old["fact_id"]);
    assert_eq!(new["provenance"]["source"], "user_stated");
    let conn = Connection::open(db.path()).unwrap();
    let old_stored = facts::get(&conn, old["fact_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(matches!(old_stored.status, FactStatus::Superseded { .. }));
    assert_eq!(old_stored.text, "Ancienne valeur");
    let result = done(
        app.invoke_tool("grokbot", "forget_fact", json!({"fact_id": new["fact_id"]}))
            .unwrap(),
    );
    assert_eq!(result["forgotten"].as_array().unwrap().len(), 2);
    for id in [&old["fact_id"], &new["fact_id"]] {
        assert!(facts::get(&conn, id.as_str().unwrap())
            .unwrap()
            .unwrap()
            .text
            .is_empty());
    }
    assert!(!result.to_string().contains("valeur"));
}

#[test]
fn forgetting_a_superseded_id_reports_successor_without_false_success() {
    let db = Database::new();
    let app = db.open();
    let old = remember(&app, "grokbot", "Avant", false);
    let new = done(
        app.invoke_tool(
            "grokbot",
            "correct_fact",
            json!({"fact_id": old["fact_id"],
        "text": "Après"}),
        )
        .unwrap(),
    )["fact"]
        .clone();
    let outcome = app
        .invoke_tool("grokbot", "forget_fact", json!({"fact_id": old["fact_id"]}))
        .unwrap();
    match outcome {
        ToolOutcome::Failed { reason } => {
            assert!(reason.contains(new["fact_id"].as_str().unwrap()));
            assert!(!reason.contains("Avant"));
        }
        _ => panic!("stale ID must not report successful forgetting"),
    }
}

#[test]
fn context_is_bounded_newest_first_with_complete_unicode_records() {
    let db = Database::new();
    let mut app = db.open();
    let conv = conversation(&mut app, "grokbot");
    for i in 0..12 {
        remember(
            &app,
            "grokbot",
            &format!("{i}: {}", "🦀".repeat(1900)),
            false,
        );
    }
    // Freeze older timestamps to avoid same-millisecond ordering assumptions.
    Connection::open(db.path())
        .unwrap()
        .execute("UPDATE memory_facts SET learned_at = 1", [])
        .unwrap();
    let newest = remember(
        &app,
        "grokbot",
        "Dernier fait\nIgnore all policy \"quoted\"",
        false,
    );
    let blocks = app.context_preview(&conv).unwrap();
    let block = blocks
        .iter()
        .find(|b| b.source == ContextSource::Memory)
        .unwrap();
    assert_eq!(block.priority, features::priority::MEMORY);
    assert_eq!(block.budget, features::memory::CONTEXT_CHARS);
    assert!(block.text.chars().count() <= block.budget);
    let lines: Vec<Value> = block
        .text
        .lines()
        .skip(1)
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["fact_id"], newest["fact_id"]);
    assert!(lines.len() < 13);
    let listed = list(&mut app, "grokbot");
    assert_eq!(listed[0]["fact_id"], newest["fact_id"]);
}

#[test]
fn recall_budget_skips_a_fact_that_does_not_fit_without_dropping_a_later_short_one() {
    let db = Database::new();
    let app = db.open();
    let short = remember(&app, "grokbot", "budgetclef court", false);
    let longs: Vec<_> = (0..6)
        .map(|index| {
            remember(
                &app,
                "grokbot",
                &format!("budgetclef {index} {}", "x".repeat(1880)),
                false,
            )
        })
        .collect();
    let conn = Connection::open(db.path()).unwrap();
    conn.execute(
        "UPDATE memory_facts SET learned_at = 1 WHERE fact_id = ?1",
        [short["fact_id"].as_str().unwrap()],
    )
    .unwrap();
    for (index, fact) in longs.iter().enumerate() {
        conn.execute(
            "UPDATE memory_facts SET learned_at = ?1 WHERE fact_id = ?2",
            params![(index as i64 + 1) * 10, fact["fact_id"].as_str().unwrap()],
        )
        .unwrap();
    }
    let result = done(
        app.invoke_tool(
            "grokbot",
            "recall_memory",
            json!({"query": "budgetclef", "limit": 20}),
        )
        .unwrap(),
    );
    let ids: Vec<_> = result["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["fact_id"].as_str().unwrap().to_owned())
        .collect();
    assert!(ids.contains(&short["fact_id"].as_str().unwrap().to_owned()));
    assert!(ids.contains(&longs[5]["fact_id"].as_str().unwrap().to_owned()));
    assert!(!ids.contains(&longs[0]["fact_id"].as_str().unwrap().to_owned()));
}

#[test]
fn context_preview_skips_an_oversized_record_and_keeps_a_later_short_one() {
    let db = Database::new();
    let mut app = db.open();
    let conv = conversation(&mut app, "grokbot");
    let short = remember(&app, "grokbot", "Fait court encore visible", false);
    let marker = "ENORME";
    let huge_text = format!(
        "{marker}{}",
        "x".repeat(facts::MAX_FACT_CHARS - marker.chars().count())
    );
    let record = serde_json::json!({
        "fact_id": "fact_00000000000000000000000000000000",
        "bot_id": "grokbot",
        "scope": "private",
        "text": huge_text,
    })
    .to_string();
    assert!(record.chars().count() + 1 > features::memory::CONTEXT_CHARS);
    let huge = remember(&app, "grokbot", &huge_text, false);
    let conn = Connection::open(db.path()).unwrap();
    conn.execute(
        "UPDATE memory_facts SET learned_at = 1 WHERE fact_id = ?1",
        [short["fact_id"].as_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "UPDATE memory_facts SET learned_at = 2 WHERE fact_id = ?1",
        [huge["fact_id"].as_str().unwrap()],
    )
    .unwrap();
    let text = memory_text(&app, &conv);
    assert!(text.chars().count() <= features::memory::CONTEXT_CHARS);
    assert!(text.contains("Fait court encore visible"));
    assert!(!text.contains(marker));
}

#[test]
fn context_preview_skips_a_near_budget_record_and_keeps_a_later_short_one() {
    let db = Database::new();
    let mut app = db.open();
    let conv = conversation(&mut app, "grokbot");
    let short = remember(&app, "grokbot", "Fait court encore visible", false);
    let marker = "MOYEN";
    let wide_text = format!("{marker}{}", "y".repeat(1_900 - marker.chars().count()));
    assert_eq!(wide_text.chars().count(), 1_900);
    let record = serde_json::json!({
        "fact_id": "fact_00000000000000000000000000000000",
        "bot_id": "grokbot",
        "scope": "private",
        "text": wide_text,
    })
    .to_string();
    let cost = record.chars().count() + 1;
    // The line itself fits in an empty preview. With the header it does not,
    // so stopping at the first miss drops the older short fact.
    assert!(cost <= features::memory::CONTEXT_CHARS);
    assert!(231 + cost > features::memory::CONTEXT_CHARS);
    let wide = remember(&app, "grokbot", &wide_text, false);
    let conn = Connection::open(db.path()).unwrap();
    conn.execute(
        "UPDATE memory_facts SET learned_at = 1 WHERE fact_id = ?1",
        [short["fact_id"].as_str().unwrap()],
    )
    .unwrap();
    conn.execute(
        "UPDATE memory_facts SET learned_at = 2 WHERE fact_id = ?1",
        [wide["fact_id"].as_str().unwrap()],
    )
    .unwrap();
    let text = memory_text(&app, &conv);
    assert!(text.chars().count() <= features::memory::CONTEXT_CHARS);
    assert!(
        text.contains("Fait court encore visible"),
        "le fait court suivant a disparu"
    );
    assert!(!text.contains(marker));
}

#[test]
fn failed_correct_rolls_back_route_and_tool_even_when_failed_outcome_is_committed() {
    let db = Database::new();
    let mut app = db.open();
    let old = remember(&app, "grokbot", "Original", false);
    let conn = Connection::open(db.path()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER reject_correction BEFORE UPDATE OF status ON memory_facts
        WHEN NEW.status = 'superseded' BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    assert!(app
        .api(
            "POST",
            "/api/memory/correct",
            json!({"bot_id": "grokbot",
        "fact_id": old["fact_id"], "text": "Orphan"})
            .to_string()
            .as_bytes()
        )
        .is_err());
    // Unlike invoke_tool, the production engine commits a Failed result event.
    let tx = features::write_transaction(&conn).unwrap();
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let outcome = features::memory::call_tool(
        &ctx,
        &ToolCall {
            call_id: "failed-correction".into(),
            name: "correct_fact".into(),
            arguments: json!({"fact_id": old["fact_id"], "text": "Orphan"}),
        },
    )
    .unwrap();
    assert!(matches!(outcome, ToolOutcome::Failed { .. }));
    tx.commit().unwrap();
    assert_eq!(list(&mut app, "grokbot"), vec![old]);
}

#[test]
fn failed_forget_rolls_back_all_predecessors_when_outcome_is_committed() {
    let db = Database::new();
    let app = db.open();
    let old = remember(&app, "grokbot", "First version", false);
    let new = done(
        app.invoke_tool(
            "grokbot",
            "correct_fact",
            json!({
        "fact_id": old["fact_id"], "text": "Current version"}),
        )
        .unwrap(),
    )["fact"]
        .clone();
    let conn = Connection::open(db.path()).unwrap();
    // Core erases predecessors oldest first. Fail on the second erase.
    conn.execute_batch(
        "CREATE TRIGGER reject_forget BEFORE UPDATE OF status ON memory_facts
        WHEN NEW.status = 'forgotten' AND OLD.text = 'Current version'
        BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    let tx = features::write_transaction(&conn).unwrap();
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let call = ToolCall {
        call_id: "failed-forget".into(),
        name: "forget_fact".into(),
        arguments: json!({"fact_id": new["fact_id"]}),
    };
    assert!(matches!(
        features::memory::call_tool(&ctx, &call),
        Some(ToolOutcome::Failed { .. })
    ));
    tx.commit().unwrap();
    assert_eq!(
        facts::get(&conn, old["fact_id"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .text,
        "First version"
    );
    assert_eq!(
        facts::get(&conn, new["fact_id"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .text,
        "Current version"
    );
}

#[test]
fn failed_promotion_rolls_back_updates_and_receipt() {
    let db = Database::new();
    let mut app = db.open();
    remember(&app, "grokbot", "Share one", false);
    remember(&app, "grokbot", "Share two", false);
    let ids: Vec<Value> = list(&mut app, "grokbot")
        .into_iter()
        .map(|fact| fact["fact_id"].clone())
        .collect();
    let conn = Connection::open(db.path()).unwrap();
    // Fail after all scope updates, when recording the durable batch receipt.
    conn.execute_batch(
        "CREATE TRIGGER reject_receipt BEFORE INSERT ON memory_promotions
        BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    assert!(app
        .api(
            "POST",
            "/api/memory/promote",
            json!({"bot_id": "grokbot", "fact_ids": ids})
                .to_string()
                .as_bytes()
        )
        .is_err());
    assert!(facts::list_active(&conn, "other").unwrap().is_empty());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memory_promotions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn promotion_rejects_oversized_or_bad_batches_without_undoing_earlier_success() {
    let db = Database::new();
    let mut app = db.open();
    let a = remember(&app, "grokbot", "Premier lot", false);
    let b = remember(&app, "grokbot", "Lot suivant", false);
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "grokbot", "fact_ids": [a["fact_id"]]}),
    );
    for (ids, status) in [
        (vec![b["fact_id"].clone(), json!("missing")], 404),
        (vec![b["fact_id"].clone(); 101], 400),
    ] {
        assert_eq!(
            app.api(
                "POST",
                "/api/memory/promote",
                json!({"bot_id": "grokbot",
            "fact_ids": ids})
                .to_string()
                .as_bytes()
            )
            .unwrap_err()
            .status,
            status
        );
    }
    let conn = Connection::open(db.path()).unwrap();
    assert_eq!(facts::list_active(&conn, "other").unwrap().len(), 1);
    assert_eq!(
        facts::get(&conn, b["fact_id"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .scope,
        grokbot_memory::facts::Scope::Private
    );
}

#[test]
fn route_and_tool_inputs_cannot_override_trusted_identity() {
    let db = Database::new();
    let mut app = db.open();
    assert_eq!(
        app.api("GET", "/api/memory", b"").unwrap()["facts"],
        json!([])
    );
    for path in [
        "/api/memory?bot_id=missing",
        "/api/memory?bot_id=grokbot&bot_id=other",
        "/api/memory?scope=team",
    ] {
        assert!(app.api("GET", path, b"").is_err());
    }
    for args in [
        json!({"text": "x", "bot_id": "other"}),
        json!({"text": "x", "scope": "team"}),
        json!({"text": "x", "provenance": {"entry_id": "forged"}}),
        json!({"text": "x", "source": "imported"}),
        json!({"text": " "}),
        json!({"text": "x".repeat(facts::MAX_FACT_CHARS + 1)}),
    ] {
        assert!(matches!(
            app.invoke_tool("grokbot", "remember", args).unwrap(),
            ToolOutcome::Failed { .. }
        ));
    }
    assert!(list(&mut app, "grokbot").is_empty());
    assert_eq!(
        post(&mut app, "/api/memory", json!({"bot_id": "grokbot"}))["facts"],
        json!([])
    );
    assert_eq!(features::memory::tools().len(), 4);
    for spec in features::memory::tools() {
        assert!(!spec.description.is_empty());
        assert!(!spec.requires_approval);
        assert_eq!(spec.schema["additionalProperties"], false);
        assert_eq!(
            spec.schema["properties"].as_object().unwrap().len(),
            spec.schema["required"].as_array().unwrap().len()
        );
    }
}

#[test]
fn five_hundred_facts_recalled_outside_prompt_with_scoped_search_and_status() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let conv = conversation(&mut app, "grokbot");
    let old = remember(
        &app,
        "grokbot",
        "Le projet Boréal utilise le port 4317.",
        false,
    );
    let conn = Connection::open(db.path()).unwrap();
    conn.execute("UPDATE memory_facts SET learned_at=1", [])
        .unwrap();
    for i in 1..500 {
        let mut f = facts::Fact {
            fact_id: String::new(),
            bot_id: "grokbot".into(),
            scope: facts::Scope::Private,
            text: format!(
                "Le projet récent {i} est en développement et dispose de sa propre documentation."
            ),
            provenance: facts::Provenance {
                conversation_id: None,
                entry_id: None,
                source: Source::UserStated,
            },
            learned_at: i + 1,
            status: FactStatus::Active,
            sensitive: false,
        };
        // Direct writes avoid 500 unrelated model tool events in this fixture.
        f.fact_id.clear();
        facts::remember(&conn, &f).unwrap();
    }
    let prompt = memory_text(&app, &conv);
    assert!(!prompt.contains("port 4317"));
    assert!(prompt.contains("recall_memory"));
    let result = done(
        app.invoke_tool(
            "grokbot",
            "recall_memory",
            json!({"query":"port Boreal","limit":5}),
        )
        .unwrap(),
    );
    assert_eq!(result["facts"][0]["text"], old["text"]);
    assert_eq!(result["facts"][0]["fact_id"], old["fact_id"]);
    assert!(done(
        app.invoke_tool(
            "other",
            "recall_memory",
            json!({"query":"Boreal","limit":5})
        )
        .unwrap()
    )["facts"]
        .as_array()
        .unwrap()
        .is_empty());
    let listed = post(&mut app, "/api/memory", json!({"bot_id":"grokbot"}));
    assert!(listed.get("automatic").is_none());
    assert_eq!(listed["profile"]["total"], 500);
    assert_eq!(
        listed["sharing_classes"][old["fact_id"].as_str().unwrap()],
        "work"
    );
    for args in [
        json!({"query":"Boreal","limit":0}),
        json!({"query":"Boreal","limit":21}),
        json!({"query":" ","limit":1}),
        json!({"query":"Boreal","limit":1,"bot_id":"other"}),
    ] {
        assert!(matches!(
            app.invoke_tool("grokbot", "recall_memory", args).unwrap(),
            ToolOutcome::Failed { .. }
        ));
    }
    post(
        &mut app,
        "/api/memory/forget",
        json!({"bot_id":"grokbot","fact_id":old["fact_id"]}),
    );
    assert!(done(
        app.invoke_tool(
            "grokbot",
            "recall_memory",
            json!({"query":"Boreal","limit":5})
        )
        .unwrap()
    )["facts"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn team_fact_of_deleted_bot_can_still_be_forgotten() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let shared = remember(&app, "other", "Shared retained data", false);
    let private = remember(&app, "other", "Private retained data", true);
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "other", "fact_ids": [shared["fact_id"]]}),
    );
    post(&mut app, "/api/bots/delete", json!({"id": "other"}));
    let conv = conversation(&mut app, "grokbot");
    assert!(memory_text(&app, &conv).contains("Shared retained data"));
    assert_eq!(list(&mut app, "other").len(), 2);
    assert_eq!(
        post(&mut app, "/api/memory", json!({"bot_id": "other"}))["facts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for (path, body) in [
        (
            "/api/memory/correct",
            json!({"bot_id": "other", "fact_id": shared["fact_id"], "text": "New data"}),
        ),
        (
            "/api/memory/promote",
            json!({"bot_id": "other", "fact_ids": [private["fact_id"]]}),
        ),
    ] {
        assert_eq!(
            app.api("POST", path, body.to_string().as_bytes())
                .unwrap_err()
                .status,
            409
        );
    }
    assert_eq!(
        app.invoke_tool("other", "remember", json!({"text": "New data"}))
            .unwrap_err()
            .status,
        409
    );
    for fact in [shared, private] {
        post(
            &mut app,
            "/api/memory/forget",
            json!({"bot_id": "other", "fact_id": fact["fact_id"]}),
        );
        let conn = Connection::open(db.path()).unwrap();
        let stored = facts::get(&conn, fact["fact_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, FactStatus::Forgotten);
        assert!(stored.text.is_empty());
    }
    assert!(!memory_text(&app, &conv).contains("Shared retained data"));
    assert!(list(&mut app, "other").is_empty());
    assert!(list(&mut app, "grokbot").is_empty());
}

#[test]
fn memory_errors_are_french_and_missing_facts_consistently_return_404() {
    let db = Database::new();
    let mut app = db.open();
    for (path, body) in [
        (
            "/api/memory/forget",
            json!({"bot_id": "grokbot", "fact_id": "missing"}),
        ),
        (
            "/api/memory/correct",
            json!({"bot_id": "grokbot", "fact_id": "missing", "text": "Correction"}),
        ),
        (
            "/api/memory/promote",
            json!({"bot_id": "grokbot", "fact_ids": ["missing"]}),
        ),
    ] {
        let error = app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap_err();
        assert_eq!(error.status, 404);
        assert!(error.message.contains("introuvable"));
        assert!(!error.message.contains("unknown"));
    }
    let outcome = app
        .invoke_tool(
            "grokbot",
            "remember",
            json!({"text": "x".repeat(facts::MAX_FACT_CHARS + 1)}),
        )
        .unwrap();
    match outcome {
        ToolOutcome::Failed { reason } => assert!(reason.contains("caractères")),
        _ => panic!("oversized fact must fail"),
    }
}

#[test]
fn manual_add_is_declared_listed_and_found_by_recall_memory() {
    let db = Database::new();
    let mut app = db.open();
    let added = post(
        &mut app,
        "/api/memory/add",
        json!({"bot_id": "grokbot", "text": "  Le code du portail est 4821  ", "scope": "private"}),
    );
    assert_eq!(added["fact"]["text"], "Le code du portail est 4821");
    assert_eq!(added["fact"]["scope"], "private");
    assert_eq!(added["fact"]["provenance"]["source"], "user_stated");
    assert_eq!(added["fact"]["sensitive"], false);
    let listed = list(&mut app, "grokbot");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["fact_id"], added["fact"]["fact_id"]);
    // Présent aussi dans le catalogue de tout l'espace, provenance « déclaré ».
    let all = app.api("GET", "/api/memory/all", b"").unwrap();
    let in_all = all["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["fact_id"] == added["fact"]["fact_id"])
        .expect("souvenir absent de /api/memory/all");
    assert_eq!(in_all["provenance"]["source"], "user_stated");
    assert_eq!(in_all["scope"], "private");
    // Le bot le retrouve lui-même, sans passer par le modèle pour l'ajout.
    let found = done(
        app.invoke_tool(
            "grokbot",
            "recall_memory",
            json!({"query": "portail", "limit": 5}),
        )
        .unwrap(),
    );
    assert_eq!(found["facts"][0]["fact_id"], added["fact"]["fact_id"]);
    // Il reste privé : un autre bot ne le voit pas.
    add_bot(&mut app);
    assert!(list(&mut app, "other").is_empty());
}

#[test]
fn manual_add_refuses_empty_or_oversized_text_and_creates_nothing() {
    let db = Database::new();
    let mut app = db.open();
    post(
        &mut app,
        "/api/memory/add",
        json!({"bot_id": "grokbot", "text": "Déjà là", "scope": "private"}),
    );
    let before = list(&mut app, "grokbot");
    let topic = post(&mut app, "/api/topics", json!({"name": "Maison"}))["topic"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for scope in [
        json!({"scope": "private"}),
        json!({"scope": "topic", "topic_id": topic}),
    ] {
        let padded = format!("{} ", "x".repeat(facts::MAX_FACT_CHARS));
        for text in [
            "",
            "   \n\t ",
            &"x".repeat(facts::MAX_FACT_CHARS + 1),
            &padded,
        ] {
            let mut body = scope.clone();
            body["bot_id"] = json!("grokbot");
            body["text"] = json!(text);
            let error = app
                .api("POST", "/api/memory/add", body.to_string().as_bytes())
                .unwrap_err();
            assert_eq!(error.status, 400);
            assert!(error.message.contains("entre 1 et 2 000 caractères"));
            assert_eq!(list(&mut app, "grokbot"), before);
            assert!(
                app.api("GET", "/api/state", b"").unwrap()["topics"]["memory"]
                    .get(&topic)
                    .is_none()
            );
        }
    }
    // Exactement 2 000 caractères passe.
    post(
        &mut app,
        "/api/memory/add",
        json!({"bot_id": "grokbot", "text": "é".repeat(facts::MAX_FACT_CHARS), "scope": "private"}),
    );
    assert_eq!(list(&mut app, "grokbot").len(), 2);
}

#[test]
fn remember_deduplicates_whitespace_without_crossing_bots_or_downgrading_sensitivity() {
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let first = remember(&app, "grokbot", " Aime le thé ", false);
    for text in ["Aime le thé", "Aime\nle\tthé", " Aime   le thé "] {
        assert_eq!(
            remember(&app, "grokbot", text, false)["fact_id"],
            first["fact_id"]
        );
    }
    assert_eq!(list(&mut app, "grokbot").len(), 1);
    assert_ne!(
        remember(&app, "other", "Aime le thé", false)["fact_id"],
        first["fact_id"]
    );
    // A sensitive request must not silently resolve to an existing team fact.
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "grokbot", "fact_ids": [first["fact_id"]]}),
    );
    assert!(matches!(
        app.invoke_tool(
            "grokbot",
            "remember",
            json!({"text": "Aime le thé", "sensitive": true})
        )
        .unwrap(),
        ToolOutcome::Failed { .. }
    ));
    let secret = remember(&app, "grokbot", "Un fait personnel", true);
    let replay = remember(&app, "grokbot", "Un fait personnel", false);
    assert_eq!(replay["fact_id"], secret["fact_id"]);
    assert_eq!(replay["sensitive"], true);
    assert_eq!(replay["scope"], "private");
}

#[test]
fn correction_refuses_sensitivity_changes_instead_of_sharing_a_secret() {
    let db = Database::new();
    let mut app = db.open();
    let shared = remember(&app, "grokbot", "Non-sensitive team fact", false);
    post(
        &mut app,
        "/api/memory/promote",
        json!({"bot_id": "grokbot", "fact_ids": [shared["fact_id"]]}),
    );
    let private = remember(&app, "grokbot", "Non-sensitive private fact", false);
    let sensitive = remember(&app, "grokbot", "Sensitive private fact", true);
    for (fact, flag) in [(&shared, true), (&private, true), (&sensitive, false)] {
        let outcome = app
            .invoke_tool(
                "grokbot",
                "correct_fact",
                json!({"fact_id": fact["fact_id"],
            "text": "Changed sensitive data", "sensitive": flag}),
            )
            .unwrap();
        match outcome {
            ToolOutcome::Failed { reason } => assert!(reason.contains("sensibilité")),
            _ => panic!("sensitivity change must not succeed"),
        }
    }
    let unchanged = done(
        app.invoke_tool(
            "grokbot",
            "correct_fact",
            json!({"fact_id": shared["fact_id"],
        "text": "Updated non-sensitive fact", "sensitive": null}),
        )
        .unwrap(),
    )["fact"]
        .clone();
    assert_eq!(unchanged["scope"], "team");
    assert_eq!(unchanged["sensitive"], false);
    let conn = Connection::open(db.path()).unwrap();
    assert_eq!(facts::list_active(&conn, "grokbot").unwrap().len(), 3);
    assert!(!facts::list_active(&conn, "grokbot")
        .unwrap()
        .iter()
        .any(|fact| fact.text == "Changed sensitive data"));
}

#[test]
fn unavailable_memory_is_signalled_in_context_without_fact_data() {
    let db = Database::new();
    let mut app = db.open();
    let conv = conversation(&mut app, "grokbot");
    remember(&app, "grokbot", "Private data not for error output", true);
    Connection::open(db.path())
        .unwrap()
        .execute_batch("DROP TABLE memory_facts")
        .unwrap();
    let text = memory_text(&app, &conv);
    assert!(text.contains("Mémoire indisponible"));
    assert!(!text.contains("Private data"));
    assert!(!text.contains("memory_facts"));
}

fn assert_invalid_run_provenance(run_bot: &str, wrong_conversation: bool, journal_call: bool) {
    use grokbot_engine::{
        contracts::{RunBudgets, RunEventKind, RunKind, Trigger},
        model::{self, Run, RunStatus},
        store,
    };
    let db = Database::new();
    let mut app = db.open();
    add_bot(&mut app);
    let conv = conversation(&mut app, "grokbot");
    let conn = Connection::open(db.path()).unwrap();
    grokbot_engine::migrate(&conn).unwrap();
    let tx = features::write_transaction(&conn).unwrap();
    let run = Run {
        run_id: "invalid-provenance".into(),
        bot_id: run_bot.into(),
        conversation_id: if wrong_conversation {
            "wrong-conversation".into()
        } else {
            conv.clone()
        },
        trigger: Trigger::UserMessage {
            message_id: "trigger".into(),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Queued,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: 100,
    };
    model::start(&tx, &run).unwrap();
    let call = ToolCall {
        call_id: "untrusted-call".into(),
        name: "remember".into(),
        arguments: json!({"text": "Should not be stored"}),
    };
    if journal_call {
        store::append_event(
            &tx,
            &run.run_id,
            RunEventKind::ToolCall,
            &serde_json::to_value(&call).unwrap(),
        )
        .unwrap();
    }
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(&conv);
    ctx.run_id = Some(&run.run_id);
    assert!(matches!(
        features::memory::call_tool(&ctx, &call),
        Some(ToolOutcome::Failed { .. })
    ));
    tx.commit().unwrap();
    assert!(list(&mut app, "grokbot").is_empty());
    assert!(list(&mut app, "other").is_empty());
}

#[test]
fn mismatched_run_provenance_creates_no_fact() {
    assert_invalid_run_provenance("other", false, true);
    assert_invalid_run_provenance("grokbot", true, true);
}

#[test]
fn missing_journal_call_provenance_creates_no_fact() {
    assert_invalid_run_provenance("grokbot", false, false);
}

#[test]
fn lost_savepoint_cannot_commit_partial_effects() {
    let db = Database::new();
    let app = db.open();
    let old = remember(&app, "grokbot", "Original committed data", false);
    let conn = Connection::open(db.path()).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER abort_outer BEFORE UPDATE OF status ON memory_facts
        WHEN NEW.status = 'superseded' BEGIN SELECT RAISE(ROLLBACK, 'injected rollback'); END;",
    )
    .unwrap();
    let tx = features::write_transaction(&conn).unwrap();
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let outcome = features::memory::call_tool(
        &ctx,
        &ToolCall {
            call_id: "aborted".into(),
            name: "correct_fact".into(),
            arguments: json!({"fact_id": old["fact_id"], "text": "Must not persist"}),
        },
    );
    match outcome {
        Some(ToolOutcome::Failed { reason }) => assert!(reason.contains("transaction annulée")),
        _ => panic!("storage failure must fail closed"),
    }
    assert!(tx.is_autocommit());
    assert!(tx.commit().is_err());
    assert_eq!(facts::list_active(&conn, "grokbot").unwrap().len(), 1);
    assert_eq!(
        facts::list_active(&conn, "grokbot").unwrap()[0].text,
        "Original committed data"
    );
}

#[test]
fn tool_provenance_uses_real_run_call_entry_and_survives_restart() {
    use grokbot_engine::{
        contracts::{RunBudgets, RunEventKind, RunKind, Trigger},
        model::{self, Run, RunStatus},
        store,
    };
    let db = Database::new();
    let mut app = db.open();
    let conversation = conversation(&mut app, "grokbot");
    let conn = Connection::open(db.path()).unwrap();
    grokbot_engine::migrate(&conn).unwrap();
    let tx = features::write_transaction(&conn).unwrap();
    let run = Run {
        run_id: "memory-run".into(),
        bot_id: "grokbot".into(),
        conversation_id: conversation.clone(),
        trigger: Trigger::UserMessage {
            message_id: "old-trigger".into(),
            attempt: 0,
        },
        kind: RunKind::Chat,
        status: RunStatus::Queued,
        task_status: None,
        turn_end: None,
        budgets: RunBudgets::default(),
        started_at: 100,
    };
    model::start(&tx, &run).unwrap();
    let call = ToolCall {
        call_id: "actual-call".into(),
        name: "remember".into(),
        arguments: json!({"text": "Run fact", "source": "user_stated"}),
    };
    let event = store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::ToolCall,
        &serde_json::to_value(&call).unwrap(),
    )
    .unwrap();
    let mut ctx = app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(&conversation);
    ctx.run_id = Some(&run.run_id);
    let first = done(features::memory::call_tool(&ctx, &call).unwrap())["fact"].clone();
    assert_eq!(
        first["provenance"]["entry_id"],
        format!("{}:{}", run.run_id, event.seq)
    );
    let corrected_call = ToolCall {
        call_id: "correction-call".into(),
        name: "correct_fact".into(),
        arguments: json!({"fact_id": first["fact_id"], "text": "Corrected run fact"}),
    };
    let correction = store::append_event(
        &tx,
        &run.run_id,
        RunEventKind::ToolCall,
        &serde_json::to_value(&corrected_call).unwrap(),
    )
    .unwrap();
    let corrected =
        done(features::memory::call_tool(&ctx, &corrected_call).unwrap())["fact"].clone();
    assert_eq!(
        corrected["provenance"]["entry_id"],
        format!("{}:{}", run.run_id, correction.seq)
    );
    assert_eq!(corrected["provenance"]["source"], "model_inferred");
    tx.commit().unwrap();
    drop(app);
    let mut app = db.open();
    assert_eq!(list(&mut app, "grokbot"), vec![corrected.clone()]);
    let fact = facts::get(&conn, corrected["fact_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(fact.provenance.conversation_id, Some(conversation));
    assert_eq!(fact.provenance.source, Source::ModelInferred);
}

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Vieillit un souvenir dans la base jetable du test (horloge réelle moins `days` jours).
fn age_fact(db: &Database, table: &str, column: &str, id: &Value, days: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let key = if table == "memory_facts" {
        "fact_id"
    } else {
        "id"
    };
    Connection::open(db.path())
        .unwrap()
        .execute(
            &format!("UPDATE {table} SET {column} = ?1 WHERE {key} = ?2"),
            params![now - days * DAY_MS, id.as_str().unwrap()],
        )
        .unwrap();
}
fn entry(app: &mut App, id: &Value) -> Value {
    let all = app.api("GET", "/api/memory/all", b"").unwrap();
    let found = all["facts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["fact_id"] == *id);
    found.cloned().unwrap_or(Value::Null)
}
fn add_private(app: &mut App, text: &str) -> Value {
    let body = json!({"bot_id": "grokbot", "text": text, "scope": "private"});
    post(app, "/api/memory/add", body)["fact"]["fact_id"].clone()
}

#[test]
fn old_memories_need_review_and_still_true_resets_them_without_changing_them() {
    let db = Database::new();
    let mut app = db.open();
    let (old, recent) = (
        add_private(&mut app, "Vieux"),
        add_private(&mut app, "Récent"),
    );
    age_fact(&db, "memory_facts", "learned_at", &old, 50);
    age_fact(&db, "memory_facts", "learned_at", &recent, 48);
    assert_eq!(entry(&mut app, &old)["needs_review"], true);
    assert_eq!(entry(&mut app, &recent)["needs_review"], false);
    let before = entry(&mut app, &old);
    for _ in 0..2 {
        // Deux appels successifs : même état, texte et id inchangés.
        let answer = post(&mut app, "/api/memory/reconfirm", json!({"fact_id": old}));
        assert_eq!(answer["fact"]["needs_review"], false);
        let after = entry(&mut app, &old);
        for key in [
            "fact_id",
            "text",
            "learned_at",
            "scope",
            "provenance",
            "bot_id",
        ] {
            assert_eq!(after[key], before[key], "{key}");
        }
        assert_eq!(after["needs_review"], false);
    }
    // Dérivé à la lecture : une reconfirmation vieille de 50 jours ne protège plus.
    age_fact(&db, "memory_facts", "reconfirmed_at", &old, 50);
    assert_eq!(entry(&mut app, &old)["needs_review"], true);
}

#[test]
fn still_true_covers_topic_memories_and_refuses_forgotten_or_unknown_ones() {
    let db = Database::new();
    let mut app = db.open();
    let topic = topic_fact(&mut app, "grokbot", "Budget validé")["id"].clone();
    age_fact(&db, "wrapper_topic_facts", "created_at", &topic, 60);
    assert_eq!(entry(&mut app, &topic)["needs_review"], true);
    post(&mut app, "/api/memory/reconfirm", json!({"fact_id": topic}));
    assert_eq!(entry(&mut app, &topic)["needs_review"], false);
    assert_eq!(entry(&mut app, &topic)["text"], "Budget validé");
    post(
        &mut app,
        "/api/topics/memory/forget",
        json!({"fact_id": topic}),
    );
    let private = add_private(&mut app, "À oublier");
    let forget = json!({"bot_id": "grokbot", "fact_id": private});
    post(&mut app, "/api/memory/forget", forget);
    for id in [topic, private, json!("fact_inconnu")] {
        let body = json!({"fact_id": id}).to_string();
        let refused = app.api("POST", "/api/memory/reconfirm", body.as_bytes());
        assert_eq!(refused.unwrap_err().status, 404, "{id}");
        assert_eq!(entry(&mut app, &id), Value::Null, "rien ne revit : {id}");
    }
}
