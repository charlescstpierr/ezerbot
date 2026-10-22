//! Routine adapter tests: injected host clock, real scheduler/engine uniqueness,
//! and conversation confirmation (Q5 / REA cron-detail.md CR19), not approval cards.
mod support;
use grokbot_engine::{contracts::*, model::Run, store};
use grokbot_scheduler::{
    occurrences::{self, CatchUpPolicy, OccurrenceStatus},
    routine::{self, Routine},
};
use grokbot_wrapper::{features, App, Provider};
use rusqlite::{params, Connection};
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

struct Fixture {
    app: App,
    path: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new(live: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "grokbot-routines-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ))
            .join("app.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        if live {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test-host","client_id":CLIENT,"access_token":"access-2","refresh_token":"refresh-2","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        }
        let provider = Provider::with_endpoints(&path, server.endpoints(), false);
        let app = App::open_with(&path, provider.clone()).unwrap();
        if live {
            wait("catalog", || provider.status()["models_ready"] == true);
        }
        Self {
            app,
            path,
            server,
            provider,
        }
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn post(&mut self, path: &str, v: Value) -> Value {
        self.app
            .api("POST", path, v.to_string().as_bytes())
            .unwrap()
    }
    fn create(&mut self, prompt: &str) -> Routine {
        serde_json::from_value(
            self.post(
                "/api/routines",
                json!({"bot_id":"grokbot","name":"Report","prompt":prompt,"schedule":"@every 1m"}),
            )["routine"]
                .clone(),
        )
        .unwrap()
    }
    fn tick_due(&mut self, r: &Routine) {
        self.app.tick((r.modified_at + 60_000) as u64);
    }
    fn history(&mut self, r: &Routine) -> Value {
        self.post(
            "/api/routines/history",
            json!({"bot_id":r.bot_id,"routine_id":r.routine_id}),
        )
    }
    fn run(&self, r: &Routine) -> Run {
        let record:String=self.db().query_row("SELECT record FROM engine_runs WHERE conversation_id=?1 ORDER BY rowid DESC LIMIT 1",[&r.conversation_id],|row|row.get(0)).unwrap();
        serde_json::from_str(&record).unwrap()
    }
    fn count(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT count(*) FROM engine_runs WHERE trigger_kind='occurrence'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn events(&self, events: Vec<Value>) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn reopen(&mut self) {
        self.app = App::open_with(&self.path, self.provider.clone()).unwrap();
    }
    fn wait_run(&self, r: &Routine) {
        wait("routine finished", || self.run(r).status.is_terminal());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}
fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"type":"response.output_item.done","item":{"id":format!("fc-{id}"),"type":"function_call","status":"completed","call_id":id,"name":name,"arguments":args.to_string()}})
}
fn complete() -> Value {
    json!({"type":"response.completed"})
}
fn output(outcome: ToolOutcome) -> Value {
    match outcome {
        ToolOutcome::Done { output } => output,
        other => panic!("{other:?}"),
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn confirm_current(f: &Fixture, conv: &str, message: &str, routine: &str) -> ToolOutcome {
    let db = f.db();
    let run = store::latest_run_for_trigger(
        &db,
        &Trigger::UserMessage {
            message_id: format!("{conv}/{message}"),
            attempt: 0,
        },
    )
    .unwrap()
    .unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(conv);
    ctx.run_id = Some(&run.run_id);
    let out = features::routines::call_tool(
        &ctx,
        &ToolCall {
            call_id: "confirm".into(),
            name: "enable_routine".into(),
            arguments: json!({"routine_id":routine,"confirm":true}),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    out
}
fn model_create(f: &Fixture, name: &str, prompt: &str) -> Value {
    output(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({"name":name,"prompt":prompt,"schedule":"@every 1m"}),
            )
            .unwrap(),
    )
}

#[test]
fn generic_consent_is_only_valid_for_the_first_user_turn_but_named_consent_can_be_later() {
    let mut f = Fixture::new(false);
    let created = model_create(&f, "Stale report", "Report");
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let id = created["routine"]["routine_id"].as_str().unwrap();
    for (msg, text) in [("unrelated", "Parle-moi de Rust"), ("ok-later", "ok")] {
        f.post(
            "/api/send",
            json!({"conversation_id":conv,"message_id":msg,"body":text}),
        );
    }
    assert!(matches!(
        confirm_current(&f, conv, "ok-later", id),
        ToolOutcome::Failed { .. }
    ));
    assert!(!routine::get(&f.db(), id).unwrap().unwrap().enabled);
    f.post("/api/send",json!({"conversation_id":conv,"message_id":"named","body":"activer la routine Stale report"}));
    assert_eq!(
        output(confirm_current(&f, conv, "named", id))["routine"]["enabled"],
        true
    );
}

#[test]
fn stale_confirmation_can_be_reasked_without_spam_and_obsolete_runs_cannot_enable() {
    let mut f = Fixture::new(false);
    let created = model_create(&f, "Reask", "Report");
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let id = created["routine"]["routine_id"].as_str().unwrap();
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"old-yes","body":"oui"}),
    );
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"new-no","body":"non"}),
    );
    assert!(matches!(
        confirm_current(&f, conv, "old-yes", id),
        ToolOutcome::Failed { .. }
    ));
    output(
        f.app
            .invoke_tool("grokbot", "enable_routine", json!({"routine_id":id}))
            .unwrap(),
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"fresh-yes","body":"oui"}),
    );
    assert_eq!(
        output(confirm_current(&f, conv, "fresh-yes", id))["routine"]["enabled"],
        true
    );
}

#[test]
fn immediate_consent_accepts_native_punctuation_and_politeness_without_model_message_ids() {
    for text in [
        "oui",
        "d’accord",
        "oui merci",
        "oui, vas-y",
        "Oui !!",
        "ok.  ",
    ] {
        let mut f = Fixture::new(false);
        let created = model_create(&f, "Report", "Report");
        let conv = created["confirmation_conversation_id"].as_str().unwrap();
        let id = created["routine"]["routine_id"].as_str().unwrap();
        f.post(
            "/api/send",
            json!({"conversation_id":conv,"message_id":"opaque-host-only-uuid","body":text}),
        );
        assert_eq!(
            output(confirm_current(&f, conv, "opaque-host-only-uuid", id))["routine"]["enabled"],
            true,
            "{text}"
        );
    }
    let tool = features::routines::tools()
        .into_iter()
        .find(|t| t.name == "enable_routine")
        .unwrap();
    assert!(tool.schema["properties"]["confirmation_message_id"].is_null());
    assert_eq!(tool.schema["properties"]["confirm"]["type"], "boolean");
}

#[test]
fn default_timezone_matches_host_and_on_demand_clock_uses_injected_time_without_permanent_context()
{
    let mut f = Fixture::new(false);
    let conv = f.post("/api/conversations", json!({"title":"Clock"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut ctx = f.app.feature_context();
    ctx.now_ms = 1_900_000_000_000;
    let request = ContextRequest {
        run_id: "preview".into(),
        trigger: Trigger::UserMessage {
            message_id: "preview".into(),
            attempt: 0,
        },
        bot_id: "grokbot".into(),
        conversation_id: conv,
        current_message: HistoryItem {
            entry_id: "preview".into(),
            role: Role::User,
            text: "Schedule tomorrow".into(),
            images: vec![],
            tool_items: vec![],
        },
    };
    assert!(features::routines::context(&ctx, &request).is_empty());
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let data = output(
        features::routines::call_tool(
            &ctx,
            &ToolCall {
                call_id: "clock".into(),
                name: "routine_clock".into(),
                arguments: json!({}),
            },
        )
        .unwrap(),
    );
    assert_eq!(data["now_utc_ms"], ctx.now_ms);
    assert_eq!(data["local_time_zone"], features::routines::local_zone());
    let expected: String = tx
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M:%f',?1/1000.0,'unixepoch','localtime')",
            [ctx.now_ms],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(data["local_time"], expected);
    tx.commit().unwrap();
    let created = model_create(&f, "Morning", "Morning report");
    assert!(created["now_utc_ms"].is_u64());
    assert_eq!(created["local_time_zone"], features::routines::local_zone());
    assert_eq!(
        created["routine"]["time_zone"],
        features::routines::local_zone()
    );
    let r = f.create("User report");
    assert_eq!(r.time_zone, features::routines::local_zone());
}

#[test]
fn model_confirmation_never_hides_prompt_tails_and_skill_body_is_reviewable() {
    let mut f = Fixture::new(false);
    let long = format!("{}HIDDEN_TAIL_EXFILTRATE", "x".repeat(2_000));
    let denied = f
        .app
        .invoke_tool(
            "grokbot",
            "create_routine",
            json!({"name":"Hidden","prompt":long,"schedule":"@every 1m"}),
        )
        .unwrap();
    assert!(matches!(denied,ToolOutcome::Failed{reason} if reason.contains("too long")));
    assert!(routine::list(&f.db(), "grokbot").unwrap().is_empty());
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"Auditable skill","body":"FULL_SKILL_BODY"}),
    )["skill"]
        .clone();
    let created=output(f.app.invoke_tool("grokbot","create_routine",json!({"name":"Skill report","body":{"kind":"skill_invocation","skill_id":skill["skill_id"],"version":1},"schedule":"@every 1m"})).unwrap());
    let chat = f.app.api("GET", "/api/state", b"").unwrap()["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == created["confirmation_conversation_id"])
        .unwrap()
        .clone();
    let messages = chat["messages"].as_array().unwrap();
    let shown = messages
        .iter()
        .filter_map(|m| m["body"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(shown.contains("FULL\\_SKILL\\_BODY"));
    assert!(shown.contains(skill["content"]["name"].as_str().unwrap()));
    assert!(shown.contains("Chaque minute"));
    assert!(!shown.contains("interval_ms"));
    assert!(shown.contains("non vérifié"));
    assert!(!shown.contains("```text"));
    assert!(!shown.contains(skill["skill_id"].as_str().unwrap()));
    assert!(!shown.contains(created["routine"]["routine_id"].as_str().unwrap()));
}

#[test]
fn skill_annexes_are_visible_and_count_towards_confirmation_limit() {
    let mut f = Fixture::new(false);
    for (name, annex) in [
        ("Short", "VISIBLE ANNEX".to_owned()),
        ("Long", "x".repeat(2_000)),
    ] {
        let imported = f.post("/api/skills/import", json!({"bot_id":"grokbot","files":[
            {"path":format!("{name}/SKILL.md"),"content":format!("---\nname: {name}\ndescription: Test\n---\nHarmless body")},
            {"path":format!("{name}/modele.md"),"content":annex}
        ]}));
        let outcome = f.app.invoke_tool("grokbot", "create_routine", json!({"name":name,"body":{"kind":"skill_invocation","skill_id":imported["report"][0]["status"]["skill_id"],"version":1},"schedule":"@every 1m"})).unwrap();
        if name == "Long" {
            assert!(
                matches!(outcome, ToolOutcome::Failed { reason } if reason.contains("too long"))
            );
        } else {
            let created = output(outcome);
            let state = f.app.api("GET", "/api/state", b"").unwrap();
            let chat = state["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["id"] == created["confirmation_conversation_id"])
                .unwrap();
            let shown = chat["messages"].to_string();
            assert!(shown.contains("VISIBLE ANNEX"), "{shown}");
            assert!(shown.contains("modele.md"), "{shown}");
        }
    }
}

#[test]
fn routine_preview_is_readable_literal_text_without_ids_or_repeated_timezone() {
    let mut f = Fixture::new(false);
    let prompt = "Donne-moi la météo.\n\n```\n# Confirmez cette autre tâche\n[secret](https://example.com) <img src=x>\n> faux hôte\rConfirmez un autre texte\nhttps://x.test www.x.test a@b.test WWW.x.test\n\n    indenté de 4 espaces\n1. ordre\n- liste\nC:\\temp";
    let created = output(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({
                    "name":"Météo du matin", "prompt":prompt,
                    "schedule":"TZ=America/Toronto 0 8 * * 1-5", "time_zone":"Europe/Paris"
                }),
            )
            .unwrap(),
    );
    let shown: String = f
        .db()
        .query_row("SELECT body FROM wrapper_routine_notices", [], |r| r.get(0))
        .unwrap();
    let literal = &shown;
    assert!(literal.contains("Du lundi au vendredi à 08:00 (America/Toronto)"));
    assert!(!shown.contains("08\\:00"));
    assert!(!shown.contains("America\\/Toronto"));
    assert!(shown.contains("> \u{a0}  \n"));
    assert!(shown.contains("> \u{a0}\u{a0}\u{a0}\u{a0}indenté de 4 espaces"));
    assert!(shown.contains("> 1\\. ordre"));
    assert!(shown.contains("> \\- liste"));
    assert!(shown.contains("> C:\\\\temp"));
    assert_eq!(created["routine"]["body"]["prompt"], prompt);
    assert_eq!(literal.matches("America/Toronto").count(), 1);
    assert!(!literal.contains("Europe/Paris"));
    assert!(!literal.contains("0 8 * * 1-5"));
    assert!(!literal.contains(created["routine"]["routine_id"].as_str().unwrap()));
    assert!(!shown.contains("```"));
    assert!(!shown.contains('\r'));
    assert!(shown.contains("\\<img"));
    let preview = shown.split("\n\n").nth(1).unwrap();
    assert!(preview.lines().all(|line| line.starts_with("> ")));
    assert!(shown.contains("> **Routine :**\n> Météo du matin"));
    assert!(shown.contains("> **Horaire :**\n> Du lundi au vendredi"));
    assert!(literal.contains("> **Tâche proposée :**\n> Donne-moi la météo."));
    for link in ["https://x.test", "www.x.test", "a@b.test", "WWW.x.test"] {
        assert!(!literal.contains(link), "active autolink: {link}");
    }
    assert!(literal.contains("https:\u{2060}//x.test"));
    assert!(literal.contains("www\u{2060}.x.test"));
    assert!(literal.contains("a\u{2060}@b.test"));
    assert!(shown.contains("\\[secret\\](https:\u{2060}//example.com) \\<img src=x\\>"));
    assert!(shown.contains("\n\nConfirmez-vous son activation ?"));
    let list = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    assert_eq!(
        list["routines"][0]["schedule_description"],
        "Du lundi au vendredi à 08:00 (America/Toronto)"
    );
}

#[test]
fn same_named_routines_show_local_seconds_and_distinct_references_in_panel_metadata_and_chat() {
    let mut f = Fixture::new(false);
    let create_at = |at| {
        let db = f.db();
        let tx = features::write_transaction(&db).unwrap();
        let mut ctx = f.app.feature_context();
        ctx.connection = &tx;
        ctx.bot_id = Some("grokbot");
        ctx.now_ms = at;
        let created = output(features::routines::call_tool(&ctx, &ToolCall {
            call_id: "create".into(),
            name: "create_routine".into(),
            arguments: json!({"name":"Météo du matin","prompt":"Météo","schedule":"@every 1m","time_zone":"Pacific/Honolulu"}),
        }).unwrap());
        tx.commit().unwrap();
        created
    };
    // Same second: dropping milliseconds must not make homonyms identical.
    let a = create_at(1_792_000_000_123);
    let b = create_at(1_792_000_000_124);
    assert_ne!(a["routine"]["review_stamp"], b["routine"]["review_stamp"]);
    let notices: String = f
        .db()
        .query_row(
            "SELECT group_concat(body, char(10)) FROM wrapper_routine_notices",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let list = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    for item in [&a, &b] {
        let stamp = item["routine"]["review_stamp"].as_str().unwrap();
        let (date, reference) = stamp.split_once(" · Réf. ").unwrap();
        let expected: String = f.db().query_row(
            "SELECT strftime('Modifiée le %d/%m/%Y à %H:%M:%S',?1/1000.0,'unixepoch','localtime')",
            [item["routine"]["modified_at"].as_i64().unwrap()],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(date, expected);
        assert!(!date.contains('.'));
        assert!(!stamp.contains("UTC"));
        assert!(reference.len() >= 8 && reference.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(notices.contains(stamp));
        assert!(list["routines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["review_stamp"] == stamp));
    }
}

#[test]
fn routine_review_references_extend_colliding_id_prefixes_and_survive_deletion() {
    let mut f = Fixture::new(false);
    let mut a = f.create("Report");
    a.routine_id = "routine-12345678aaaaaaaaaaaaaaaaaaaaaaaa".into();
    a.conversation_id = "collision-a".into();
    routine::create(&f.db(), &a).unwrap();
    let mut b = a.clone();
    b.routine_id = "routine-12345678bbbbbbbbbbbbbbbbbbbbbbbb".into();
    b.conversation_id = "collision-b".into();
    routine::create(&f.db(), &b).unwrap();
    let list = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    for (id, reference) in [(&a.routine_id, "12345678a"), (&b.routine_id, "12345678b")] {
        let stamp = list["routines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["routine_id"] == *id)
            .unwrap()["review_stamp"]
            .as_str()
            .unwrap();
        assert!(stamp.ends_with(&format!(" · Réf. {reference}")));
    }
    routine::delete(&f.db(), &a.routine_id).unwrap();
    f.reopen();
    let list = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    let stamp = list["routines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["routine_id"] == b.routine_id)
        .unwrap()["review_stamp"]
        .as_str()
        .unwrap();
    assert!(stamp.ends_with(" · Réf. 12345678b"));
}

#[test]
fn model_edit_provenance_is_exposed_and_human_edit_clears_it() {
    let mut f = Fixture::new(false);
    let r = f.create("User authored");
    let edited=output(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"prompt":"Model authored"})).unwrap())["routine"].clone();
    assert!(edited["model_edited_at"].is_i64());
    let list = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    assert!(list["routines"][0]["model_edited_at"].is_i64());
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["routines"]["items"][0]["routine"]["model_edited_at"].is_i64());
    let human=f.post("/api/routines/update",json!({"bot_id":"grokbot","routine_id":r.routine_id,"modified_at":edited["modified_at"],"schedule_version":edited["schedule_version"],"prompt":"Reviewed by user"}));
    assert!(human["routine"]["model_edited_at"].is_null());
}

#[test]
fn pending_questions_are_reused_bounded_and_revision_changes_require_a_later_turn() {
    let mut f = Fixture::new(false);
    let created = model_create(&f, "One", "Report");
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let mut r = created["routine"].clone();
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"old-consent","body":"oui"}),
    );
    for i in 0..5 {
        r=output(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r["routine_id"],"modified_at":r["modified_at"],"schedule_version":r["schedule_version"],"name":format!("One {i}")})).unwrap())["routine"].clone();
    }
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert!(matches!(
        confirm_current(&f, conv, "old-consent", r["routine_id"].as_str().unwrap()),
        ToolOutcome::Failed { .. }
    ));
    for i in 0..4 {
        model_create(&f, &format!("Question {i}"), "Report");
    }
    assert!(matches!(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({"name":"Overflow","prompt":"Report","schedule":"@every 1m"})
            )
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(routine::list(&f.db(), "grokbot").unwrap().len(), 5);
}

#[test]
fn deleted_bots_are_paused_after_one_deterministic_failure() {
    let mut f = Fixture::new(false);
    f.post(
        "/api/bots",
        json!({"id":"gone","name":"Gone","color":"citron","description":"","instructions":""}),
    );
    let r: Routine = serde_json::from_value(
        f.post(
            "/api/routines",
            json!({"bot_id":"gone","name":"Report","prompt":"Report","schedule":"@every 1m"}),
        )["routine"]
            .clone(),
    )
    .unwrap();
    f.post("/api/bots/delete", json!({"id":"gone"}));
    for i in 1..=5 {
        f.app.tick((r.modified_at + i * 60_000) as u64);
    }
    assert!(
        !routine::get(&f.db(), &r.routine_id)
            .unwrap()
            .unwrap()
            .enabled
    );
    assert_eq!(occurrences::list(&f.db(), &r.routine_id).unwrap().len(), 1);
}

#[test]
fn deleted_questions_leave_context_and_deleted_chats_get_fresh_questions() {
    let mut f = Fixture::new(false);
    let created = model_create(&f, "Ghost", "GHOST_PROMPT");
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let id = created["routine"]["routine_id"].as_str().unwrap();
    f.post("/api/conversations/delete", json!({"id":conv}));
    let again = output(
        f.app
            .invoke_tool("grokbot", "enable_routine", json!({"routine_id":id}))
            .unwrap(),
    );
    assert_ne!(again["confirmation_conversation_id"], conv);
    let new_conv = again["confirmation_conversation_id"].as_str().unwrap();
    f.post(
        "/api/routines/delete",
        json!({"bot_id":"grokbot","routine_id":id}),
    );
    assert!(!f
        .app
        .context_preview(new_conv)
        .unwrap()
        .iter()
        .any(|b| b.text.contains("GHOST_PROMPT")));
}

#[test]
fn transient_engine_creation_errors_keep_occurrence_retryable_and_get_reads_do_not_take_write_lock()
{
    let mut f = Fixture::new(false);
    let r = f.create("Retry");
    let db = f.db();
    db.execute_batch("CREATE TRIGGER fail_engine_insert BEFORE INSERT ON wrapper_engine_runs BEGIN SELECT RAISE(ABORT,'transient hook/storage failure'); END").unwrap();
    f.tick_due(&r);
    assert_eq!(
        occurrences::list(&db, &r.routine_id).unwrap()[0].status,
        OccurrenceStatus::Pending
    );
    assert!(routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    db.execute_batch("DROP TRIGGER fail_engine_insert").unwrap();
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    f.app
        .feature_context()
        .connection
        .busy_timeout(Duration::from_millis(30))
        .unwrap();
    assert!(f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .is_ok());
    assert!(f
        .app
        .api(
            "GET",
            &format!(
                "/api/routines/history?bot_id=grokbot&routine_id={}",
                r.routine_id
            ),
            b""
        )
        .is_ok());
    db.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn principal_ancestor_routing_matches_forms_instead_of_latest_other_chat() {
    let mut f = Fixture::new(false);
    let parent = f.post("/api/conversations", json!({"title":"Parent"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let other = f.post("/api/conversations", json!({"title":"Other principal"}))["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.post(
        "/api/bots",
        json!({"id":"child","name":"Child","color":"citron","description":"","instructions":""}),
    );
    let db = f.db();
    db.execute(
        "UPDATE wrapper_conversations SET updated_at=updated_at+10000 WHERE id=?1",
        [&other],
    )
    .unwrap();
    db.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) SELECT account_id,'child-task','Child task','child','task',id,created_at,updated_at FROM wrapper_conversations WHERE id=?1",[&parent]).unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("child");
    ctx.conversation_id = Some("child-task");
    let result = output(
        features::routines::call_tool(
            &ctx,
            &ToolCall {
                call_id: "child-create".into(),
                name: "create_routine".into(),
                arguments: json!({"name":"Child report","prompt":"Report","schedule":"@every 1m"}),
            },
        )
        .unwrap(),
    );
    assert_eq!(result["confirmation_conversation_id"], parent);
    tx.commit().unwrap();
}

#[test]
fn automatic_occurrences_do_not_overlap_an_active_run_in_the_routine_thread() {
    let mut f = Fixture::new(false);
    let r = f.create("Sequential");
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    grokbot_engine::model::start(
        &tx,
        &Run {
            run_id: "active-run".into(),
            bot_id: r.bot_id.clone(),
            conversation_id: r.conversation_id.clone(),
            trigger: Trigger::Delegation {
                delegation_id: "blocker".into(),
            },
            kind: RunKind::Task,
            status: grokbot_engine::model::RunStatus::Running,
            task_status: Some(TaskStatus::Working),
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: r.modified_at,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let mut ctx = f.app.feature_context();
    ctx.now_ms = (r.modified_at + 60_000) as u64;
    features::routines::tick(&ctx);
    assert_eq!(f.count(), 0);
    assert_eq!(
        occurrences::list(&db, &r.routine_id).unwrap()[0].status,
        OccurrenceStatus::Pending
    );
    let tx = features::write_transaction(&db).unwrap();
    let deferred: String = tx
        .query_row(
            "SELECT error FROM wrapper_routine_errors WHERE routine_id=?1",
            [&r.routine_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(deferred.contains("reportée"));
    grokbot_engine::model::cancel(&tx, "active-run", TurnEnd::Stopped).unwrap();
    tx.commit().unwrap();
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM wrapper_routine_errors WHERE routine_id=?1",
            [&r.routine_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn long_model_edits_fail_atomically_without_changing_a_trusted_active_routine() {
    let mut f = Fixture::new(false);
    let r = f.create("Original");
    let result=f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"prompt":"x".repeat(2_001)})).unwrap();
    assert!(matches!(result, ToolOutcome::Failed { .. }));
    assert_eq!(routine::get(&f.db(), &r.routine_id).unwrap().unwrap(), r);
    assert_eq!(
        f.db()
            .query_row(
                "SELECT count(*) FROM wrapper_routine_provenance",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

fn context_request(conversation: &str) -> ContextRequest {
    ContextRequest {
        run_id: "preview".into(),
        trigger: Trigger::UserMessage {
            message_id: format!("{conversation}/current"),
            attempt: 0,
        },
        bot_id: "grokbot".into(),
        conversation_id: conversation.into(),
        current_message: HistoryItem {
            entry_id: "current".into(),
            role: Role::User,
            text: "What does the image show?".into(),
            images: vec![],
            tool_items: vec![],
        },
    }
}

#[test]
fn routine_context_is_relevant_stable_and_does_not_reduce_historical_caption_budget() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    let request = context_request(&r.conversation_id);
    let mut ctx = f.app.feature_context();
    let blocks = features::routines::context(&ctx, &request);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].budget, 16_000);
    assert!(blocks[0].priority > features::priority::HISTORY);
    assert!(!blocks[0].text.contains("now_utc_ms"));
    ctx.now_ms += 999_999;
    assert_eq!(
        features::routines::context(&ctx, &request)[0].text,
        blocks[0].text
    );
    let app = ContextBlock {
        source: ContextSource::AppContract,
        priority: 0,
        budget: 10_000,
        text: "policy".into(),
    };
    let caption = ContextBlock {
        source: ContextSource::ImageCaption,
        priority: 90,
        budget: 16_000,
        text: format!(
            "<untrusted-image-captions>\n{}\n</untrusted-image-captions>",
            "c".repeat(12_000)
        ),
    };
    let before = grokbot_engine::context::assemble_input(
        &request,
        &[app.clone(), caption.clone()],
        &[],
        None,
        None,
        &ContextBudget::default(),
    )
    .unwrap();
    let after = grokbot_engine::context::assemble_input(
        &request,
        &[app, caption, blocks[0].clone()],
        &[],
        None,
        None,
        &ContextBudget::default(),
    )
    .unwrap();
    let before = before
        .blocks
        .iter()
        .find(|b| b.source == ContextSource::ImageCaption)
        .unwrap();
    let after = after
        .blocks
        .iter()
        .find(|b| b.source == ContextSource::ImageCaption)
        .unwrap();
    assert_eq!(before.text, after.text);
    assert!(!after
        .text
        .contains(grokbot_engine::context::TRUNCATION_MARKER));
}

#[test]
fn named_consent_requires_unambiguous_pending_name_or_the_exact_routine_id() {
    let mut f = Fixture::new(false);
    let a = model_create(&f, "Rapport!", "Legitimate report");
    let b = model_create(&f, "rapport,", "Different report");
    let conv = a["confirmation_conversation_id"].as_str().unwrap();
    let aid = a["routine"]["routine_id"].as_str().unwrap();
    let bid = b["routine"]["routine_id"].as_str().unwrap();
    f.post("/api/send",json!({"conversation_id":conv,"message_id":"ambiguous","body":"activer la routine Rapport"}));
    for id in [aid, bid] {
        assert!(matches!(
            confirm_current(&f, conv, "ambiguous", id),
            ToolOutcome::Failed { .. }
        ));
        assert!(!routine::get(&f.db(), id).unwrap().unwrap().enabled);
    }
    f.post("/api/send",json!({"conversation_id":conv,"message_id":"exact","body":format!("activer la routine {aid}")}));
    assert!(matches!(
        confirm_current(&f, conv, "exact", bid),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(
        output(confirm_current(&f, conv, "exact", aid))["routine"]["enabled"],
        true
    );
    assert!(!routine::get(&f.db(), bid).unwrap().unwrap().enabled);
    assert_eq!(
        output(confirm_current(&f, conv, "exact", aid))["routine"]["enabled"],
        true
    ); // idempotent same intent
}

#[test]
fn one_generic_yes_cannot_fall_through_to_multiple_pending_routines() {
    let mut f = Fixture::new(false);
    let a = model_create(&f, "Older", "A");
    let b = model_create(&f, "Newer", "B");
    let conv = a["confirmation_conversation_id"].as_str().unwrap();
    let aid = a["routine"]["routine_id"].as_str().unwrap();
    let bid = b["routine"]["routine_id"].as_str().unwrap();
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"single-yes","body":"oui"}),
    );
    assert_eq!(
        output(confirm_current(&f, conv, "single-yes", bid))["routine"]["enabled"],
        true
    );
    assert!(matches!(
        confirm_current(&f, conv, "single-yes", aid),
        ToolOutcome::Failed { .. }
    ));
    assert!(!routine::get(&f.db(), aid).unwrap().unwrap().enabled);
    // Upgrade/reopen reconstructs durable consumption from pre-receipt resolved
    // questions, so the same old user message cannot authorize another routine.
    f.db()
        .execute("DELETE FROM wrapper_routine_consents", [])
        .unwrap();
    f.reopen();
    assert!(matches!(
        confirm_current(&f, conv, "single-yes", aid),
        ToolOutcome::Failed { .. }
    ));
}

#[test]
fn a_routine_name_cannot_impersonate_another_routines_exact_id() {
    let mut f = Fixture::new(false);
    let a = model_create(&f, "Legitimate", "A");
    let aid = a["routine"]["routine_id"].as_str().unwrap();
    let impostor = model_create(&f, aid, "Other body");
    let conv = a["confirmation_conversation_id"].as_str().unwrap();
    let iid = impostor["routine"]["routine_id"].as_str().unwrap();
    f.post("/api/send",json!({"conversation_id":conv,"message_id":"exact-id","body":format!("activer la routine {aid}")}));
    assert!(matches!(
        confirm_current(&f, conv, "exact-id", iid),
        ToolOutcome::Failed { .. }
    ));
    assert_eq!(
        output(confirm_current(&f, conv, "exact-id", aid))["routine"]["enabled"],
        true
    );
}

#[test]
fn cancelled_newer_questions_do_not_block_generic_consent_for_the_latest_pending_question() {
    let mut f = Fixture::new(false);
    let a = model_create(&f, "Remaining", "A");
    let b = model_create(&f, "Cancelled", "B");
    let conv = a["confirmation_conversation_id"].as_str().unwrap();
    let aid = a["routine"]["routine_id"].as_str().unwrap();
    f.post(
        "/api/routines/delete",
        json!({"bot_id":"grokbot","routine_id":b["routine"]["routine_id"]}),
    );
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"yes-remaining","body":"oui"}),
    );
    assert_eq!(
        output(confirm_current(&f, conv, "yes-remaining", aid))["routine"]["enabled"],
        true
    );
}

#[test]
fn panel_edits_and_resume_close_visible_confirmation_questions() {
    let mut f = Fixture::new(false);
    let a = model_create(&f, "Panel edit", "A");
    let r = &a["routine"];
    f.post("/api/routines/update",json!({"bot_id":"grokbot","routine_id":r["routine_id"],"modified_at":r["modified_at"],"schedule_version":r["schedule_version"],"prompt":"User edit"}));
    let b = model_create(&f, "Panel resume", "B");
    f.post(
        "/api/routines/resume",
        json!({"bot_id":"grokbot","routine_id":b["routine"]["routine_id"]}),
    );
    let db = f.db();
    let texts = db
        .prepare("SELECT body FROM wrapper_routine_notices")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(texts.len(), 2);
    assert!(texts
        .iter()
        .all(|t| t.contains("Demande close") && !t.contains("Confirmez-vous")));
}

#[test]
fn one_shot_consent_shows_the_host_local_datetime_and_explicit_utc() {
    let f = Fixture::new(false);
    let at = now() + 86_400_000;
    let result = output(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({"name":"Local once","prompt":"Reminder","trigger":{"kind":"once","at":at}}),
            )
            .unwrap(),
    );
    let expected: String = f
        .db()
        .query_row(
            "SELECT strftime('%Y-%m-%d %H:%M',?1/1000.0,'unixepoch','localtime')",
            [at],
            |row| row.get(0),
        )
        .unwrap();
    let text: String = f
        .db()
        .query_row(
            "SELECT body FROM wrapper_routine_notices LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(text.contains(&expected));
    assert!(text.contains("UTC"));
    assert!(text.contains(&features::routines::local_zone()));
    assert!(result["local_time"].is_string());
}

#[test]
fn routine_context_index_is_bounded_and_reports_the_total() {
    let mut f = Fixture::new(false);
    let mut conv = String::new();
    for _ in 0..25 {
        conv = f.create("Report").conversation_id;
    }
    let blocks = features::routines::context(&f.app.feature_context(), &context_request(&conv));
    let index: Value = serde_json::from_str(&blocks[0].text).unwrap();
    assert_eq!(index["routines_total"], 25);
    assert_eq!(index["routines"].as_array().unwrap().len(), 20);
}

#[test]
fn pausing_retires_unattached_automatic_intent_instead_of_polling_it_for_an_hour() {
    let mut f = Fixture::new(false);
    let r = f.create("Pause intent");
    let db = f.db();
    let tx = features::write_transaction(&db).unwrap();
    let o = occurrences::materialize_due(&tx, r.modified_at + 60_000, &CatchUpPolicy::default())
        .unwrap()
        .remove(0);
    tx.commit().unwrap();
    f.post(
        "/api/routines/pause",
        json!({"bot_id":"grokbot","routine_id":r.routine_id}),
    );
    let mut ctx = f.app.feature_context();
    ctx.now_ms = (r.modified_at + 60_000) as u64;
    features::routines::tick(&ctx);
    assert!(
        matches!(occurrences::get(&db,&o.occurrence_id).unwrap().unwrap().status,OccurrenceStatus::Failed{reason} if reason.contains("paused"))
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM scheduler_occurrences WHERE status IN ('pending','running')",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn due_occurrence_starts_exactly_one_run() {
    let mut f = Fixture::new(false);
    let r = f.create("Write the daily report");
    f.app.tick((r.modified_at + 59_999) as u64);
    assert_eq!(f.count(), 0);
    f.tick_due(&r);
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    assert_eq!(f.run(&r).kind, RunKind::Task);
    let h = f.history(&r);
    assert_eq!(h["occurrences"].as_array().unwrap().len(), 1);
    // Simulated text ends a turn but is not an explicit successful task.
    assert_eq!(h["occurrences"][0]["status"]["status"], "failed");
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let c = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == r.conversation_id)
        .unwrap();
    assert_eq!(c["kind"], "routine");
    assert!(!c["messages"].as_array().unwrap().is_empty());
}

#[test]
fn restart_after_crash_does_not_duplicate_run() {
    let mut f = Fixture::new(false);
    let r = f.create("Crash-safe report");
    let db = f.db();
    let tx = db.unchecked_transaction().unwrap();
    let o = occurrences::materialize_due(&tx, r.modified_at + 60_000, &CatchUpPolicy::default())
        .unwrap()
        .remove(0);
    tx.commit().unwrap();
    f.reopen(); // crash after materializing the outbox, before creating the run
    f.tick_due(&r);
    let first = f.run(&r);
    assert_eq!(f.count(), 1);
    // Crash after engine creation but before scheduler attachment: reset only the
    // attachment to simulate the boundary, keeping the real durable run/events.
    db.execute("UPDATE scheduler_occurrences SET status='pending',run_id=NULL,reason=NULL WHERE occurrence_id=?1",[&o.occurrence_id]).unwrap();
    f.reopen();
    f.app.tick((r.modified_at + 61 * 60_000) as u64); // old attachment exceeds late window
    assert_eq!(
        store::latest_run_for_trigger(
            &db,
            &Trigger::Occurrence {
                occurrence_id: o.occurrence_id.clone()
            }
        )
        .unwrap()
        .unwrap()
        .run_id,
        first.run_id
    );
    let old = occurrences::get(&db, &o.occurrence_id).unwrap().unwrap();
    assert!(matches!(old.status, OccurrenceStatus::Failed { .. }));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM engine_runs WHERE trigger_ref=?1",
            [&o.occurrence_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn routine_without_skill_runs_inline_prompt() {
    let mut f = Fixture::new(true);
    let r = f.create("INLINE_ROUTINE_PAYLOAD");
    f.events(vec![
        call("done", "complete_task", json!({"summary":"Report ready"})),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.tick_due(&r);
    f.wait_run(&r);
    assert_eq!(
        f.run(&r).task_status,
        Some(TaskStatus::Done {
            summary: "Report ready".into()
        })
    );
    let history = f.history(&r);
    assert_eq!(history["occurrences"][0]["status"]["status"], "done");
    assert_eq!(
        history["occurrences"][0]["conversation_id"],
        r.conversation_id
    );
    assert_eq!(
        history["occurrences"][0]["message_id"],
        format!("reply-{}", f.run(&r).run_id)
    );
    let seen = f.server.seen.lock().unwrap();
    assert!(seen
        .bodies
        .iter()
        .any(|b| b.to_string().contains("INLINE_ROUTINE_PAYLOAD")));
}

#[test]
fn skill_invocation_uses_pinned_version() {
    let mut f = Fixture::new(true);
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"grokbot","name":"Pinned","body":"PINNED_V1_DATA"}),
    )["skill"]
        .clone();
    let id = skill["skill_id"].as_str().unwrap();
    f.post(
        "/api/skills/update",
        json!({"bot_id":"grokbot","skill_id":id,"expected_version":1,"body":"NEW_V2_NOT_FOR_ROUTINE"}),
    );
    let r:Routine=serde_json::from_value(f.post("/api/routines",json!({"bot_id":"grokbot","name":"Pinned report","body":{"kind":"skill_invocation","skill_id":id,"version":1},"schedule":"@every 1m"}))["routine"].clone()).unwrap();
    f.events(vec![
        call(
            "done",
            "complete_task",
            json!({"summary":"Pinned report ready"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.tick_due(&r);
    f.wait_run(&r);
    let seen = f.server.seen.lock().unwrap();
    let text = seen.bodies.iter().map(Value::to_string).collect::<String>();
    assert!(text.contains("PINNED_V1_DATA"));
    // The normal skill index contains names/descriptions, never the V2 body.
    assert!(!text.contains("NEW_V2_NOT_FOR_ROUTINE"));
    assert!(text.contains("routine-skill-data"));
}

#[test]
fn model_enable_requires_approval() {
    // Historical plan test name; Q5 steering replaces a separate approval gate
    // with a real user confirmation in the principal conversation (CR19).
    let mut f = Fixture::new(false);
    let created = output(
        f.app
            .invoke_tool(
                "grokbot",
                "create_routine",
                json!({"name":"Model report","prompt":"Report","schedule":"@every 1m"}),
            )
            .unwrap(),
    );
    let r: Routine = serde_json::from_value(created["routine"].clone()).unwrap();
    assert!(!r.enabled);
    assert!(r.untrusted);
    assert_eq!(created["confirmation_required"], true);
    assert!(matches!(
        f.app
            .invoke_tool(
                "grokbot",
                "enable_routine",
                json!({"routine_id":r.routine_id,"confirmation_message_id":"invented"})
            )
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT count(*) FROM forms_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let conv = created["confirmation_conversation_id"].as_str().unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let chat = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert!(chat["messages"].to_string().contains("Confirmez-vous"));
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"confirm-1","body":"oui"}),
    );
    let user_trigger = Trigger::UserMessage {
        message_id: format!("{conv}/confirm-1"),
        attempt: 0,
    };
    let run = store::latest_run_for_trigger(&db, &user_trigger)
        .unwrap()
        .unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    ctx.conversation_id = Some(conv);
    ctx.run_id = Some(&run.run_id);
    let tool = ToolCall {
        call_id: "enable".into(),
        name: "enable_routine".into(),
        arguments: json!({"routine_id":r.routine_id,"confirm":true}),
    };
    assert!(
        output(features::routines::call_tool(&ctx, &tool).unwrap())["routine"]["enabled"] == true
    );
    tx.commit().unwrap();
    assert!(routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    assert!(
        !features::routines::tools()
            .iter()
            .find(|t| t.name == "enable_routine")
            .unwrap()
            .requires_approval
    );
}

#[test]
fn conversational_confirmation_enables_through_real_model_tool_loop() {
    let mut f = Fixture::new(true);
    let conv = f.post("/api/conversations", json!({"title":"Principal"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.events(vec![
        call(
            "create",
            "create_routine",
            json!({"name":"Confirmed live","prompt":"Report","schedule":"@every 1m"}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"request","body":"Create a daily report"}),
    );
    let db = f.db();
    wait("created chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/request"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    let r = routine::list(&db, "grokbot").unwrap().remove(0);
    assert!(!r.enabled);
    f.events(vec![
        call(
            "activate",
            "enable_routine",
            json!({"routine_id":r.routine_id,"confirm":true}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"denied","body":"non"}),
    );
    wait("denied chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/denied"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    assert!(!routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    f.events(vec![
        call(
            "activate",
            "enable_routine",
            json!({"routine_id":r.routine_id,"confirm":true}),
        ),
        complete(),
    ]);
    f.events(vec![complete()]);
    f.post(
        "/api/send",
        json!({"conversation_id":conv,"message_id":"accepted","body":"activer la routine Confirmed live"}),
    );
    wait("accepted chat finished", || {
        store::latest_run_for_trigger(
            &db,
            &Trigger::UserMessage {
                message_id: format!("{conv}/accepted"),
                attempt: 0,
            },
        )
        .unwrap()
        .is_some_and(|r| r.status.is_terminal())
    });
    assert!(routine::get(&db, &r.routine_id).unwrap().unwrap().enabled);
    let seen = f.server.seen.lock().unwrap();
    // The scripted activation needs only an ID actually present in model input
    // and confirm:true. No invisible current-message ID is ever supplied.
    let bodies = seen.bodies.iter().map(Value::to_string).collect::<String>();
    assert!(bodies.contains(&r.routine_id));
    assert!(!bodies.contains("confirmation_message_id"));
    assert!(!bodies.contains("\"accepted\""));
    drop(seen);
    assert_eq!(
        db.query_row("SELECT count(*) FROM forms_requests", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn pause_resume_manual_nonce_edit_delete_and_history() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    let id = json!({"bot_id":"grokbot","routine_id":r.routine_id});
    f.post("/api/routines/pause", id.clone());
    f.tick_due(&r);
    assert_eq!(f.count(), 0);
    let manual = json!({"bot_id":"grokbot","routine_id":r.routine_id,"nonce":"click-1"});
    let a = f.post("/api/routines/run-now", manual.clone());
    let b = f.post("/api/routines/run-now", manual);
    assert_eq!(a, b);
    f.tick_due(&r);
    assert_eq!(f.count(), 1);
    let enabled = f.post("/api/routines/resume", id.clone())["routine"].clone();
    assert_eq!(enabled["enabled"], true);
    let edited=f.post("/api/routines/update",json!({"bot_id":"grokbot","routine_id":r.routine_id,"modified_at":enabled["modified_at"],"schedule_version":enabled["schedule_version"],"name":"Renamed","schedule":"@every 2m"}))["routine"].clone();
    assert_eq!(edited["schedule_version"], 2);
    let listing = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    assert_eq!(listing["routines"][0]["name"], "Renamed");
    let history = f
        .app
        .api(
            "GET",
            &format!(
                "/api/routines/history?bot_id=grokbot&routine_id={}",
                r.routine_id
            ),
            b"",
        )
        .unwrap();
    assert!(history["missed_ranges"].is_array());
    f.post("/api/routines/delete", id);
    f.app.tick(now() + 86_400_000);
    assert_eq!(f.count(), 1);
}

#[test]
fn ownership_and_forged_fields_are_rejected() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    f.post(
        "/api/bots",
        json!({"id":"other","name":"Other","color":"citron","description":"","instructions":""}),
    );
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/routines/pause",
                json!({"bot_id":"other","routine_id":r.routine_id})
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        404
    );
    assert!(f.app.api("POST","/api/routines",json!({"bot_id":"grokbot","name":"Forged","prompt":"X","schedule":"@every 1m","modified_at":0,"origin":"user"}).to_string().as_bytes()).is_err());
    assert!(matches!(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":1,"enabled":true})).unwrap(),ToolOutcome::Failed{..}));
    let skill = f.post(
        "/api/skills",
        json!({"bot_id":"other","name":"Private","body":"private"}),
    )["skill"]
        .clone();
    assert_eq!(f.app.api("POST","/api/routines",json!({"bot_id":"grokbot","name":"Leak","schedule":"@every 1m","body":{"kind":"skill_invocation","skill_id":skill["skill_id"],"version":1}}).to_string().as_bytes()).unwrap_err().status,404);
}

#[test]
fn completion_hook_is_idempotent_and_failure_never_revives() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    f.tick_due(&r);
    let db = f.db();
    let run = f.run(&r);
    let events = store::events(&db, &run.run_id, 0).unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    for e in events.iter().rev() {
        features::routines::on_run_event(&ctx, e);
    }
    tx.commit().unwrap();
    assert_eq!(
        f.history(&r)["occurrences"][0]["status"]["status"],
        "failed"
    );
}

#[test]
fn model_update_pauses_and_invalidates_confirmation() {
    let mut f = Fixture::new(false);
    let r = f.create("Original");
    assert!(r.enabled);
    let edit=output(f.app.invoke_tool("grokbot","update_routine",json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"prompt":"Changed by model"})).unwrap());
    assert_eq!(edit["routine"]["enabled"], false);
    assert_eq!(edit["confirmation_required"], true);
    f.tick_due(&r);
    assert_eq!(f.count(), 0);
}

#[test]
fn paused_user_routine_can_request_fresh_conversational_confirmation() {
    let mut f = Fixture::new(false);
    let r = f.create("User report");
    f.post(
        "/api/routines/pause",
        json!({"bot_id":"grokbot","routine_id":r.routine_id}),
    );
    let requested = output(
        f.app
            .invoke_tool(
                "grokbot",
                "enable_routine",
                json!({"routine_id":r.routine_id}),
            )
            .unwrap(),
    );
    assert_eq!(requested["confirmation_required"], true);
    assert_eq!(requested["routine"]["enabled"], false);
    let before: i64 = f
        .db()
        .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |r| {
            r.get(0)
        })
        .unwrap();
    // Repeated requests reuse the same pending question instead of spamming chat.
    f.app
        .invoke_tool(
            "grokbot",
            "enable_routine",
            json!({"routine_id":r.routine_id}),
        )
        .unwrap();
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM wrapper_routine_notices", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn subbot_confirmation_is_visible_only_in_principal_chat_and_context() {
    let mut f = Fixture::new(false);
    f.post(
        "/api/bots",
        json!({"id":"child","name":"Child","color":"citron","description":"","instructions":""}),
    );
    let requested = output(
        f.app
            .invoke_tool(
                "child",
                "create_routine",
                json!({"name":"Child report","prompt":"Child task","schedule":"@every 1m"}),
            )
            .unwrap(),
    );
    let conv = requested["confirmation_conversation_id"].as_str().unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let c = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == conv)
        .unwrap();
    assert_eq!(c["bot_id"], "grokbot");
    assert_eq!(c["kind"], "chat");
    assert!(c["messages"].to_string().contains("Child task"));
    assert!(f
        .app
        .context_preview(conv)
        .unwrap()
        .iter()
        .any(|b| b.text.contains("Child report")));
    assert!(matches!(
        f.app
            .invoke_tool(
                "child",
                "enable_routine",
                json!({"routine_id":requested["routine"]["routine_id"],"confirm":true})
            )
            .unwrap(),
        ToolOutcome::Failed { .. }
    ));
}

#[test]
fn failing_tool_rolls_back_partial_effects_inside_engine_transaction() {
    let f = Fixture::new(false);
    let db = f.db();
    db.execute_batch("CREATE TRIGGER reject_confirmation BEFORE INSERT ON wrapper_routine_notices BEGIN SELECT RAISE(ABORT,'test storage failure'); END").unwrap();
    let tx = features::write_transaction(&db).unwrap();
    let mut ctx = f.app.feature_context();
    ctx.connection = &tx;
    ctx.bot_id = Some("grokbot");
    let result = features::routines::call_tool(
        &ctx,
        &ToolCall {
            call_id: "create".into(),
            name: "create_routine".into(),
            arguments: json!({"name":"Atomic","prompt":"Test","schedule":"@every 1m"}),
        },
    )
    .unwrap();
    assert!(matches!(result, ToolOutcome::Failed { .. }));
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM scheduler_routines", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM wrapper_conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn history_pages_are_bounded_stable_and_persist_across_restart() {
    let mut f = Fixture::new(false);
    let r = f.create("Paged report");
    let db = f.db();
    for n in 0..67 {
        // Equal timestamps force the ID tie-breaker; insertion order differs.
        db.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,manual_nonce) VALUES (?1,?2,1,?3,'missed',?1)",params![format!("page-{n:03}"),r.routine_id,n/3]).unwrap();
        db.execute(
            "INSERT INTO scheduler_missed_ranges VALUES (?1,1,?2,?3,?4,'UTC')",
            params![
                r.routine_id,
                n * 100,
                n * 100 + 50,
                json!({"kind":"every","interval_ms":60000,"phase_ms":null}).to_string()
            ],
        )
        .unwrap();
    }
    let first = f.history(&r);
    assert_eq!(first["occurrences"].as_array().unwrap().len(), 30);
    assert_eq!(first["missed_ranges"].as_array().unwrap().len(), 30);
    let mut ids = Vec::new();
    let mut ranges = Vec::new();
    let mut page = first.clone();
    // Both newer and backdated rows must stay outside this traversal.
    for (id, at) in [("newer", 1000), ("backdated", 0)] {
        db.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,manual_nonce) VALUES (?1,?2,1,?3,'missed',?1)",params![id,r.routine_id,at]).unwrap();
    }
    db.execute(
        "INSERT INTO scheduler_missed_ranges VALUES (?1,2,0,50,?2,'UTC')",
        params![
            r.routine_id,
            json!({"kind":"every","interval_ms":60000,"phase_ms":null}).to_string()
        ],
    )
    .unwrap();
    f.reopen();
    loop {
        ids.extend(
            page["occurrences"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["occurrence_id"].as_str().unwrap().to_owned()),
        );
        ranges.extend(
            page["missed_ranges"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["from_inclusive"].as_i64().unwrap()),
        );
        if page["next_cursor"].is_null() {
            break;
        }
        page=f.post("/api/routines/history",json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"cursor":page["next_cursor"],"ranges_cursor":page["next_ranges_cursor"]}));
        assert!(page["occurrences"].as_array().unwrap().len() <= 30);
    }
    assert_eq!(ids.len(), 67);
    let mut expected = (0..67)
        .rev()
        .map(|n| format!("page-{n:03}"))
        .collect::<Vec<_>>();
    assert_eq!(ids, expected);
    expected.sort();
    ids.sort();
    ids.dedup();
    assert_eq!(ids, expected);
    assert_eq!(ranges, (0..67).rev().map(|n| n * 100).collect::<Vec<_>>());
    assert_eq!(f.history(&r)["occurrences"][0]["occurrence_id"], "newer");
    let other = f.create("Other");
    for cursor in [json!("invalid"), json!("-1.0"), json!("999999.999999")] {
        assert!(f
            .app
            .api(
                "POST",
                "/api/routines/history",
                json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"cursor":cursor})
                    .to_string()
                    .as_bytes()
            )
            .is_err());
    }
    assert!(f.app.api("POST","/api/routines/history",json!({"bot_id":other.bot_id,"routine_id":other.routine_id,"cursor":first["next_cursor"]}).to_string().as_bytes()).is_err());
}

#[test]
fn concurrent_panel_resume_and_launch_coalesce_and_aliases_survive_restart() {
    let mut f = Fixture::new(false);
    let r = f.create("Concurrent report");
    f.post(
        "/api/routines/pause",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id}),
    );
    for endpoint in ["/api/routines/resume", "/api/routines/run-now"] {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let threads = (0..2)
            .map(|n| {
                let path = f.path.clone();
                let provider = f.provider.clone();
                let id = r.routine_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut app = App::open_with(&path, provider).unwrap();
                    barrier.wait();
                    let payload = if endpoint.ends_with("run-now") {
                        json!({"bot_id":"grokbot","routine_id":id,"nonce":format!("client-{n}")})
                    } else {
                        json!({"bot_id":"grokbot","routine_id":id})
                    };
                    app.api("POST", endpoint, payload.to_string().as_bytes())
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let replies = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(replies[0], replies[1]);
    }
    let first = f.post(
        "/api/routines/run-now",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"nonce":"client-0"}),
    )["occurrence"]["occurrence_id"]
        .clone();
    let mut ctx = f.app.feature_context();
    ctx.now_ms = r.modified_at as u64;
    features::routines::tick(&ctx);
    assert_eq!(f.count(), 1);
    f.reopen();
    // Even a new client intent just after synchronous completion is coalesced.
    let cooldown = f.post(
        "/api/routines/run-now",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"nonce":"third-client"}),
    );
    assert_eq!(cooldown["occurrence"]["occurrence_id"], first);
    // Replay after cooldown (force old timestamp) still returns the old identity.
    f.db()
        .execute(
            "UPDATE scheduler_occurrences SET scheduled_at_utc=1 WHERE occurrence_id=?1",
            [first.as_str().unwrap()],
        )
        .unwrap();
    let alias = f.post(
        "/api/routines/run-now",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"nonce":"client-1"}),
    );
    assert_eq!(alias["occurrence"]["occurrence_id"], first);
    let h = f.history(&r);
    assert_eq!(h["occurrences"][0]["conversation_id"], r.conversation_id);
    assert_eq!(h["occurrences"][0]["run_id"], f.run(&r).run_id);
    assert_eq!(
        h["occurrences"][0]["message_id"],
        format!("reply-{}", f.run(&r).run_id)
    );
    // Failure outputs are linked as well as successes.
    assert_eq!(h["occurrences"][0]["status"]["status"], "failed");
    assert_eq!(f.count(), 1);
}

#[test]
fn concurrent_resumes_preserve_one_schedule_anchor_and_one_due_run() {
    let mut f = Fixture::new(false);
    let r = f.create("Resume once");
    f.post(
        "/api/routines/pause",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id}),
    );
    let reviewed = routine::get(&f.db(), &r.routine_id).unwrap().unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads=(0..2).map(|_| {
        let path=f.path.clone(); let provider=f.provider.clone(); let barrier=barrier.clone(); let reviewed=reviewed.clone();
        std::thread::spawn(move || {
            let mut app=App::open_with(&path,provider).unwrap(); barrier.wait();
            app.api("POST","/api/routines/resume",json!({"bot_id":reviewed.bot_id,"routine_id":reviewed.routine_id,"modified_at":reviewed.modified_at,"schedule_version":reviewed.schedule_version}).to_string().as_bytes()).unwrap()
        })
    }).collect::<Vec<_>>();
    let results = threads
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0], results[1]);
    let resumed = routine::get(&f.db(), &r.routine_id).unwrap().unwrap();
    f.tick_due(&resumed);
    f.tick_due(&resumed);
    f.reopen();
    f.tick_due(&resumed);
    assert_eq!(f.count(), 1);
    assert_eq!(f.history(&r)["occurrences"].as_array().unwrap().len(), 1);
}

#[test]
fn panel_confirmation_rejects_a_changed_review_and_history_denies_wrong_bot() {
    let mut f = Fixture::new(false);
    let r = f.create("Reviewed report");
    f.post(
        "/api/routines/pause",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id}),
    );
    let err=f.app.api("POST","/api/routines/resume",json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version}).to_string().as_bytes()).unwrap_err();
    assert_eq!(err.status, 409);
    assert!(err.message.contains("Relisez"));
    assert!(
        !routine::get(&f.db(), &r.routine_id)
            .unwrap()
            .unwrap()
            .enabled
    );
    assert_eq!(f.count(), 0);
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/routines/history",
                json!({"bot_id":"missing","routine_id":r.routine_id})
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        404
    );
}

#[test]
fn idle_ping_is_small_stable_and_detects_local_and_background_writes() {
    let mut f = Fixture::new(false);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let first = f.app.api("GET", "/api/ping", b"").unwrap();
    assert_eq!(first["revision"], state["state_revision"]);
    assert_eq!(first.as_object().unwrap().len(), 1);
    assert!(first.to_string().len() < 150);
    assert_eq!(first, f.app.api("GET", "/api/ping", b"").unwrap());
    f.create("A local write changes the revision");
    let local = f.app.api("GET", "/api/ping", b"").unwrap();
    assert_ne!(local, first);
    f.db()
        .execute(
            "UPDATE wrapper_bots SET description=description || ' suite' WHERE id='grokbot'",
            [],
        )
        .unwrap();
    let background = f.app.api("GET", "/api/ping", b"").unwrap();
    assert_ne!(background, local);
    assert_eq!(background, f.app.api("GET", "/api/ping", b"").unwrap());
    assert_eq!(
        background["revision"],
        f.app.api("GET", "/api/state", b"").unwrap()["state_revision"]
    );
}

#[test]
fn a_scheduler_diagnostic_is_not_coalesced_with_a_manual_launch() {
    let mut f = Fixture::new(false);
    let r = f.create("Manual launch after a diagnostic");
    f.db().execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,manual_nonce,status,reason) VALUES ('quarantine-row',?1,1,?2,?3,'failed','invalid schedule')", params![r.routine_id, now(),format!("scheduler-error:{}",now())]).unwrap();
    let result = f.post(
        "/api/routines/run-now",
        json!({"bot_id":r.bot_id,"routine_id":r.routine_id,"nonce":"fresh-manual"}),
    );
    assert_ne!(result["occurrence"]["occurrence_id"], "quarantine-row");
    assert_eq!(result["occurrence"]["manual_nonce"], "fresh-manual");
    assert_eq!(result["occurrence"]["status"]["status"], "pending");
}

#[test]
fn past_once_resume_is_refused_and_creation_time_is_server_owned() {
    let mut f = Fixture::new(false);
    let r = f.create("Report");
    assert!(r.modified_at > 0);
    let db = f.db();
    db.execute(
        "UPDATE scheduler_routines SET enabled=0,trigger=?2 WHERE routine_id=?1",
        params![r.routine_id, json!({"kind":"once","at":1}).to_string()],
    )
    .unwrap();
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/routines/resume",
                json!({"bot_id":"grokbot","routine_id":r.routine_id})
                    .to_string()
                    .as_bytes()
            )
            .unwrap_err()
            .status,
        400
    );
}

fn webhook_routine(f: &mut Fixture, prompt: &str) -> (Routine, String, String) {
    let created = f.post(
        "/api/routines",
        json!({"bot_id":"grokbot","name":"Hook","prompt":prompt,"trigger":{"kind":"webhook"}}),
    );
    let r: Routine = serde_json::from_value(created["routine"].clone()).unwrap();
    let url = format!(
        "/hooks/{}/{}",
        r.routine_id,
        created["webhook"]["trigger_id"].as_str().unwrap()
    );
    let bearer = format!("Bearer {}", created["webhook"]["token"].as_str().unwrap());
    (r, url, bearer)
}

#[test]
fn webhook_body_reaches_the_model_as_untrusted_data_and_the_thread_shows_it() {
    let mut f = Fixture::new(true);
    let (r, url, bearer) = webhook_routine(&mut f, "HOOK_PROMPT");
    let hook = features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer)).unwrap();
    f.events(vec![
        call("done", "complete_task", json!({"summary":"Reçu"})),
        complete(),
    ]);
    f.events(vec![complete()]);
    let reply =
        features::routine_webhooks::accept(&mut f.app, &hook, None, br#"{"issue":"HOOK_BODY_42"}"#);
    assert_eq!(reply.status, 200);
    f.app.tick(now());
    f.wait_run(&r);
    assert_eq!(
        f.run(&r).task_status,
        Some(TaskStatus::Done {
            summary: "Reçu".into()
        })
    );
    let history = f.history(&r);
    assert_eq!(history["occurrences"][0]["status"]["status"], "done");
    assert_eq!(history["occurrences"][0]["trigger"]["label"], "Webhook");
    {
        let seen = f.server.seen.lock().unwrap();
        assert!(seen.bodies.iter().any(|b| {
            let text = b.to_string();
            text.contains("HOOK_PROMPT")
                && text.contains("HOOK_BODY_42")
                && text.contains("webhook-body")
                && text.contains("non fiable")
        }));
    }
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    let thread = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == r.conversation_id)
        .unwrap()["messages"]
        .to_string();
    assert!(thread.contains("HOOK_BODY_42"));
    assert!(!thread.contains(bearer.trim_start_matches("Bearer ")));
}

#[test]
fn webhook_calls_of_one_routine_run_one_at_a_time_in_order() {
    let mut f = Fixture::new(true);
    let (r, url, bearer) = webhook_routine(&mut f, "Serial");
    let hook = features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer)).unwrap();
    for _ in 0..2 {
        f.events(vec![
            call("done", "complete_task", json!({"summary":"Reçu"})),
            complete(),
        ]);
        f.events(vec![complete()]);
    }
    for n in 0..2 {
        let body = format!("{{\"n\":{n}}}");
        let reply = features::routine_webhooks::accept(&mut f.app, &hook, None, body.as_bytes());
        assert_eq!(reply.status, 200);
    }
    assert_eq!(f.history(&r)["occurrences"].as_array().unwrap().len(), 2);
    f.app.tick(now());
    // The first run is still active in the routine thread: the second call waits.
    assert_eq!(f.count(), 1);
    wait("both webhook calls ran, one after the other", || {
        f.app.tick(now());
        f.count() == 2
    });
    let statuses: Vec<String> = f.history(&r)["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["status"]["status"].to_string())
        .collect();
    assert_eq!(statuses.len(), 2);
}

#[test]
fn a_bot_can_neither_create_nor_switch_a_routine_to_a_webhook_trigger() {
    let mut f = Fixture::new(false);
    let refused = f
        .app
        .invoke_tool(
            "grokbot",
            "create_routine",
            json!({"name":"Public","prompt":"Report","trigger":{"kind":"webhook"}}),
        )
        .unwrap();
    assert!(matches!(refused, ToolOutcome::Failed { .. }));
    let r = f.create("Original");
    let switched = f
        .app
        .invoke_tool(
            "grokbot",
            "update_routine",
            json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"trigger":{"kind":"webhook"}}),
        )
        .unwrap();
    assert!(matches!(switched, ToolOutcome::Failed { .. }));
    let listed = f
        .app
        .api("GET", "/api/routines?bot_id=grokbot", b"")
        .unwrap();
    assert_eq!(listed["routines"][0]["trigger"]["kind"], "every");
}

#[test]
fn deleting_or_leaving_the_webhook_trigger_kills_its_token() {
    let mut f = Fixture::new(false);
    let (r, url, bearer) = webhook_routine(&mut f, "Temporary");
    assert!(features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer)).is_ok());
    let edited = f.post(
        "/api/routines/update",
        json!({"bot_id":"grokbot","routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"schedule":"@every 1h"}),
    );
    assert_eq!(edited["routine"]["trigger"]["kind"], "every");
    assert_eq!(
        features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer))
            .unwrap_err()
            .status,
        404
    );
    let (r2, url2, bearer2) = webhook_routine(&mut f, "Deleted");
    f.post(
        "/api/routines/delete",
        json!({"bot_id":"grokbot","routine_id":r2.routine_id}),
    );
    assert_eq!(
        features::routine_webhooks::authorize(&f.app, "POST", &url2, Some(&bearer2))
            .unwrap_err()
            .status,
        404
    );
}

#[test]
fn a_webhook_routine_resumes_after_a_pause_without_a_future_slot() {
    let mut f = Fixture::new(false);
    let (r, _, _) = webhook_routine(&mut f, "Pausable");
    let id = json!({"bot_id":"grokbot","routine_id":r.routine_id});
    let paused = f.post("/api/routines/pause", id.clone())["routine"].clone();
    assert_eq!(paused["enabled"], false);
    assert_eq!(
        f.post("/api/routines/resume", id)["routine"]["enabled"],
        true
    );
    f.app.tick(now() + 86_400_000);
    assert_eq!(f.count(), 0);
}

#[test]
fn a_webhook_whose_routine_a_bot_moved_to_a_schedule_answers_nothing() {
    let mut f = Fixture::new(false);
    let (r, url, bearer) = webhook_routine(&mut f, "Moved");
    assert!(features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer)).is_ok());
    let moved = f
        .app
        .invoke_tool(
            "grokbot",
            "update_routine",
            json!({"routine_id":r.routine_id,"modified_at":r.modified_at,"schedule_version":r.schedule_version,"schedule":"@every 1h"}),
        )
        .unwrap();
    assert!(matches!(moved, ToolOutcome::Done { .. }));
    assert_eq!(
        features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer))
            .unwrap_err()
            .status,
        404
    );
    assert!(
        f.app.api("GET", "/api/state", b"").unwrap()["routine_webhooks"]["items"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_queued_webhook_call_obeys_pause_and_edit_cutoffs_unlike_a_manual_launch() {
    let mut f = Fixture::new(false);
    let (r, url, bearer) = webhook_routine(&mut f, "Cutoff");
    let hook = features::routine_webhooks::authorize(&f.app, "POST", &url, Some(&bearer)).unwrap();
    let id = json!({"bot_id":"grokbot","routine_id":r.routine_id});
    // Pause before the host tick launches the accepted call.
    assert_eq!(
        features::routine_webhooks::accept(&mut f.app, &hook, None, b"{}").status,
        200
    );
    f.post("/api/routines/pause", id.clone());
    f.app.tick(now());
    assert_eq!(f.count(), 0);
    assert_eq!(
        f.history(&r)["occurrences"][0]["status"]["status"],
        "failed"
    );
    // A manual launch is not a webhook call: it still runs on a paused routine.
    f.post(
        "/api/routines/run-now",
        json!({"bot_id":"grokbot","routine_id":r.routine_id,"nonce":"click-1"}),
    );
    f.app.tick(now());
    assert_eq!(f.count(), 1);
    // An edit that changes the trigger cuts an accepted call off too.
    let resumed = f.post("/api/routines/resume", id)["routine"].clone();
    assert_eq!(
        features::routine_webhooks::accept(&mut f.app, &hook, None, b"{}").status,
        200
    );
    f.post(
        "/api/routines/update",
        json!({"bot_id":"grokbot","routine_id":r.routine_id,"modified_at":resumed["modified_at"],"schedule_version":resumed["schedule_version"],"schedule":"@every 1h"}),
    );
    f.app.tick(now());
    assert_eq!(f.count(), 1);
}
