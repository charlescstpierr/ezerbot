//! Current-work API against the real local journals; no model, network or UI.
use grokbot_engine::{
    contracts::{
        with_tx, RunBudgets, RunKind, SuspendCause, TaskStatus, ToolCall, Trigger, TurnEnd,
    },
    model::{self, Run, RunStatus},
    store,
};
use grokbot_wrapper::{
    features::{self, forms, Context},
    App,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    app: App,
    conn: Connection,
    dir: PathBuf,
    conversation: String,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tasks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.sqlite");
        let mut app = App::open(&path).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let conn = Connection::open(path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        Self {
            app,
            conn,
            dir,
            conversation,
        }
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn items(&mut self) -> Vec<Value> {
        self.app.api("GET", "/api/taches", b"").unwrap()["items"]
            .as_array()
            .unwrap()
            .clone()
    }
    fn run(&self, id: &str, trigger: Trigger, started_at: i64) {
        let task = !matches!(trigger, Trigger::UserMessage { .. });
        with_tx(&self.conn, |c| {
            model::start(
                c,
                &Run {
                    run_id: id.into(),
                    bot_id: "grokbot".into(),
                    conversation_id: self.conversation.clone(),
                    trigger,
                    kind: if task { RunKind::Task } else { RunKind::Chat },
                    status: RunStatus::Queued,
                    task_status: task.then_some(TaskStatus::Working),
                    turn_end: None,
                    budgets: RunBudgets::default(),
                    started_at,
                },
            )?;
            model::set_running_at(c, id, started_at)
        })
        .unwrap();
    }
    fn turn(&self, id: &str, started_at: i64) {
        self.run(
            id,
            Trigger::UserMessage {
                message_id: format!("message-{id}"),
                attempt: 0,
            },
            started_at,
        );
    }
    fn status(&self, id: &str, status: RunStatus) {
        with_tx(&self.conn, |c| {
            let mut run = store::required(c, id)?;
            run.status = status;
            run.turn_end = match status {
                RunStatus::Finished => Some(TurnEnd::Finished),
                RunStatus::Cancelled => Some(TurnEnd::Stopped),
                _ => None,
            };
            if status == RunStatus::Finished && run.kind == RunKind::Task {
                run.task_status = Some(TaskStatus::Done {
                    summary: "PRIVATE_RESULT".into(),
                });
            }
            store::save(c, &run)
        })
        .unwrap();
    }
    fn shell(&self) {
        self.turn("shell-run", 1000);
        self.status("shell-run", RunStatus::Finished);
        self.conn.execute("INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES ('shell-run','call','en cours')", []).unwrap();
        self.conn.execute("INSERT INTO engine_external_tools(run_id,call_id,outcome) VALUES ('shell-run','call',?1)", [json!({"status":"done","output":{"status":"en cours","stdout":"PRIVATE_OUTPUT"}}).to_string()]).unwrap();
    }
    fn children(&self) {
        self.turn("parent", 2000);
        self.run(
            "child",
            Trigger::Delegation {
                delegation_id: "edge".into(),
            },
            3000,
        );
        self.conn.execute("INSERT INTO delegation_graph(id,parent_run_id,root_run_id,parent_bot,child_bot,depth,child_run_id,conversation_id,selected_context,status) VALUES ('edge','parent','parent','grokbot','grokbot',1,'child',NULL,'{\"texts\":[],\"entry_ids\":[],\"fact_ids\":[],\"skill_ids\":[]}','{\"status\":\"running\"}')", []).unwrap();
        self.run(
            "routine-run",
            Trigger::Occurrence {
                occurrence_id: "occurrence".into(),
            },
            4000,
        );
        self.conn.execute("INSERT INTO scheduler_routines(routine_id,bot_id,conversation_id,name,body,trigger,time_zone,enabled,origin,untrusted,schedule_version,modified_at,materialized_after,active_since) VALUES ('routine','grokbot',?1,'PRIVATE_ROUTINE','{}','{}','UTC',0,'user',0,1,0,0,0)", [&self.conversation]).unwrap();
        self.conn.execute("INSERT INTO scheduler_occurrences(occurrence_id,routine_id,schedule_version,scheduled_at_utc,manual_nonce,status,run_id) VALUES ('occurrence','routine',1,4000,'nonce','running','routine-run')", []).unwrap();
    }
    fn request(&self, id: &str, kind: Value) {
        self.conn.execute("INSERT INTO forms_requests(id,run_id,entry_id,kind_json,route_json,boot_id,created_at) VALUES (?1,'parent',?1,?2,?3,(SELECT boot_id FROM wrapper_forms_boot WHERE singleton=1),5000)", params![id,kind.to_string(),json!({"target_conversation_id":self.conversation}).to_string()]).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn approval_is_first_has_its_card_and_disappears_after_a_real_decision() {
    let mut f = Fixture::new();
    f.post("/api/yolo/mode", json!({"mode":"ask"}));
    f.turn("turn", 1000);
    f.shell();
    let run = store::required(&f.conn, "turn").unwrap();
    let tx = features::write_transaction(&f.conn).unwrap();
    let ctx = Context {
        connection: &tx,
        run_id: Some(&run.run_id),
        bot_id: Some(&run.bot_id),
        conversation_id: Some(&run.conversation_id),
        ..f.app.feature_context()
    };
    let cause = forms::request_tool_approval(
        &ctx,
        &run,
        &ToolCall {
            call_id: "approval".into(),
            name: "run_shell".into(),
            arguments: json!({"command":"PRIVATE_ARGUMENTS","cwd":"/tmp"}),
        },
    )
    .unwrap();
    let SuspendCause::HumanInput { id } = cause else {
        panic!("missing request")
    };
    let request = grokbot_forms::requests::get(&tx, &id).unwrap().unwrap();
    tx.commit().unwrap();
    let items = f.items();
    assert_eq!(items[0]["group"], "decision");
    assert_eq!(items[0]["request"]["id"], id);
    assert_eq!(items[0]["request"]["card_id"], request.entry_id);
    assert_eq!(items[0]["bot_id"], "grokbot");
    assert_eq!(items[0]["conversation_id"], f.conversation);
    assert!(!json!(items).to_string().contains("PRIVATE_"));
    f.post("/api/approvals/resolve", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":f.conversation,"decision":"deny"}));
    assert!(f.items().iter().all(|i| i["request"]["id"] != id));
}

#[test]
fn background_stop_and_restart_never_leave_running_ghosts() {
    for restart in [false, true] {
        let mut f = Fixture::new();
        f.shell();
        f.children();
        let shell_id = "shell:shell-run/call";
        assert!(f
            .items()
            .iter()
            .any(|i| i["id"] == shell_id && i["group"] == "running"));
        if restart {
            f.app = App::open(&f.dir.join("workspace.sqlite")).unwrap();
            assert!(
                f.items().iter().all(|i| i["group"] != "running"),
                "{:?}",
                f.items()
            );
        } else {
            f.post(
                "/api/local/tasks/stop",
                json!({"run_id":"shell-run","call_id":"call"}),
            );
        }
        let shell = f.items().into_iter().find(|i| i["id"] == shell_id).unwrap();
        assert_eq!(shell["group"], "recent");
        assert_eq!(shell["status"], "stopped");
    }
}

#[test]
fn finished_children_are_not_running_even_before_their_hosts_reconcile() {
    let mut f = Fixture::new();
    f.children();
    f.status("child", RunStatus::Finished);
    f.status("routine-run", RunStatus::Finished);
    for kind in ["delegation", "routine"] {
        let items = f.items();
        let item = items.iter().find(|i| i["kind"] == kind).unwrap();
        assert_eq!(item["group"], "recent");
        assert_eq!(item["status"], "finished");
    }
}

#[test]
fn a_task_turn_ending_without_complete_task_is_not_success_or_running() {
    let mut f = Fixture::new();
    f.turn("incomplete", 1000);
    with_tx(&f.conn, |c| {
        let mut run = store::required(c, "incomplete")?;
        run.kind = RunKind::Task;
        run.task_status = Some(TaskStatus::Working);
        store::save(c, &run)?;
        run.status = RunStatus::Finished;
        run.turn_end = Some(TurnEnd::Finished);
        store::save(c, &run)
    })
    .unwrap();
    let items = f.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["group"], "recent");
    assert_eq!(items[0]["status"], "stopped");
}

#[test]
fn reconciled_host_terminal_states_remain_authoritative() {
    let mut f = Fixture::new();
    f.children();
    f.status("child", RunStatus::Finished);
    f.status("routine-run", RunStatus::Finished);
    f.conn
        .execute(
            "UPDATE delegation_graph SET status='{\"status\":\"cancelled\"}'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE scheduler_occurrences SET status='failed',reason='PRIVATE_REASON'",
            [],
        )
        .unwrap();
    let items = f.items();
    assert_eq!(
        items.iter().find(|i| i["kind"] == "delegation").unwrap()["status"],
        "stopped"
    );
    assert_eq!(
        items.iter().find(|i| i["kind"] == "routine").unwrap()["status"],
        "failed"
    );
    assert!(!json!(items).to_string().contains("PRIVATE_"));
}

#[test]
fn child_and_occurrence_are_each_projected_once_and_get_is_read_only() {
    let mut f = Fixture::new();
    f.children();
    let revision = f.app.api("GET", "/api/ping", b"").unwrap();
    let items = f.items();
    assert_eq!(items.len(), 3);
    for kind in ["delegation", "routine"] {
        let rows: Vec<_> = items.iter().filter(|i| i["kind"] == kind).collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["group"], "running");
        assert_eq!(rows[0]["conversation_id"], f.conversation);
    }
    assert_eq!(f.items(), items);
    assert_eq!(f.app.api("GET", "/api/ping", b"").unwrap(), revision);
    assert!(f.app.api("POST", "/api/taches", b"{}").is_err());
}

#[test]
fn pending_metadata_never_contains_form_text_tool_arguments_or_secret_names_or_values() {
    let mut f = Fixture::new();
    f.children();
    for (id, kind) in [
        (
            "approval",
            json!({"kind":"approval","tool":"PRIVATE_TOOL","arguments":{"value":"PRIVATE_ARGUMENTS"},"expires_at":i64::MAX}),
        ),
        (
            "form",
            json!({"kind":"form","fields":[{"name":"PRIVATE_FIELD","label":"PRIVATE_FORM_TEXT"}]}),
        ),
        (
            "secret",
            json!({"kind":"secret","name":"PRIVATE_SECRET_NAME","purpose":"PRIVATE_PURPOSE","value":"PRIVATE_SECRET_VALUE"}),
        ),
    ] {
        f.request(id, kind);
    }
    let items = f.items();
    assert_eq!(items.iter().filter(|i| i["group"] == "decision").count(), 3);
    assert!(!json!(items).to_string().contains("PRIVATE_"));
    f.conn
        .execute("UPDATE forms_requests SET status='resolved'", [])
        .unwrap();
    assert!(f.items().iter().all(|i| i["group"] != "decision"));
}

#[test]
fn expired_replaced_and_previous_boot_secret_cards_are_not_decisions() {
    let mut f = Fixture::new();
    f.children();
    f.request("expired", json!({"kind":"approval","expires_at":1}));
    f.request("replaced", json!({"kind":"approval"}));
    f.request(
        "stale",
        json!({"kind":"secret","name":"PRIVATE_SECRET_NAME"}),
    );
    f.conn
        .execute(
            "UPDATE forms_requests SET replaced=1 WHERE id='replaced'",
            [],
        )
        .unwrap();
    f.conn
        .execute(
            "UPDATE forms_requests SET boot_id='old' WHERE id='stale'",
            [],
        )
        .unwrap();
    assert!(f.items().iter().all(|i| i["group"] != "decision"));
}

#[test]
fn deleted_bots_conversations_and_delegation_destinations_leave_no_orphans() {
    for target in ["conversation", "bot", "child"] {
        let mut f = Fixture::new();
        f.shell();
        f.children();
        f.request("request", json!({"kind":"form","fields":[]}));
        assert!(!f.items().is_empty());
        match target {
            "conversation" => {
                f.post("/api/conversations/delete", json!({"id":f.conversation}));
            }
            "bot" => {
                f.conn
                    .execute("UPDATE wrapper_bots SET deleted=1 WHERE id='grokbot'", [])
                    .unwrap();
            }
            _ => {
                let child = f.post("/api/conversations", json!({}))["conversation"]["id"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                f.conn
                    .execute("UPDATE delegation_graph SET conversation_id=?1", [&child])
                    .unwrap();
                f.post("/api/conversations/delete", json!({"id":child}));
            }
        }
        let items = f.items();
        if target == "child" {
            assert!(items.iter().all(|i| i["kind"] != "delegation"));
        } else {
            assert!(items.is_empty(), "{target}: {items:?}");
        }
    }
}

#[test]
fn recent_is_capped_at_twenty_and_sorted_by_completion_not_start_and_survives_restart() {
    let mut f = Fixture::new();
    for n in 0..25 {
        let id = format!("turn-{n:02}");
        f.turn(&id, 1000 + n);
        f.status(&id, RunStatus::Finished);
        // Control the durable clock rather than sleep between completions.
        f.conn
            .execute(
                "UPDATE wrapper_task_clock SET changed_at=?2 WHERE source='run' AND id=?1",
                params![id, 2000 + n],
            )
            .unwrap();
    }
    f.turn("old-but-last", 1);
    f.conn
        .execute(
            "UPDATE wrapper_task_clock SET changed_at=1 WHERE id='old-but-last'",
            [],
        )
        .unwrap();
    f.status("old-but-last", RunStatus::Failed);
    let changed: i64 = f
        .conn
        .query_row(
            "SELECT changed_at FROM wrapper_task_clock WHERE source='run' AND id='old-but-last'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(changed > 1, "terminal transition must update the clock");
    f.conn
        .execute(
            "UPDATE wrapper_task_clock SET changed_at=3000 WHERE id='old-but-last'",
            [],
        )
        .unwrap();
    let items = f.items();
    assert_eq!(items.len(), 20);
    assert_eq!(items[0]["id"], "task:old-but-last");
    assert_eq!(items[0]["status"], "failed");
    assert_eq!(items[1]["id"], "task:turn-24");
    assert!(items.iter().all(|i| i["group"] == "recent"));
    f.app = App::open(&f.dir.join("workspace.sqlite")).unwrap();
    assert_eq!(f.items(), items);
}
