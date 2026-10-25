//! Notification policy + durable outbox, without opening Cocoa or a network socket.
use grokbot_engine::{
    contracts::{with_tx, RunBudgets, RunKind, Trigger, TurnEnd},
    model::{self, Run, RunStatus},
};
use grokbot_wrapper::App;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct Fixture {
    app: App,
    conn: Connection,
    dir: PathBuf,
    conversation: String,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("notifications");
        let dir = guard.to_path_buf();
        let path = dir.join("workspace.sqlite");
        let mut app = App::open(&path).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conn = Connection::open(&path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        Self {
            app,
            conn,
            dir,
            conversation,
            _dir: guard,
        }
    }
    fn post(&mut self, path: &str, value: Value) -> Value {
        self.app
            .api("POST", path, value.to_string().as_bytes())
            .unwrap()
    }
    fn get(&mut self) -> Value {
        self.app.api("GET", "/api/notifications", b"").unwrap()
    }
    fn settings(&mut self, change: Value) {
        let mut prefs = self.get()["settings"].clone();
        for (key, value) in change.as_object().unwrap() {
            prefs[key] = value.clone();
        }
        self.post("/api/notifications/settings", prefs);
    }
    fn poll(&mut self, focused: bool, visible: bool) -> Value {
        self.post(
            "/api/notifications/poll",
            json!({"focused":focused,"visible":visible,"conversation_id":self.conversation}),
        )
    }
    fn reply(&self, id: &str) {
        with_tx(&self.conn, |conn| {
            model::start(
                conn,
                &Run {
                    run_id: id.into(),
                    bot_id: "grokbot".into(),
                    conversation_id: self.conversation.clone(),
                    trigger: Trigger::UserMessage {
                        message_id: format!("{}/message-{id}", self.conversation),
                        attempt: 0,
                    },
                    kind: RunKind::Chat,
                    status: RunStatus::Running,
                    task_status: None,
                    turn_end: None,
                    budgets: RunBudgets::default(),
                    started_at: 1000,
                },
            )?;
            let mut run = grokbot_engine::store::required(conn, id)?;
            run.status = RunStatus::Finished;
            run.turn_end = Some(TurnEnd::Finished);
            grokbot_engine::store::save(conn, &run)
        })
        .unwrap();
        self.conn.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,created_at,kind) VALUES ('local-workspace',?1,?2,1000,'submission')",params![self.conversation,format!("message-{id}")]).unwrap();
    }
    fn request(&self, id: &str, kind: &str) {
        self.conn.execute("INSERT INTO forms_requests(id,run_id,entry_id,kind_json,route_json) VALUES (?1,'source',?1,?2,?3)", params![id,json!({"kind":kind,"name":"NE_JAMAIS_NOTIFIER_CE_SECRET"}).to_string(),json!({"target_conversation_id":self.conversation}).to_string()]).unwrap();
    }
    /// An approval card still waiting, created `age_ms` ago and valid for 24 h more.
    fn waiting_approval(&self, id: &str, age_ms: u64) {
        let now = now_ms();
        self.conn.execute("INSERT INTO forms_requests(id,run_id,entry_id,kind_json,route_json) VALUES (?1,'source',?1,?2,?3)", params![id,json!({"kind":"approval","tool":"NE_JAMAIS_NOTIFIER_CET_OUTIL","expires_at":now+24*HOUR}).to_string(),json!({"target_conversation_id":self.conversation}).to_string()]).unwrap();
        self.conn.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,created_at,kind) VALUES ('local-workspace',?1,?2,?3,'approval')",params![self.conversation,id,now-age_ms]).unwrap();
    }
    /// Moves the clock of the card forward without waiting: the card is `age_ms` old now.
    fn age(&self, id: &str, age_ms: u64) {
        self.conn
            .execute(
                "UPDATE wrapper_messages SET created_at=?2 WHERE message_id=?1",
                params![id, now_ms() - age_ms],
            )
            .unwrap();
    }
    fn reminder(&mut self, choice: &str) {
        self.post(
            "/api/approvals/waiting",
            json!({"task":true,"chat":false,"validity":"24h","reminder":choice}),
        );
    }
    fn routine(&self, id: &str, status: &str, run: Option<&str>) {
        self.conn.execute("INSERT OR IGNORE INTO scheduler_routines(routine_id,bot_id,conversation_id,name,body,trigger,time_zone,enabled,origin,untrusted,schedule_version,modified_at,last_served_at,materialized_after,active_since) VALUES ('routine','grokbot',?1,'Routine','{}','{}','UTC',0,'user',0,1,0,NULL,0,0)",[&self.conversation]).unwrap();
        self.conn.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,manual_nonce,status,run_id,reason) VALUES (?1,'routine',1,1000,?1,?2,?3,'failure')",params![id,status,run]).unwrap();
    }
    fn message_count(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM wrapper_messages WHERE conversation_id=?1",
                [&self.conversation],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn restart(&mut self) {
        self.app = App::open(&self.dir.join("workspace.sqlite")).unwrap();
    }
}
#[test]
fn finished_reply_focus_click_and_replay() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("visible");
    let seen = f.poll(true, true);
    assert_eq!(seen["deliveries"], json!([]));
    assert_eq!(seen["unread"], 0);
    f.reply("hidden");
    let seen = f.poll(false, true);
    assert_eq!(seen["deliveries"].as_array().unwrap().len(), 1);
    assert_eq!(seen["deliveries"][0]["card_id"], "reply-message-hidden");
    assert_eq!(seen["unread"], 1);
    assert_eq!(f.poll(false, true)["deliveries"], json!([]));
    f.restart();
    assert_eq!(f.poll(false, true)["deliveries"], json!([]));
    assert_eq!(f.get()["unread"], 1);
    let opened = f.post("/api/notifications/open", json!({"id":"reply:hidden"}));
    assert_eq!(opened["conversation_id"], f.conversation);
    assert_eq!(f.get()["unread"], 0);
    f.restart();
    assert_eq!(f.get()["unread"], 0);
}

#[test]
fn engine_reply_anchor_uses_the_same_durable_key_without_the_submission_join() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("group-turn");
    f.conn
        .execute(
            "DELETE FROM wrapper_messages WHERE message_id='message-group-turn'",
            [],
        )
        .unwrap();
    f.conn.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,kind,created_at) VALUES ('local-workspace',?1,'group-turn','engine_reply',1000)",[&f.conversation]).unwrap();
    let result = f.poll(false, false);
    assert_eq!(result["deliveries"][0]["id"], "reply:group-turn");
    assert_eq!(result["deliveries"][0]["card_id"], "reply-group-turn");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

#[test]
fn conversation_hidden_or_other_conversation_notifies_even_if_window_focused() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("modal");
    assert_eq!(
        f.poll(true, false)["deliveries"].as_array().unwrap().len(),
        1
    );
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("other");
    let result = f.post(
        "/api/notifications/poll",
        json!({"focused":true,"visible":true,"conversation_id":"another"}),
    );
    assert_eq!(result["deliveries"].as_array().unwrap().len(), 1);
}

#[test]
fn preferences_dnd_and_no_backlog_on_enable() {
    let mut f = Fixture::new();
    f.reply("disabled");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.settings(json!({"enabled":true,"dnd":true}));
    f.reply("dnd");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.settings(json!({"dnd":false,"reply":false}));
    f.reply("muted");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.settings(json!({"reply":true}));
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    assert_eq!(f.get()["unread"], 3);
    f.restart();
    assert_eq!(f.get()["settings"]["enabled"], true);
    f.reply("fresh");
    assert_eq!(
        f.poll(false, false)["deliveries"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn waiting_requests_each_kind_and_no_secret_payload() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    for kind in ["approval", "form", "secret"] {
        f.request(kind, kind);
    }
    let result = f.poll(false, false);
    assert_eq!(result["deliveries"].as_array().unwrap().len(), 3);
    assert!(!result.to_string().contains("NE_JAMAIS"));
    for item in result["deliveries"].as_array().unwrap() {
        assert_eq!(item["conversation_id"], f.conversation);
        assert_eq!(item["card_id"], item["kind"]);
    }
    f.request("settled", "approval");
    // Project pending input, then a refusal arrives before native polling.
    f.get();
    f.conn
        .execute(
            "UPDATE forms_requests SET status='denied' WHERE id='settled'",
            [],
        )
        .unwrap();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.settings(json!({"secret":false}));
    f.request("muted-secret", "secret");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

#[test]
fn routine_finished_failed_and_delegated_card_destinations() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("parent");
    // Real component schemas; no engine/network work is needed to exercise terminal projection.
    f.routine("done", "done", Some("parent"));
    f.routine("failed", "failed", None);
    f.conn.execute("INSERT INTO delegation_graph VALUES ('child','parent','parent','grokbot','other',1,NULL,NULL,'{}',?1)",[json!({"status":"done","result":"NE_JAMAIS_AFFICHER"}).to_string()]).unwrap();
    let result = f.poll(false, false);
    let items = result["deliveries"].as_array().unwrap();
    assert!(items.iter().any(|item| item["kind"] == "routine"
        && item["title"] == "Routine terminée"
        && item["card_id"] == "reply-parent"));
    assert!(items
        .iter()
        .any(|item| item["title"] == "Échec de la routine" && item["card_id"] == ""));
    assert!(items.iter().any(|item| item["kind"] == "delegation"
        && item["card_id"] == "child"
        && item["conversation_id"] == f.conversation));
    assert!(!result.to_string().contains("NE_JAMAIS"));
    let cards: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM wrapper_messages WHERE kind='notification_event'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cards, 0);
    f.restart();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    let cards2: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM wrapper_messages WHERE kind='notification_event'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cards2, 0);
    // Upgrading an existing workspace establishes a baseline without inserting
    // historical routine summaries into its transcript.
    f.conn
        .execute_batch(
            "DROP TABLE wrapper_notifications; DROP TABLE wrapper_notification_settings;
        DELETE FROM wrapper_messages WHERE kind='notification_event'",
        )
        .unwrap();
    f.restart();
    assert_eq!(f.get()["unread"], 0);
    let historical: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM wrapper_messages WHERE kind='notification_event'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(historical, 0);
}

#[test]
fn migration_baseline_deleted_target_and_invalid_input() {
    let mut f = Fixture::new();
    f.reply("old");
    f.conn
        .execute_batch("DROP TABLE wrapper_notifications; DROP TABLE wrapper_notification_settings")
        .unwrap();
    f.restart();
    f.settings(json!({"enabled":true}));
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    assert_eq!(f.get()["unread"], 0);
    f.reply("deleted");
    f.get();
    f.conn
        .execute(
            "UPDATE wrapper_conversations SET deleted=1 WHERE id=?1",
            [&f.conversation],
        )
        .unwrap();
    assert_eq!(f.poll(false, false)["unread"], 0);
    let err = f
        .app
        .api(
            "POST",
            "/api/notifications/open",
            br#"{"id":"reply:deleted"}"#,
        )
        .unwrap_err();
    assert_eq!(err.status, 404);
    let err = f
        .app
        .api(
            "POST",
            "/api/notifications/poll",
            br#"{"focused":"yes","visible":true}"#,
        )
        .unwrap_err();
    assert_eq!(err.status, 400);
    let err = f
        .app
        .api(
            "POST",
            "/api/notifications/settings",
            br#"{"enabled":true,"unexpected":true}"#,
        )
        .unwrap_err();
    assert_eq!(err.status, 400);
}

#[test]
fn throttle_defers_distinct_events_and_retries_after_restart_without_duplicates() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("first");
    assert_eq!(
        f.poll(false, false)["deliveries"].as_array().unwrap().len(),
        1
    );
    f.reply("rapid");
    let result = f.poll(false, false);
    assert_eq!(result["deliveries"], json!([]));
    assert_eq!(result["unread"], 2);
    let pending: bool = f
        .conn
        .query_row(
            "SELECT attempted=0 FROM wrapper_notifications WHERE id='reply:rapid'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(pending);
    f.restart();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.conn
        .execute(
            "UPDATE wrapper_notification_throttle SET attempted_at=0",
            [],
        )
        .unwrap();
    let retried = f.poll(false, false);
    assert_eq!(retried["deliveries"].as_array().unwrap().len(), 1);
    assert_eq!(retried["deliveries"][0]["id"], "reply:rapid");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.restart();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

#[test]
fn settings_change_does_not_consume_unrelated_pending_reply() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("still-pending");
    f.settings(json!({"secret":false}));
    assert_eq!(
        f.poll(false, false)["deliveries"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn simulated_reply_end_to_end_and_second_client_cannot_redeliver() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    let conversation = f.conversation.clone();
    f.post(
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"real-send","body":"Bonjour"}),
    );
    let started = std::time::Instant::now();
    loop {
        f.app.tick(f.app.feature_context().now_ms);
        if f.get()["unread"].as_u64().unwrap() > 0 {
            break;
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let first = f.poll(false, false);
    assert_eq!(first["deliveries"][0]["card_id"], "reply-real-send");
    let mut second = App::open(&f.dir.join("workspace.sqlite")).unwrap();
    assert_eq!(
        second
            .api(
                "POST",
                "/api/notifications/poll",
                br#"{"focused":false,"visible":false}"#
            )
            .unwrap()["deliveries"],
        json!([])
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert!(state["conversations"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == "reply-real-send"));
}

#[test]
fn routines_never_write_transcript_even_when_notifications_are_disabled() {
    for enabled in [false, true] {
        let mut f = Fixture::new();
        f.settings(json!({"enabled":enabled}));
        f.reply("existing-run");
        let before = f.message_count();
        f.routine("done", "done", Some("existing-run"));
        f.routine("failed-before-run", "failed", None);
        f.poll(false, false);
        f.restart();
        f.poll(false, false);
        assert_eq!(f.message_count(), before);
        let open = f.post(
            "/api/notifications/open",
            json!({"id":"routine:failed-before-run"}),
        );
        assert_eq!(open["card_id"], "");
        assert_eq!(open["conversation_id"], f.conversation);
    }
}

#[test]
fn upgrade_removes_only_old_notification_bubbles_and_repairs_clicks() {
    let mut f = Fixture::new();
    f.routine("pre-run", "failed", None);
    f.get();
    f.conn.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,created_at,kind) VALUES ('local-workspace',?1,'routine:pre-run',1000,'notification_event')",[&f.conversation]).unwrap();
    f.conn.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,created_at,kind) VALUES ('local-workspace',?1,'genuine',1000,'message')",[&f.conversation]).unwrap();
    f.conn.execute_batch("DELETE FROM component_versions WHERE component='wrapper.notifications'; UPDATE wrapper_notifications SET card_id='routine:pre-run' WHERE kind='routine'").unwrap();
    f.restart();
    assert_eq!(f.message_count(), 1);
    assert_eq!(
        f.conn
            .query_row(
                "SELECT kind FROM wrapper_messages WHERE message_id='genuine'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "message"
    );
    assert_eq!(
        f.post("/api/notifications/open", json!({"id":"routine:pre-run"}))["card_id"],
        ""
    );
    f.restart();
    assert_eq!(f.message_count(), 1);
}

#[test]
fn resolved_throttled_request_is_not_delivered_later() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.request("first", "approval");
    assert_eq!(
        f.poll(false, false)["deliveries"].as_array().unwrap().len(),
        1
    );
    f.request("second", "approval");
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.conn
        .execute(
            "UPDATE forms_requests SET status='denied' WHERE id='second'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE wrapper_notification_throttle SET attempted_at=0",
            [],
        )
        .unwrap();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

#[test]
fn thousand_turns_are_indexed_incremental_and_not_projected_on_host_ticks() {
    let mut f = Fixture::new();
    for n in 0..1000 {
        f.reply(&format!("history-{n}"));
    }
    grokbot_wrapper::features::notifications::tick(&f.app.feature_context());
    assert_eq!(
        f.conn
            .query_row("SELECT count(*) FROM wrapper_notifications", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let start = std::time::Instant::now();
    let first = f.poll(false, false);
    let elapsed = start.elapsed();
    eprintln!("1000 tours, poll froid : {elapsed:?}");
    assert_eq!(first["unread"], 1000);
    // The exact indexed plan is also asserted in the module unit test. This
    // generous ceiling catches the former >1s quadratic query without flaky 50ms CI.
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "poll: {elapsed:?}"
    );
    f.conn.execute_batch("CREATE TRIGGER never_reinsert BEFORE INSERT ON wrapper_notifications WHEN EXISTS(SELECT 1 FROM wrapper_notifications WHERE account_id=NEW.account_id AND id=NEW.id) BEGIN SELECT RAISE(ABORT,'historical source reinserted'); END").unwrap();
    let warm = std::time::Instant::now();
    f.poll(false, false);
    eprintln!("1000 tours, poll chaud : {:?}", warm.elapsed());
    // A new source is still admitted despite the incremental historical filter.
    f.reply("new-source");
    assert_eq!(f.poll(false, false)["unread"], 1001);
}

#[test]
fn storage_error_does_not_claim_or_lose_source() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reply("retry");
    f.conn.execute_batch("CREATE TRIGGER fail_notification BEFORE INSERT ON wrapper_notifications BEGIN SELECT RAISE(ABORT,'test disk failure'); END").unwrap();
    let err = f
        .app
        .api(
            "POST",
            "/api/notifications/poll",
            br#"{"focused":false,"visible":false}"#,
        )
        .unwrap_err();
    assert_eq!(err.status, 500);
    f.conn
        .execute_batch("DROP TRIGGER fail_notification")
        .unwrap();
    assert_eq!(
        f.poll(false, false)["deliveries"].as_array().unwrap().len(),
        1
    );
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

fn titles(result: &Value) -> Vec<String> {
    result["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["title"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_waiting_approval_is_reminded_exactly_once_after_the_delay() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.waiting_approval("a", 10 * MINUTE);
    assert_eq!(titles(&f.poll(false, false)), ["Autorisation demandée"]);
    // 59 minutes: too early. 1 h 01: exactly one reminder, then nothing more.
    f.age("a", 59 * MINUTE);
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.age("a", 61 * MINUTE);
    let result = f.poll(false, false);
    assert_eq!(
        titles(&result),
        ["Une demande attend votre décision depuis 1 h"]
    );
    let reminder = &result["deliveries"][0];
    assert_eq!(reminder["kind"], "approval");
    assert_eq!(reminder["conversation_id"], f.conversation);
    assert_eq!(reminder["card_id"], "a");
    assert!(!result.to_string().contains("NE_JAMAIS"));
    f.conn
        .execute(
            "UPDATE wrapper_notification_throttle SET attempted_at=0",
            [],
        )
        .unwrap();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    f.age("a", 5 * HOUR);
    f.restart();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}

#[test]
fn the_reminder_delay_accepts_30_minutes_1_hour_and_3_hours_and_never_stays_silent() {
    for (choice, phrase, age) in [
        ("30min", "30 min", 31 * MINUTE),
        ("1h", "1 h", 61 * MINUTE),
        ("3h", "3 h", 181 * MINUTE),
    ] {
        let mut f = Fixture::new();
        f.settings(json!({"enabled":true}));
        f.reminder(choice);
        f.waiting_approval("a", MINUTE);
        assert_eq!(
            titles(&f.poll(false, false)),
            ["Autorisation demandée"],
            "{choice}"
        );
        f.age("a", age - 2 * MINUTE);
        assert_eq!(
            f.poll(false, false)["deliveries"],
            json!([]),
            "{choice} trop tôt"
        );
        f.age("a", age);
        assert_eq!(
            titles(&f.poll(false, false)),
            [format!("Une demande attend votre décision depuis {phrase}")],
            "{choice}"
        );
    }
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.reminder("never");
    f.waiting_approval("a", MINUTE);
    assert_eq!(titles(&f.poll(false, false)), ["Autorisation demandée"]);
    f.age("a", 10 * HOUR);
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    assert_eq!(f.get()["unread"], 1);
}

#[test]
fn do_not_disturb_or_disabled_notifications_send_no_reminder_but_the_card_keeps_waiting() {
    for change in [
        json!({"dnd":true}),
        json!({"enabled":false}),
        json!({"approval":false}),
    ] {
        let mut f = Fixture::new();
        f.settings(json!({"enabled":true}));
        f.waiting_approval("a", MINUTE);
        assert_eq!(titles(&f.poll(false, false)), ["Autorisation demandée"]);
        f.settings(change.clone());
        f.age("a", 2 * HOUR);
        assert_eq!(f.poll(false, false)["deliveries"], json!([]), "{change}");
        // Switching alerts back on never replays the muted reminder.
        f.settings(json!({"enabled":true,"dnd":false,"approval":true}));
        assert_eq!(f.poll(false, false)["deliveries"], json!([]), "{change}");
        let pending: String = f
            .conn
            .query_row("SELECT status FROM forms_requests WHERE id='a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pending, "pending");
    }
}

#[test]
fn an_approval_decided_or_expired_before_the_delay_sends_no_reminder() {
    let mut f = Fixture::new();
    f.settings(json!({"enabled":true}));
    f.waiting_approval("decided", MINUTE);
    f.waiting_approval("expired", MINUTE);
    // The throttle spaces two notices of one conversation: one per poll.
    for _ in 0..2 {
        assert_eq!(
            f.poll(false, false)["deliveries"].as_array().unwrap().len(),
            1
        );
        f.conn
            .execute(
                "UPDATE wrapper_notification_throttle SET attempted_at=0",
                [],
            )
            .unwrap();
    }
    f.conn
        .execute(
            "UPDATE forms_requests SET status='resolved' WHERE id='decided'",
            [],
        )
        .unwrap();
    f.conn.execute("UPDATE forms_requests SET kind_json=json_set(kind_json,'$.expires_at',?1) WHERE id='expired'", [now_ms() - 1]).unwrap();
    f.age("decided", 2 * HOUR);
    f.age("expired", 2 * HOUR);
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
    // A reminder projected just before the decision is not posted either.
    f.waiting_approval("late", MINUTE);
    f.poll(false, false);
    f.age("late", 2 * HOUR);
    f.get();
    f.conn
        .execute(
            "UPDATE forms_requests SET status='denied' WHERE id='late'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE wrapper_notification_throttle SET attempted_at=0",
            [],
        )
        .unwrap();
    assert_eq!(f.poll(false, false)["deliveries"], json!([]));
}
