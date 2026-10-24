//! L5: quota validation/privacy and measured local roll-ups, no external account.
mod support;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use grokbot_engine::{
    contracts::{RunBudgets, RunKind, TaskStatus, TokenUsage, Trigger, TurnEnd},
    model::{Run, RunStatus},
    store,
};
use grokbot_wrapper::{App, Endpoints, Provider};
use rusqlite::{params, Connection};
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
    dir: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
}
impl Fixture {
    fn new(reply: ScriptedResponse) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-usage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen {
            quota_reply: Some(reply),
            ..Seen::default()
        })));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = format!(
            "e30.{}.fake",
            URL_SAFE_NO_PAD.encode(
                json!({"https://api.openai.com/auth":{"chatgpt_account_id":"ezerbot-only"}})
                    .to_string()
            )
        );
        std::fs::write(dir.join("app.codex-auth.json"), json!({"host_id":"test", "client_id":"app_EMoamEEZ73f0CkXaXp7hrann","account_id":"ezerbot-only","access_token":token,"refresh_token":"only-ezerbot","expires_at":now+3600,"welcomed":true}).to_string()).unwrap();
        let provider = Provider::with_backends(
            &dir.join("app.sqlite"),
            server.endpoints(),
            Endpoints {
                api: format!("{}/backend-api/codex", server.base),
                ..server.endpoints()
            },
            false,
        );
        Self {
            dir,
            server,
            provider,
        }
    }
    fn quota(&self) -> Value {
        self.provider.codex_usage(false);
        wait("quota terminé", || {
            self.provider.codex_usage(false)["status"] != "loading"
        });
        self.provider.codex_usage(false)
    }
    fn app(&self) -> App {
        App::open_with(&self.dir.join("app.sqlite"), self.provider.clone()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
fn valid() -> Value {
    json!({"plan_type":"plus", "rate_limit":{"primary_window":{"used_percent":80,"limit_window_seconds":604800,"reset_at":4102444800u64}}})
}

#[test]
fn quota_windows_are_validated_missing_is_not_zero_and_private_fields_do_not_escape() {
    let mut value = valid();
    value["private"] = json!("secret-upstream");
    let f = Fixture::new(ScriptedResponse::json(200, value));
    let q = f.quota();
    assert_eq!(q["status"], "ready");
    assert_eq!(q["quota"]["primary"]["remaining_percent"], 20.0);
    assert!(q["quota"]["secondary"].is_null());
    assert!(!q.to_string().contains("secret") && !q.to_string().contains("ezerbot-only"));
    for value in [
        json!({}),
        json!({"rate_limit":{"primary_window":{"used_percent":-1,"limit_window_seconds":604800}}}),
        json!({"rate_limit":{"primary_window":{"used_percent":101,"limit_window_seconds":604800}}}),
        json!({"rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":604800,"reset_at":"secret-upstream"}}}),
    ] {
        let f = Fixture::new(ScriptedResponse::json(200, value));
        let q = f.quota();
        assert_eq!(q["status"], "unavailable");
        assert!(q["quota"].is_null());
        assert!(!q.to_string().contains("secret-upstream"));
    }
    let f = Fixture::new(ScriptedResponse::json(
        200,
        json!({"rate_limit":{"primary_window":{"used_percent":90,"limit_window_seconds":604800,"reset_at":1}}}),
    ));
    assert_eq!(f.quota()["status"], "unavailable"); // expired is not a made-up reset
    let value = json!({"rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":18000,"reset_after_seconds":3600}}});
    let f = Fixture::new(ScriptedResponse::json(200, value));
    assert_eq!(f.quota()["quota"]["primary"]["remaining_percent"], 100.0);
    assert!(f.quota()["quota"]["primary"]["reset_at"].as_u64().unwrap() > 1_700_000_000);
}

#[test]
fn quota_errors_and_signout_during_a_read_never_restore_a_gauge() {
    for status in [403, 429, 503] {
        let f = Fixture::new(ScriptedResponse::json(
            status,
            json!({"error":"secret-upstream"}),
        ));
        let q = f.quota();
        assert_eq!(q["status"], "unavailable");
        assert_eq!(q["kind"], "other");
        assert!(!q.to_string().contains("secret-upstream"));
    }
    let f = Fixture::new(ScriptedResponse::json(200, valid()));
    let gate = Arc::new(AtomicBool::new(false));
    f.server.seen.lock().unwrap().quota_gate = Some(gate.clone());
    f.provider.codex_usage(false);
    wait("lecture en vol", || {
        !f.server.seen.lock().unwrap().quota_headers.is_empty()
    });
    f.provider.signout().unwrap();
    gate.store(true, Ordering::SeqCst);
    assert_eq!(f.provider.codex_usage(false)["status"], "signin");
    assert!(f.provider.codex_usage(false)["quota"].is_null());
}

fn seed_run(
    conn: &Connection,
    id: &str,
    bot: &str,
    conversation: &str,
    kind: &str,
    measured: bool,
) {
    insert_run(conn, id, bot, conversation, kind, 1);
    if measured {
        add_usage(conn, id, 100, 20, 30);
    }
}

fn insert_run(conn: &Connection, id: &str, bot: &str, conversation: &str, kind: &str, at: i64) {
    let trigger = match kind {
        "routine" => Trigger::Occurrence {
            occurrence_id: id.into(),
        },
        "delegation" => Trigger::Delegation {
            delegation_id: id.into(),
        },
        _ => Trigger::UserMessage {
            message_id: id.into(),
            attempt: 0,
        },
    };
    let trigger_kind = match kind {
        "routine" => "occurrence",
        "delegation" => "delegation",
        _ => "user_message",
    };
    let run = Run {
        run_id: id.into(),
        bot_id: bot.into(),
        conversation_id: conversation.into(),
        trigger,
        kind: if matches!(kind, "routine" | "delegation") {
            RunKind::Task
        } else {
            RunKind::Chat
        },
        status: RunStatus::Finished,
        task_status: matches!(kind, "routine" | "delegation").then(|| TaskStatus::Done {
            summary: "Fini".into(),
            captures: vec![],
        }),
        turn_end: Some(TurnEnd::Finished),
        budgets: RunBudgets::default(),
        started_at: at,
    };
    conn.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES (?1,?2,?3,?4,?1,0,'finished',?5)", params![id,bot,conversation,trigger_kind,serde_json::to_string(&run).unwrap()]).unwrap();
}

fn add_usage(conn: &Connection, id: &str, input: u64, output: u64, cached: u64) {
    let tx = conn.unchecked_transaction().unwrap();
    let usage = TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: cached,
    };
    store::save_token_usage(&tx, id, 1, &usage).unwrap();
    store::save_token_usage(&tx, id, 1, &usage).unwrap(); // duplicate telemetry
    store::save_token_usage(&tx, id, 2, &usage).unwrap(); // second model response
    tx.commit().unwrap();
}

#[test]
fn local_tokens_roll_up_once_per_bot_and_source_and_survive_deletion_restart() {
    let f = Fixture::new(ScriptedResponse::json(200, valid()));
    let mut app = f.app();
    let conn = Connection::open(f.dir.join("app.sqlite")).unwrap();
    let bot: String = conn
        .query_row("SELECT id FROM wrapper_bots LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (id, source) in [
        ("chat", "chat"),
        ("routine", "routine"),
        ("delegate", "delegation"),
    ] {
        seed_run(&conn, id, &bot, &conversation, source, true);
    }
    let group = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    conn.execute(
        "UPDATE wrapper_conversations SET kind='group' WHERE id=?1",
        [&group],
    )
    .unwrap();
    seed_run(&conn, "group", &bot, &group, "group", true);
    seed_run(&conn, "interbot", &bot, &group, "group", true);
    conn.execute("INSERT INTO wrapper_group_turns(account_id,conversation_id,id,root_id,parent_id,bot_id,cause_id,prompt,priority,status,run_id) SELECT account_id,?1,'turn','root','parent',id,'cause','',0,'completed','interbot' FROM wrapper_bots WHERE id=?2",params![group,bot]).unwrap();
    seed_run(&conn, "unknown", &bot, &conversation, "chat", false);
    seed_run(
        &conn,
        "deleted",
        "old-bot",
        "old-conversation",
        "chat",
        true,
    );
    let get = |app: &mut App| app.api("GET", "/api/usage", b"").unwrap()["local"].clone();
    let data = get(&mut app);
    assert_eq!(data["total"]["input_tokens"], 1200);
    assert_eq!(data["total"]["output_tokens"], 240);
    assert_eq!(data["total"]["cached_input_tokens"], 360); // included in input, not added
    assert_eq!(data["total"]["measured_runs"], 6);
    assert_eq!(data["total"]["unknown_runs"], 1);
    assert_eq!(data["by_bot"].as_array().unwrap().len(), 2);
    for source in ["chat", "routine", "group", "delegation", "bot_to_bot"] {
        let row = data["by_source"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["source"] == source)
            .unwrap();
        assert_eq!(
            row["tokens"]["input_tokens"],
            if source == "chat" { 400 } else { 200 }
        );
    }
    conn.execute(
        "UPDATE wrapper_conversations SET deleted=1 WHERE id=?1",
        [conversation],
    )
    .unwrap();
    assert_eq!(get(&mut app), data);
    drop(app);
    let mut app = f.app();
    assert_eq!(get(&mut app), data);
}

fn edge(conn: &Connection, parent: &str, child: &str) {
    conn.execute("INSERT INTO delegation_graph(id,parent_run_id,root_run_id,parent_bot,child_bot,depth,child_run_id,conversation_id,selected_context,status) VALUES (?1,?2,?2,'b','b',1,?1,NULL,'{}','{\"status\":\"done\"}')", params![child, parent]).unwrap();
}

#[test]
fn usage_totals_are_frozen_for_chat_routine_and_delegation_runs() {
    let f = Fixture::new(ScriptedResponse::json(200, valid()));
    let mut app = f.app();
    let conn = Connection::open(f.dir.join("app.sqlite")).unwrap();
    let bot: String = conn
        .query_row("SELECT id FROM wrapper_bots LIMIT 1", [], |r| r.get(0))
        .unwrap();
    let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for (id, source, measured) in [
        ("chat", "chat", true),
        ("routine", "routine", true),
        ("child", "delegation", true),
        ("grandchild", "delegation", true),
        ("unknown", "chat", false),
    ] {
        seed_run(&conn, id, &bot, &conversation, source, measured);
    }
    edge(&conn, "routine", "child");
    edge(&conn, "child", "grandchild");
    let name: String = conn
        .query_row("SELECT name FROM wrapper_bots WHERE id=?1", [&bot], |r| {
            r.get(0)
        })
        .unwrap();
    let tokens = |input: u64, output: u64, cached: u64, measured: u64, unknown: u64| json!({"input_tokens":input,"output_tokens":output,"cached_input_tokens":cached,"measured_runs":measured,"unknown_runs":unknown});
    let local = app.api("GET", "/api/usage", b"").unwrap()["local"].clone();
    assert_eq!(
        local,
        json!({
            "total": tokens(800, 160, 240, 4, 1),
            "by_bot": [{"id":bot,"name":name,"deleted":false,"tokens":tokens(800, 160, 240, 4, 1)}],
            "by_source": [
                {"source":"bot_to_bot","tokens":tokens(0, 0, 0, 0, 0)},
                {"source":"chat","tokens":tokens(200, 40, 60, 1, 1)},
                {"source":"delegation","tokens":tokens(400, 80, 120, 2, 0)},
                {"source":"group","tokens":tokens(0, 0, 0, 0, 0)},
                {"source":"routine","tokens":tokens(200, 40, 60, 1, 0)},
            ],
        })
    );
}

const DAY_MS: i64 = 86_400_000;

/// A routine on `schedule` with its bot and conversation, plus the app and a raw connection.
struct Cost {
    f: Fixture,
    app: App,
    conn: Connection,
    bot: String,
    routine: String,
    conversation: String,
}
impl Cost {
    fn new(quota: Value, trigger: Value) -> Self {
        let f = Fixture::new(ScriptedResponse::json(200, quota));
        let mut app = f.app();
        let conn = Connection::open(f.dir.join("app.sqlite")).unwrap();
        let bot: String = conn
            .query_row("SELECT id FROM wrapper_bots LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let body = json!({"bot_id":bot,"name":"Veille","prompt":"Résume.","trigger":trigger});
        let created = app
            .api("POST", "/api/routines", body.to_string().as_bytes())
            .unwrap();
        f.quota();
        let routine = created["routine"]["routine_id"].as_str().unwrap().into();
        let conversation = created["routine"]["conversation_id"]
            .as_str()
            .unwrap()
            .into();
        Self {
            f,
            app,
            conn,
            bot,
            routine,
            conversation,
        }
    }
    fn every_30_minutes() -> Self {
        Self::new(valid(), json!({"kind":"every","interval_ms":1_800_000}))
    }
    /// One passage whose run started `age_ms` ago and used `tokens` in total.
    fn pass(&self, id: &str, status: &str, age_ms: i64, tokens: Option<u64>) -> &Self {
        self.pass_for(&self.routine, id, status, age_ms, tokens)
    }
    fn pass_for(
        &self,
        routine: &str,
        id: &str,
        status: &str,
        age_ms: i64,
        tokens: Option<u64>,
    ) -> &Self {
        self.late_pass_for(routine, id, status, age_ms, age_ms, tokens)
    }
    /// A passage scheduled `slot_age_ms` ago whose run started `age_ms` ago (a catch-up).
    fn late_pass_for(
        &self,
        routine: &str,
        id: &str,
        status: &str,
        slot_age_ms: i64,
        age_ms: i64,
        tokens: Option<u64>,
    ) -> &Self {
        let started = (now() * 1000) as i64 - age_ms;
        let scheduled = (now() * 1000) as i64 - slot_age_ms;
        insert_run(
            &self.conn,
            id,
            &self.bot,
            &self.conversation,
            "routine",
            started,
        );
        if let Some(tokens) = tokens {
            add_usage(&self.conn, id, tokens / 2 - 100, 100, 0);
        }
        self.conn.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,run_id,reason) VALUES (?1,?2,1,?3,?4,?1,'test')", params![id, routine, scheduled, status]).unwrap();
        self
    }
    /// A local run with `tokens` in total, started a day ago, that no routine owns.
    fn chat(&self, tokens: u64) {
        let started = (now() * 1000) as i64 - DAY_MS;
        insert_run(
            &self.conn,
            "chat",
            &self.bot,
            &self.conversation,
            "chat",
            started,
        );
        add_usage(&self.conn, "chat", tokens / 2 - 100, 100, 0);
    }
    /// Ticks so the app notes the signed-in account, then dates that sign-in long ago.
    fn trust_account(&mut self) {
        self.app.tick(now() * 1000);
        self.conn
            .execute("UPDATE wrapper_routine_cost_account SET since=0", [])
            .unwrap();
    }
    /// The cost the routine list and the routines state both return, which must agree.
    fn cost(&mut self) -> Value {
        let ours = |routines: &Value, routine: &str| {
            routines
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["routine_id"] == routine)
                .unwrap()["cost"]
                .clone()
        };
        let list = self
            .app
            .api("GET", &format!("/api/routines?bot_id={}", self.bot), b"")
            .unwrap()["routines"]
            .clone();
        let state = self.app.api("GET", "/api/state", b"").unwrap()["routines"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["routine"].clone())
            .collect::<Vec<_>>();
        let (list, state) = (
            ours(&list, &self.routine),
            ours(&json!(state), &self.routine),
        );
        assert_eq!(list, state);
        list
    }
}

#[test]
fn a_routine_costs_the_mean_of_its_last_ten_finished_passages_and_seven_days_of_tokens() {
    let mut c = Cost::every_30_minutes();
    c.pass("a", "done", 3 * 3_600_000, Some(1000))
        .pass("b", "done", 2 * 3_600_000, Some(2000))
        .pass("c", "done", 3_600_000, Some(3000));
    let cost = c.cost();
    assert_eq!(cost["state"], "measured");
    assert_eq!(cost["average_tokens"], 2000);
    assert_eq!(cost["average_sample"], 3);
    assert_eq!(cost["tokens_7d"], 6000);
    // Eleven older passages: only the ten latest finished ones average; the 8-day one is out of the 7 days.
    let mut c = Cost::every_30_minutes();
    c.pass("old", "done", 8 * DAY_MS, Some(90_000))
        .pass("older", "done", 6 * DAY_MS, Some(90_000));
    for n in 0..10 {
        c.pass(&format!("n{n}"), "done", n * 60_000 + 60_000, Some(1000));
    }
    let cost = c.cost();
    assert_eq!(cost["average_tokens"], 1000);
    assert_eq!(cost["average_sample"], 10);
    assert_eq!(cost["tokens_7d"], 100_000);
}

#[test]
fn a_subtask_and_its_own_subtask_count_once_in_the_passage_that_started_them() {
    let mut c = Cost::every_30_minutes();
    c.pass("root", "done", 3_600_000, Some(1000));
    for (id, parent, tokens) in [("child", "root", 500), ("grandchild", "child", 250)] {
        insert_run(&c.conn, id, &c.bot, &c.conversation, "delegation", 1);
        add_usage(&c.conn, id, tokens / 2 - 50, 50, 0);
        edge(&c.conn, parent, id);
    }
    // Another routine's passage, with its own subtask, is not ours.
    let other = c
        .app
        .api(
            "POST",
            "/api/routines",
            json!({"bot_id":c.bot,"name":"Autre","prompt":"Résume.","schedule":"@every 1h"})
                .to_string()
                .as_bytes(),
        )
        .unwrap()["routine"]["routine_id"]
        .as_str()
        .unwrap()
        .to_owned();
    c.pass_for(&other, "foreign", "done", 3_600_000, Some(5000));
    insert_run(
        &c.conn,
        "foreign-child",
        &c.bot,
        &c.conversation,
        "delegation",
        1,
    );
    add_usage(&c.conn, "foreign-child", 1450, 50, 0);
    edge(&c.conn, "foreign", "foreign-child");
    let cost = c.cost();
    assert_eq!(cost["average_tokens"], 1750);
    assert_eq!(cost["tokens_7d"], 1750);
}

#[test]
fn the_daily_forecast_is_the_mean_times_the_passages_the_schedule_plans_each_day() {
    for trigger in [
        json!({"kind":"every","interval_ms":1_800_000}),
        json!({"kind":"cron","expression":"*/30 * * * *","time_zone":"UTC"}),
    ] {
        let mut c = Cost::new(valid(), trigger);
        c.pass("a", "done", 3_600_000, Some(2000));
        let forecast = c.cost()["forecast"].clone();
        assert_eq!(forecast["state"], "estimated");
        assert_eq!(forecast["tokens_per_day"], 96_000);
        assert_eq!(forecast["tokens_next_7d"], 672_000);
    }
    // A weekly routine is one passage a week, not zero on six days out of seven.
    let mut c = Cost::new(
        valid(),
        json!({"kind":"cron","expression":"0 9 * * 1","time_zone":"UTC"}),
    );
    c.pass("a", "done", 3_600_000, Some(2000));
    let forecast = c.cost()["forecast"].clone();
    assert_eq!(forecast["runs_next_7d"], 1);
    assert_eq!(forecast["tokens_per_day"], 286);
    // Events have no clock: the forecast is impossible, not zero.
    let mut c = Cost::new(valid(), json!({"kind":"webhook"}));
    c.pass("a", "done", 3_600_000, Some(2000));
    let forecast = c.cost()["forecast"].clone();
    assert_eq!(forecast["state"], "impossible");
    assert_eq!(forecast["reason"], "event_trigger");
    assert!(forecast["tokens_per_day"].is_null());
}

#[test]
fn a_routine_without_a_measured_finished_passage_has_no_measure_yet() {
    let mut c = Cost::every_30_minutes();
    let cost = c.cost();
    assert_eq!(cost["state"], "no_measure_yet");
    assert!(cost["average_tokens"].is_null() && cost["tokens_7d"].is_null());
    assert_eq!(cost["forecast"]["state"], "no_measure_yet");
    // A failed passage, and one whose provider sent no usage, are not zeros.
    c.pass("failed", "failed", 60_000, Some(4000))
        .pass("blind", "done", 120_000, None);
    let cost = c.cost();
    assert_eq!(cost["state"], "no_measure_yet");
    assert!(cost["average_tokens"].is_null());
    // What the failed passage did use is still the week's measured total.
    assert_eq!(cost["tokens_7d"], 4000);
}

#[test]
fn the_forecast_follows_the_scheduler_cursor_and_counts_each_group_member_on_its_own() {
    // Every ten days, last due nine days ago: the next passage is tomorrow, inside the week.
    let mut c = Cost::new(valid(), json!({"kind":"every","interval_ms":864_000_000}));
    c.pass("a", "done", 3_600_000, Some(2000));
    let cursor = (now() * 1000) as i64 - 9 * DAY_MS;
    c.conn
        .execute(
            "UPDATE scheduler_routines SET materialized_after=?1 WHERE routine_id=?2",
            params![cursor, c.routine],
        )
        .unwrap();
    assert_eq!(c.cost()["forecast"]["runs_next_7d"], 1);
    // Two coincident daily triggers start two passages a day, as the scheduler materializes them.
    let daily = json!({"trigger":{"kind":"cron","expression":"0 9 * * *","time_zone":"UTC"}});
    let mut c = Cost::new(valid(), json!({"kind":"group","members":[daily, daily]}));
    c.pass("a", "done", 3_600_000, Some(2000));
    assert_eq!(c.cost()["forecast"]["runs_next_7d"], 14);
}

#[test]
fn a_weekday_schedule_is_counted_whatever_the_day_it_is_read() {
    // Any 7 days hold five weekdays of 1 440 minutes.
    let mut c = Cost::new(
        valid(),
        json!({"kind":"cron","expression":"* * * * 1-5","time_zone":"UTC"}),
    );
    c.pass("a", "done", 3_600_000, Some(2000));
    let forecast = c.cost()["forecast"].clone();
    assert_eq!(forecast["runs_next_7d"], 7200);
    assert_eq!(forecast["tokens_next_7d"], 14_400_000);
    // Nothing is planned while the routine is paused.
    c.conn
        .execute("UPDATE scheduler_routines SET enabled=0", [])
        .unwrap();
    assert_eq!(c.cost()["forecast"]["runs_next_7d"], 0);
}

#[test]
fn the_quota_share_needs_five_percent_used_and_fifty_thousand_local_tokens() {
    // The routine's own passage ("a", 2 000 tokens) is local too: chat + 2 000 = local.
    for (used, chat, share) in [
        (4.9, 98_000, "unknown"),
        (5.0, 98_000, "estimated"),
        (20.0, 47_998, "unknown"),
        (20.0, 48_000, "estimated"),
    ] {
        let mut c = Cost::new(
            weekly(used),
            json!({"kind":"every","interval_ms":1_800_000}),
        );
        c.chat(chat);
        c.pass("a", "done", 3_600_000, Some(2000));
        c.trust_account();
        let quota_share = c.cost()["quota_share"].clone();
        assert_eq!(
            quota_share["state"], share,
            "{used} % {chat}: {quota_share}"
        );
        if share == "unknown" {
            assert_eq!(quota_share["reason"], "calibration_weak");
        }
    }
}

#[test]
fn another_account_never_calibrates_this_ones_quota() {
    // Account A ran 2 000 000 tokens; account B signs in, and its gauge says 20 % used.
    let mut c = Cost::new(
        weekly(20.0),
        json!({"kind":"every","interval_ms":1_800_000}),
    );
    c.chat(2_000_000);
    c.pass("a", "done", 3_600_000, Some(2000));
    c.conn
        .execute(
            "INSERT OR REPLACE INTO wrapper_routine_cost_account(id,identity,since) VALUES (1,'account-a',0)",
            [],
        )
        .unwrap();
    let share = c.cost()["quota_share"].clone();
    assert_eq!(share["state"], "unknown");
    assert_eq!(share["reason"], "account_unconfirmed");
    assert!(share["percent"].is_null());
    // The tick dates the new account's sign-in now: still unconfirmed for the window it began in.
    c.app.tick(now() * 1000);
    assert_eq!(c.cost()["quota_share"]["reason"], "account_unconfirmed");
}

#[test]
fn a_return_to_the_confirmed_account_after_another_one_ran_stays_unconfirmed_without_a_tick() {
    // Account A is confirmed since before the window; its gauge says 20 % used.
    let mut c = Cost::new(
        weekly(20.0),
        json!({"kind":"every","interval_ms":1_800_000}),
    );
    c.chat(50_000);
    c.pass("a", "done", 3_600_000, Some(2000));
    c.trust_account();
    assert_eq!(c.cost()["quota_share"]["state"], "estimated");
    // Account B signs in and uses the app; no tick runs. Then A signs back in, still no tick.
    let session = c.f.dir.join("app.codex-auth.json");
    let account_a = std::fs::read(&session).unwrap();
    support::write_codex_session(&session, "account-b");
    let conversation = c.app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let body = json!({"conversation_id":conversation,"message_id":"b1","body":"Bonjour"});
    c.app
        .api("POST", "/api/send", body.to_string().as_bytes())
        .unwrap();
    wait("la réponse du compte B", || {
        c.conn
            .query_row(
                "SELECT COUNT(*) FROM engine_runs WHERE conversation_id=?1 AND status='finished'",
                [&conversation],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0
    });
    let started = (now() * 1000) as i64 - 1000;
    insert_run(&c.conn, "chat-b", &c.bot, &conversation, "chat", started);
    add_usage(&c.conn, "chat-b", 1_999_900, 100, 0);
    std::fs::write(&session, account_a).unwrap();
    let share = c.cost()["quota_share"].clone();
    assert_eq!(share["state"], "unknown", "{share}");
    assert_eq!(share["reason"], "account_unconfirmed");
    assert!(share["percent"].is_null());
}

#[test]
fn passages_without_telemetry_never_hide_a_measured_one_however_many_are_newer() {
    let mut c = Cost::every_30_minutes();
    c.pass("measured", "done", 2 * DAY_MS, Some(2000));
    for n in 0..200 {
        c.pass(&format!("blind{n}"), "done", 3_600_000 + n * 1000, None);
    }
    let cost = c.cost();
    assert_eq!(
        (
            cost["average_tokens"].clone(),
            cost["average_sample"].clone()
        ),
        (json!(2000), json!(1)),
        "{cost}"
    );
    assert_eq!(cost["forecast"]["state"], "estimated");
}

#[test]
fn catch_up_passages_count_by_when_their_run_started_and_a_backlog_hides_nothing() {
    // Twelve slots owed for ten days, run today: the week counts them all, the average takes ten.
    let mut c = Cost::every_30_minutes();
    for n in 0..12 {
        c.late_pass_for(
            &c.routine,
            &format!("late{n}"),
            "done",
            10 * DAY_MS + n * 1000,
            3_600_000 - n * 1000,
            Some(1000),
        );
    }
    let cost = c.cost();
    assert_eq!(cost["tokens_7d"], 12_000);
    assert_eq!(
        (
            cost["average_tokens"].clone(),
            cost["average_sample"].clone()
        ),
        (json!(1000), json!(10))
    );
    // 250 newer catch-up occurrences still pending do not push the finished ones out of sight.
    let now_ms = (now() * 1000) as i64;
    for n in 0..250 {
        c.conn
            .execute(
                "INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,status,catch_up) VALUES (?1,?2,1,?3,'pending',1)",
                params![format!("backlog{n}"), c.routine, now_ms + n],
            )
            .unwrap();
    }
    let cost = c.cost();
    assert_eq!(cost["state"], "measured");
    assert_eq!(cost["average_sample"], 10);
}

#[test]
fn a_week_without_telemetry_is_unknown_and_a_week_without_passages_is_zero() {
    let mut c = Cost::every_30_minutes();
    c.pass("old", "done", 10 * DAY_MS, Some(1000));
    assert_eq!(c.cost()["tokens_7d"], 0);
    c.pass("blind", "done", 3_600_000, None);
    assert!(c.cost()["tokens_7d"].is_null());
}

#[test]
fn the_tick_notes_the_account_whatever_the_quota_and_a_read_never_writes() {
    // B was the account noted; the quota has no weekly window and nothing is measured yet.
    let mut c = Cost::new(json!({}), json!({"kind":"every","interval_ms":1_800_000}));
    let noted = |c: &Cost| -> (String, i64) {
        c.conn
            .query_row(
                "SELECT identity, since FROM wrapper_routine_cost_account",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    c.conn
        .execute(
            "INSERT OR REPLACE INTO wrapper_routine_cost_account(id,identity,since) VALUES (1,'account-b',0)",
            [],
        )
        .unwrap();
    assert_eq!(c.cost()["state"], "no_measure_yet");
    assert_eq!(noted(&c), ("account-b".into(), 0)); // reading changed nothing
    let before = (now() * 1000) as i64;
    c.app.tick(before as u64);
    let (identity, since) = noted(&c);
    assert_ne!(identity, "account-b");
    assert!(since >= before);
    // Ticking again with the same account keeps its date.
    c.app.tick(before as u64 + 5000);
    assert_eq!(noted(&c), (identity, since));
}

fn is_money(key: &str) -> bool {
    ["usd", "dollar", "price", "prix", "eur", "currency"]
        .iter()
        .any(|word| key.to_lowercase().contains(word))
}
fn money_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => map
            .iter()
            .flat_map(|(k, v)| {
                let mut found = money_keys(v);
                if is_money(k) {
                    found.push(k.clone());
                }
                found
            })
            .collect(),
        Value::Array(items) => items.iter().flat_map(money_keys).collect(),
        _ => vec![],
    }
}

fn weekly(used: f64) -> Value {
    json!({"rate_limit":{"primary_window":{"used_percent":used,"limit_window_seconds":604800,"reset_at":now()+3*86400}}})
}

#[test]
fn the_weekly_quota_share_is_estimated_from_this_weeks_local_tokens_and_has_no_dollar_field() {
    // 20 % of the window used while 2 000 000 tokens ran locally since it began:
    // 100 000 tokens per percent, so a 672 000-token week is 6.72 %.
    let mut c = Cost::new(
        weekly(20.0),
        json!({"kind":"every","interval_ms":1_800_000}),
    );
    assert_eq!(c.f.quota()["status"], "ready");
    insert_run(
        &c.conn,
        "chat",
        &c.bot,
        &c.conversation,
        "chat",
        (now() * 1000) as i64 - DAY_MS,
    );
    add_usage(&c.conn, "chat", 999_900, 100, 0);
    c.pass("a", "done", 3_600_000, Some(2000));
    c.trust_account();
    let cost = c.cost();
    assert_eq!(cost["quota_share"]["state"], "estimated");
    assert_eq!(cost["quota_share"]["percent"], 6.7);
    assert!(money_keys(&cost).is_empty(), "{cost}");
    // Too little use of the window, or no quota at all: no share, tokens stay.
    for (quota, reason) in [
        (weekly(1.0), "calibration_weak"),
        (json!({}), "quota_unavailable"),
    ] {
        let mut c = Cost::new(quota, json!({"kind":"every","interval_ms":1_800_000}));
        c.pass("a", "done", 3_600_000, Some(2000));
        c.trust_account();
        let cost = c.cost();
        assert_eq!(cost["quota_share"]["state"], "unknown");
        assert_eq!(cost["quota_share"]["reason"], reason);
        assert!(cost["quota_share"]["percent"].is_null());
        assert_eq!(cost["average_tokens"], 2000);
    }
}

fn window(reset: Value) -> Value {
    json!({"rate_limit":{"primary_window":{"used_percent":10,"limit_window_seconds":18000,"reset_at":reset}}})
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn reset_at_is_accepted_as_an_integer_a_string_of_seconds_or_an_iso_date() {
    for reset in [
        json!(4102444800u64),
        json!("4102444800"),
        json!("2100-01-01T00:00:00Z"),
        json!("2100-01-01T00:00:00.000Z"),
        json!("2100-01-01T01:00:00+01:00"),
    ] {
        let f = Fixture::new(ScriptedResponse::json(200, window(reset.clone())));
        let q = f.quota();
        assert_eq!(q["status"], "ready", "{reset}");
        assert_eq!(q["quota"]["primary"]["reset_at"], 4102444800u64, "{reset}");
    }
    for reset in [
        json!("secret-upstream"),
        json!("2100-01-01"),
        json!("2100-13-45T00:00:00Z"),
        json!("-5"),
        json!(-5),
        json!(0),
        json!("0"),
        json!(1.5),
        json!(300_000_000_000u64),
        json!(true),
    ] {
        let f = Fixture::new(ScriptedResponse::json(200, window(reset.clone())));
        let q = f.quota();
        assert_eq!(q["status"], "unavailable", "{reset}");
        assert!(q["quota"].is_null(), "{reset}");
        assert!(!q.to_string().contains("secret-upstream"));
    }
}

#[test]
fn an_expired_window_alone_is_unavailable_the_other_stays_and_the_quota_is_read_again() {
    let reset = now() + 2;
    let f = Fixture::new(ScriptedResponse::json(
        200,
        json!({"rate_limit":{
            "primary_window":{"used_percent":10,"limit_window_seconds":18000,"reset_at":reset},
            "secondary_window":{"used_percent":40,"limit_window_seconds":604800,"reset_at":4102531200u64}
        }}),
    ));
    let started = std::time::Instant::now();
    let q = f.quota();
    assert_eq!(q["status"], "ready");
    assert!(q["quota"]["primary"]["remaining_percent"].is_number());
    wait("fenêtre échue", || {
        f.provider.codex_usage(false)["quota"]["primary"].is_null()
    });
    let q = f.provider.codex_usage(false);
    assert_eq!(q["status"], "ready");
    assert_eq!(q["quota"]["secondary"]["remaining_percent"], 60.0);
    assert_eq!(q["quota"]["unavailable"], json!(["primary"]));
    if started.elapsed() < std::time::Duration::from_secs(4) {
        assert_eq!(f.server.seen.lock().unwrap().quota_headers.len(), 1); // 5 s between reads
    }
    wait("quota relu après l’échéance", || {
        let q = f.provider.codex_usage(false);
        q["quota"]["primary"]["remaining_percent"] == 65.0
    });
    let q = f.provider.codex_usage(false);
    assert!(q["quota"].get("unavailable").is_none());
    assert_eq!(f.server.seen.lock().unwrap().quota_headers.len(), 2);
}

#[test]
fn every_window_expired_is_unavailable_and_an_upstream_still_expired_is_not_polled_early() {
    let f = Fixture::new(ScriptedResponse::json(200, window(json!(1))));
    assert_eq!(f.quota()["status"], "unavailable");
    std::thread::sleep(std::time::Duration::from_millis(5600));
    assert_eq!(f.provider.codex_usage(false)["status"], "unavailable");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(f.server.seen.lock().unwrap().quota_headers.len(), 1); // read after the reset, still expired: wait 60 s
}

#[test]
fn quota_reads_are_coalesced_and_an_explicit_refresh_still_waits_five_seconds() {
    let f = Fixture::new(ScriptedResponse::json(200, valid()));
    assert_eq!(f.quota()["status"], "ready");
    for _ in 0..3 {
        f.provider.codex_usage(false);
        f.provider.codex_usage(true);
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert_eq!(f.provider.codex_usage(true)["status"], "ready");
    assert_eq!(f.server.seen.lock().unwrap().quota_headers.len(), 1);
}

#[test]
fn a_quota_redirect_is_never_followed() {
    use std::sync::atomic::AtomicUsize;
    let hits = Arc::new(AtomicUsize::new(0));
    let target = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let target_base = format!("http://{}", target.server_addr().to_ip().unwrap());
    let seen = hits.clone();
    std::thread::spawn(move || {
        for request in target.incoming_requests() {
            seen.fetch_add(1, Ordering::SeqCst);
            let _ = request.respond(tiny_http::Response::from_string(valid().to_string()));
        }
    });
    let origin = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let origin_base = format!("http://{}", origin.server_addr().to_ip().unwrap());
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let location = format!("{target_base}/wham/usage");
    std::thread::spawn(move || {
        for request in origin.incoming_requests() {
            counted.fetch_add(1, Ordering::SeqCst);
            let redirect = tiny_http::Response::from_string(valid().to_string())
                .with_status_code(302)
                .with_header(tiny_http::Header::from_bytes("Location", location.clone()).unwrap());
            let _ = request.respond(redirect);
        }
    });
    let f = Fixture::new(ScriptedResponse::json(200, valid()));
    std::fs::copy(
        f.dir.join("app.codex-auth.json"),
        f.dir.join("redirect.codex-auth.json"),
    )
    .unwrap();
    let provider = Provider::with_backends(
        &f.dir.join("redirect.sqlite"),
        f.server.endpoints(),
        Endpoints {
            api: format!("{origin_base}/backend-api/codex"),
            ..f.server.endpoints()
        },
        false,
    );
    provider.codex_usage(false);
    wait("quota redirigé", || {
        provider.codex_usage(false)["status"] != "loading"
    });
    let q = provider.codex_usage(false);
    assert_eq!(q["status"], "unavailable");
    assert!(q["quota"].is_null());
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "la redirection a été suivie"
    );
}
