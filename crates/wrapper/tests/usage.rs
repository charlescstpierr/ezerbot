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
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen};

struct Fixture {
    dir: PathBuf,
    server: FakeOpenAi,
    provider: Provider,
    _dir: grokbot_test_support::TestDir,
}
impl Fixture {
    fn new(reply: ScriptedResponse) -> Self {
        let guard = grokbot_test_support::TestDir::new("ezerbot-usage");
        let dir = guard.to_path_buf();
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
            _dir: guard,
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
        started_at: 1,
    };
    conn.execute("INSERT INTO engine_runs(run_id,bot_id,conversation_id,trigger_kind,trigger_ref,attempt,status,record) VALUES (?1,?2,?3,?4,?1,0,'finished',?5)", params![id,bot,conversation,trigger_kind,serde_json::to_string(&run).unwrap()]).unwrap();
    if measured {
        let tx = conn.unchecked_transaction().unwrap();
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 20,
            cached_input_tokens: 30,
        };
        store::save_token_usage(&tx, id, 1, &usage, None).unwrap();
        store::save_token_usage(&tx, id, 1, &usage, None).unwrap(); // duplicate telemetry
        store::save_token_usage(&tx, id, 2, &usage, None).unwrap(); // second model response
        tx.commit().unwrap();
    }
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
