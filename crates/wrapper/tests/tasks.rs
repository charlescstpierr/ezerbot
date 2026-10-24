//! Current-work API against the real local journals; no model, network or UI.
use grokbot_engine::{
    contracts::{
        with_tx, RunBudgets, RunEventKind, RunKind, SuspendCause, TaskStatus, ToolCall,
        ToolOutcome, Trigger, TurnEnd,
    },
    model::{self, Run, RunStatus},
    store,
};
use grokbot_wrapper::{
    features::{self, forms, tasks, Context},
    App,
};
use rusqlite::{params, Connection, StatementStatus};
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
    fn run_in(&self, id: &str, bot: &str, conversation: &str, trigger: Trigger) {
        let task = !matches!(trigger, Trigger::UserMessage { .. });
        with_tx(&self.conn, |c| {
            model::start(
                c,
                &Run {
                    run_id: id.into(),
                    bot_id: bot.into(),
                    conversation_id: conversation.into(),
                    trigger,
                    kind: if task { RunKind::Task } else { RunKind::Chat },
                    status: RunStatus::Queued,
                    task_status: task.then_some(TaskStatus::Working),
                    turn_end: None,
                    budgets: RunBudgets::default(),
                    started_at: 1000,
                },
            )?;
            model::set_running_at(c, id, 1000)
        })
        .unwrap();
    }
    /// A delegated child in its own `kind='task'` conversation, linked to `parent`.
    fn delegate(
        &self,
        edge: &str,
        parent: &str,
        parent_bot: &str,
        origin: &str,
        depth: u8,
    ) -> String {
        let conversation = format!("task:{edge}");
        self.conn.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) SELECT account_id,?1,'Sous-tâche','grokbot','task',id,0,0 FROM wrapper_conversations WHERE id=?2", params![conversation, origin]).unwrap();
        self.run_in(
            edge,
            "grokbot",
            &conversation,
            Trigger::Delegation {
                delegation_id: edge.into(),
            },
        );
        self.conn.execute("INSERT INTO delegation_graph(id,parent_run_id,root_run_id,parent_bot,child_bot,depth,child_run_id,conversation_id,selected_context,status) VALUES (?1,?2,'parent',?3,'grokbot',?4,?1,?5,'{\"texts\":[],\"entry_ids\":[],\"fact_ids\":[],\"skill_ids\":[]}','{\"status\":\"running\"}')", params![edge, parent, parent_bot, depth, conversation]).unwrap();
        conversation
    }
    fn background(&self, run: &str, call: &str, status: &str) {
        self.conn
            .execute(
                "INSERT INTO wrapper_shell_tasks(run_id,call_id,status) VALUES (?1,?2,?3)",
                params![run, call, status],
            )
            .unwrap();
    }
    /// A tool call as the engine journals it: the event, its timeline entry `run:seq`, then its result.
    fn call(&self, run: &str, name: &str, arguments: Value) -> ToolOutcome {
        let run = store::required(&self.conn, run).unwrap();
        let call = ToolCall {
            call_id: format!(
                "call-{}",
                store::events(&self.conn, &run.run_id, 0).unwrap().len()
            ),
            name: name.into(),
            arguments,
        };
        let tx = features::write_transaction(&self.conn).unwrap();
        let payload = serde_json::to_value(&call).unwrap();
        let event =
            store::append_event(&tx, &run.run_id, RunEventKind::ToolCall, &payload).unwrap();
        let entry = format!("{}:{}", run.run_id, event.seq);
        features::conversations::append_entry(
            &tx,
            &run.conversation_id,
            "engine_tool",
            &entry,
            5000,
        )
        .unwrap();
        let ctx = Context {
            connection: &tx,
            run_id: Some(&run.run_id),
            bot_id: Some(&run.bot_id),
            conversation_id: Some(&run.conversation_id),
            ..self.app.feature_context()
        };
        let outcome = features::call_tool(&ctx, &call).unwrap();
        let result = json!({"call_id": call.call_id, "outcome": outcome});
        store::append_event(&tx, &run.run_id, RunEventKind::ToolResult, &result).unwrap();
        tx.commit().unwrap();
        outcome
    }
    fn ask(&self, run: &str, question: &str) {
        let outcome = self.call(run, "attendre_reponse", json!({"question": question}));
        assert!(matches!(outcome, ToolOutcome::Done { .. }), "{outcome:?}");
    }
    /// Charles writes in `conversation` (a timeline entry of kind `submission`).
    fn charles_writes(&self, conversation: &str, id: &str) {
        let tx = features::write_transaction(&self.conn).unwrap();
        features::conversations::append_entry(&tx, conversation, "submission", id, 6000).unwrap();
        tx.commit().unwrap();
    }
    fn questions(&mut self) -> Vec<Value> {
        let mut items = self.items();
        items.retain(|i| i["kind"] == "question");
        items
    }
    fn request(&self, id: &str, kind: Value) {
        self.request_on("parent", id, kind);
    }
    fn request_on(&self, run: &str, id: &str, kind: Value) {
        self.conn.execute("INSERT INTO forms_requests(id,run_id,entry_id,kind_json,route_json,boot_id,created_at) VALUES (?1,?2,?1,?3,?4,(SELECT boot_id FROM wrapper_forms_boot WHERE singleton=1),5000)", params![id,run,kind.to_string(),json!({"target_conversation_id":self.conversation}).to_string()]).unwrap();
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
    let mut waiting = run.clone();
    waiting.status = RunStatus::AwaitingApproval;
    store::save(&tx, &waiting).unwrap();
    store::set_awaiting(&tx, "turn", &SuspendCause::HumanInput { id: id.clone() }).unwrap();
    tx.commit().unwrap();
    let items = f.items();
    assert_eq!(items[0]["group"], "decision");
    assert_eq!(items[0]["request"]["id"], id);
    assert_eq!(items[0]["request"]["card_id"], request.entry_id);
    assert_eq!(items[0]["bot_id"], "grokbot");
    assert_eq!(items[0]["conversation_id"], f.conversation);
    assert!(!json!(items).to_string().contains("PRIVATE_"));
    f.post("/api/approvals/resolve", json!({"entry_id":request.entry_id,"submission_id":"decision","conversation_id":f.conversation,"decision":"deny"}));
    let items = f.items();
    assert!(items.iter().all(|i| i["request"]["id"] != id));
    assert_eq!(
        store::required(&f.conn, "turn").unwrap().status,
        RunStatus::AwaitingApproval
    );
    let turn = items.iter().find(|i| i["id"] == "task:turn").unwrap();
    assert_eq!(turn["group"], "running");
    assert_eq!(turn["status"], "running");
}

#[test]
fn deleting_the_origin_of_a_delegation_hides_every_row_its_descendants_left() {
    let mut leaked = Vec::new();
    for row in [
        "running shell",
        "finished shell",
        "request",
        "delegation",
        "late turn shell",
        "grandchild shell",
        "grandchild request",
        "grandchild delegation",
    ] {
        let mut f = Fixture::new();
        f.post(
            "/api/bots",
            json!({"id":"custom","name":"Perso","color":"cyan"}),
        );
        let origin = f.post("/api/conversations", json!({"bot_id":"custom"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.run_in(
            "parent",
            "custom",
            &origin,
            Trigger::UserMessage {
                message_id: "message-parent".into(),
                attempt: 0,
            },
        );
        f.delegate("child", "parent", "custom", &origin, 1);
        f.delegate("grandchild", "child", "grokbot", &origin, 2);
        match row {
            "running shell" => f.background("child", "call", "en cours"),
            "finished shell" => f.background("child", "call", "terminé"),
            "request" => {
                f.status("child", RunStatus::AwaitingApproval);
                f.request_on("child", "approval", json!({"kind":"approval"}));
            }
            "late turn shell" => {
                f.run_in(
                    "late",
                    "grokbot",
                    "task:child",
                    Trigger::UserMessage {
                        message_id: "message-late".into(),
                        attempt: 0,
                    },
                );
                f.background("late", "call", "en cours");
            }
            "grandchild shell" => f.background("grandchild", "call", "en cours"),
            "grandchild request" => {
                f.status("grandchild", RunStatus::AwaitingApproval);
                f.request_on("grandchild", "approval", json!({"kind":"approval"}));
            }
            _ => {}
        }
        let before = f.items();
        assert!(
            before.iter().any(|i| match row {
                "running shell" | "finished shell" => i["id"] == "shell:child/call",
                "request" | "grandchild request" => i["id"] == "input:approval",
                "late turn shell" => i["id"] == "shell:late/call",
                "grandchild shell" => i["id"] == "shell:grandchild/call",
                "grandchild delegation" => i["id"] == "delegation:grandchild",
                _ => i["id"] == "delegation:child",
            }),
            "{row}: {before:?}"
        );

        f.post("/api/conversations/delete", json!({"id":origin}));
        let live: i64 = f.conn.query_row("SELECT COUNT(*) FROM wrapper_conversations WHERE id IN ('task:child','task:grandchild') AND deleted=0", [], |r| r.get(0)).unwrap();
        assert_eq!(live, 2);
        let revision = f.app.api("GET", "/api/ping", b"").unwrap();
        let items = f.items();
        if !items.is_empty() {
            leaked.push(format!(
                "{row}: {:?}",
                items.iter().map(|i| &i["id"]).collect::<Vec<_>>()
            ));
        }
        assert_eq!(f.items(), items);
        assert_eq!(f.app.api("GET", "/api/ping", b"").unwrap(), revision);
    }
    assert!(leaked.is_empty(), "{leaked:#?}");
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
fn a_routed_delegation_request_disappears_with_its_custom_bot_parent_conversation() {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"custom","name":"Perso","color":"cyan"}),
    );
    let origin = f.post("/api/conversations", json!({"bot_id":"custom"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.run_in(
        "parent",
        "custom",
        &origin,
        Trigger::UserMessage {
            message_id: "message-parent".into(),
            attempt: 0,
        },
    );
    let child = f.delegate("edge", "parent", "custom", &origin, 1);
    f.status("edge", RunStatus::AwaitingApproval);
    f.request_on("edge", "approval", json!({"kind":"approval"}));
    let items = f.items();
    let decisions: Vec<_> = items.iter().filter(|i| i["group"] == "decision").collect();
    assert_eq!(decisions.len(), 1, "{items:?}");
    assert_eq!(decisions[0]["kind"], "delegation");
    assert_eq!(decisions[0]["conversation_id"], f.conversation);
    assert_eq!(decisions[0]["request"]["card_id"], "approval");
    assert!(items.iter().all(|i| i["id"] != "delegation:edge"));

    f.post("/api/conversations/delete", json!({"id":origin}));
    let live: i64 = f
        .conn
        .query_row(
            "SELECT COUNT(*) FROM wrapper_conversations WHERE id IN (?1,?2) AND deleted=0",
            params![child, f.conversation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, 2);
    let pending: String = f
        .conn
        .query_row(
            "SELECT status FROM forms_requests WHERE id='approval'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, "pending");
    let revision = f.app.api("GET", "/api/ping", b"").unwrap();
    let items = f.items();
    assert!(items.is_empty(), "{items:?}");
    assert_eq!(f.items(), items);
    assert_eq!(f.app.api("GET", "/api/ping", b"").unwrap(), revision);
    let pending: String = f
        .conn
        .query_row(
            "SELECT status FROM forms_requests WHERE id='approval'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, "pending");
}

#[test]
fn runs_waiting_on_a_previous_boot_secret_are_stopped_after_restart_not_running_or_hidden() {
    let mut f = Fixture::new();
    f.children();
    for run in ["parent", "child", "routine-run"] {
        f.status(run, RunStatus::AwaitingHumanInput);
        f.request_on(
            run,
            &format!("secret-{run}"),
            json!({"kind":"secret","name":"PRIVATE_SECRET_NAME"}),
        );
    }
    let items = f.items();
    assert_eq!(items.len(), 3, "{items:?}");
    assert!(items.iter().all(|i| i["group"] == "decision"));
    f.app = App::open(&f.dir.join("workspace.sqlite")).unwrap();
    let items = f.items();
    for kind in ["task", "delegation", "routine"] {
        let rows: Vec<_> = items.iter().filter(|i| i["kind"] == kind).collect();
        assert_eq!(rows.len(), 1, "{kind}: {items:?}");
        assert_eq!(rows[0]["group"], "recent");
        assert_eq!(rows[0]["status"], "stopped");
        assert_eq!(rows[0]["request"], Value::Null);
    }
    assert_eq!(items.len(), 3);
    assert!(!json!(items).to_string().contains("PRIVATE_"));
}

#[test]
fn recent_is_capped_at_twenty_and_sorted_by_recorded_start_and_survives_restart() {
    let mut f = Fixture::new();
    for n in 0..25 {
        let id = format!("turn-{n:02}");
        f.turn(&id, 1000 + n);
        f.status(&id, RunStatus::Finished);
    }
    f.turn("still-running", 1);
    f.turn("latest-failure", 5000);
    f.status("latest-failure", RunStatus::Failed);
    let items = f.items();
    assert_eq!(items.len(), 21);
    assert_eq!(items[0]["id"], "task:still-running");
    assert_eq!(items[0]["group"], "running");
    let recent = &items[1..];
    assert!(recent.iter().all(|i| i["group"] == "recent"));
    assert_eq!(recent[0]["id"], "task:latest-failure");
    assert_eq!(recent[0]["status"], "failed");
    assert_eq!(recent[1]["id"], "task:turn-24");
    assert_eq!(recent[19]["id"], "task:turn-06");
    f.app = App::open(&f.dir.join("workspace.sqlite")).unwrap();
    assert_eq!(f.items(), recent);
}

/// SQLite VM instructions of one `GET /api/taches` over `n` finished delegations on two bots.
fn snapshot_cost(n: usize) -> (i32, usize) {
    let mut f = Fixture::new();
    f.post(
        "/api/bots",
        json!({"id":"custom","name":"Perso","color":"cyan"}),
    );
    let edge = r#"{"texts":[],"entry_ids":[],"fact_ids":[],"skill_ids":[]}"#;
    with_tx(&f.conn, |c| {
        for i in 0..n {
            let bot = if i % 2 == 0 { "grokbot" } else { "custom" };
            let (parent, child, task) = (format!("p{i}"), format!("c{i}"), format!("task:c{i}"));
            c.execute("INSERT INTO wrapper_conversations(account_id,id,title,bot_id,kind,parent_id,created_at,updated_at) SELECT account_id,?1,'Sous-tâche','grokbot','task',id,0,0 FROM wrapper_conversations WHERE id=?2", params![task, f.conversation])?;
            for (id, conversation, trigger) in [
                (
                    &parent,
                    f.conversation.clone(),
                    Trigger::UserMessage {
                        message_id: format!("message-{i}"),
                        attempt: 0,
                    },
                ),
                (
                    &child,
                    task.clone(),
                    Trigger::Delegation {
                        delegation_id: child.clone(),
                    },
                ),
            ] {
                let chat = id == &parent;
                model::start(
                    c,
                    &Run {
                        run_id: id.clone(),
                        bot_id: if chat { bot } else { "grokbot" }.into(),
                        conversation_id: conversation,
                        trigger,
                        kind: if chat { RunKind::Chat } else { RunKind::Task },
                        status: RunStatus::Queued,
                        task_status: (!chat).then_some(TaskStatus::Working),
                        turn_end: None,
                        budgets: RunBudgets::default(),
                        started_at: 1000 + i as i64,
                    },
                )?;
            }
            c.execute("INSERT INTO delegation_graph(id,parent_run_id,root_run_id,parent_bot,child_bot,depth,child_run_id,conversation_id,selected_context,status) VALUES (?1,?2,?2,?3,'grokbot',1,?1,?4,?5,'{\"status\":\"done\"}')", params![child, parent, bot, task, edge])?;
            let ask = json!({"call_id":"ask","name":"attendre_reponse","arguments":{"question":"Quelle option ?"}});
            let event = store::append_event(c, &parent, RunEventKind::ToolCall, &ask)?;
            let done = json!({"call_id":"ask","outcome":ToolOutcome::Done{output:json!({})}});
            store::append_event(c, &parent, RunEventKind::ToolResult, &done)?;
            let entry = format!("{parent}:{}", event.seq);
            features::conversations::append_entry(c, &f.conversation, "engine_tool", &entry, 1).unwrap();
        }
        c.execute("UPDATE engine_runs SET status='finished'", [])?;
        Ok::<_, grokbot_engine::contracts::Error>(())
    })
    .unwrap();
    let mut statement = f.conn.prepare(tasks::SNAPSHOT).unwrap();
    let rows = statement
        .query_map(params!["local-workspace", 2_000_000_000i64], |_| Ok(()))
        .unwrap()
        .count();
    (statement.get_status(StatementStatus::VmStep), rows)
}

#[test]
fn the_snapshot_cost_grows_linearly_with_finished_delegations() {
    let (small, rows) = snapshot_cost(250);
    let (large, _) = snapshot_cost(500);
    // The 20 most recent ends, and the one open question of the conversation.
    assert_eq!(rows, 21);
    assert!(
        large <= 3 * small,
        "250 delegations: {small} VM instructions, 500: {large}"
    );
}

#[test]
fn the_tool_is_in_the_catalog_and_a_question_appears_in_to_decide() {
    assert!(features::tools()
        .iter()
        .any(|t| t.name == "attendre_reponse"));
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    f.ask("turn", "Je prends quelle option ?");
    let items = f.items();
    assert_eq!(items[0]["id"], format!("question:{}", f.conversation));
    assert_eq!(items[0]["kind"], "question");
    assert_eq!(items[0]["group"], "decision");
    assert_eq!(items[0]["status"], "pending");
    assert_eq!(items[0]["bot_id"], "grokbot");
    assert_eq!(items[0]["conversation_id"], f.conversation);
    assert_eq!(items[0]["question"], "Je prends quelle option ?");
    assert_eq!(items[0]["started_at"], 5000);
    assert_eq!(items[0]["request"], Value::Null);
}

#[test]
fn a_reply_ending_with_a_question_mark_without_the_tool_creates_no_line() {
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    with_tx(&f.conn, |c| {
        store::append_event(
            c,
            "turn",
            RunEventKind::TextDelta,
            &json!({"text":"Je prends quelle option ?","response":1}),
        )
        .map(|_| ())
    })
    .unwrap();
    f.status("turn", RunStatus::Finished);
    assert!(f.questions().is_empty());
}

#[test]
fn a_message_from_charles_removes_the_question_until_the_bot_asks_again() {
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    f.ask("turn", "Quelle option ?");
    assert_eq!(f.questions().len(), 1);
    f.charles_writes(&f.conversation, "answer");
    assert!(f.questions().is_empty());
    f.ask("turn", "Et pour la suite ?");
    assert_eq!(f.questions()[0]["question"], "Et pour la suite ?");
}

#[test]
fn a_reset_thread_or_a_deleted_conversation_or_bot_leaves_no_question() {
    for target in ["reset", "conversation", "bot"] {
        let mut f = Fixture::new();
        f.turn("turn", 1000);
        f.ask("turn", "Quelle option ?");
        assert_eq!(f.questions().len(), 1);
        match target {
            "reset" => {
                f.post(
                    "/api/conversations/reset",
                    json!({"conversation_id":f.conversation}),
                );
            }
            "conversation" => {
                f.post("/api/conversations/delete", json!({"id":f.conversation}));
            }
            _ => {
                f.conn
                    .execute("UPDATE wrapper_bots SET deleted=1 WHERE id='grokbot'", [])
                    .unwrap();
            }
        }
        assert!(f.questions().is_empty(), "{target}");
    }
}

#[test]
fn two_calls_in_one_conversation_give_one_line_with_the_last_question() {
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    f.ask("turn", "Première question ?");
    f.ask("turn", "Seconde question ?");
    let questions = f.questions();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0]["question"], "Seconde question ?");
}

#[test]
fn only_the_masked_and_truncated_question_is_returned_never_arguments_or_secrets() {
    let mut f = Fixture::new();
    let secret = "SECRET-TOKEN-VALUE-0123456789";
    forms::vault(&f.app.feature_context())
        .unwrap()
        .set("TOKEN", "purpose", secret)
        .unwrap();
    f.turn("turn", 1000);
    // The secret starts before the 140th character and ends after it: cutting first would leave a prefix.
    let question = format!("{}{secret} et la fin ?", "a".repeat(130));
    let outcome = f.call(
        "turn",
        "attendre_reponse",
        json!({"question": question, "PRIVATE": "PRIVATE_ARGUMENTS"}),
    );
    assert!(matches!(outcome, ToolOutcome::Done { .. }));
    let questions = f.questions();
    let shown = questions[0]["question"].as_str().unwrap();
    assert!(shown.chars().count() <= 140, "{shown}");
    assert!(!shown.contains("SECRET"), "{shown}");
    let mut keys: Vec<_> = questions[0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "bot_id",
            "conversation_id",
            "group",
            "id",
            "kind",
            "question",
            "request",
            "started_at",
            "status",
            "title"
        ]
    );
    assert!(!json!(f.items()).to_string().contains("PRIVATE"));
}

#[test]
fn a_blank_question_or_a_delegated_run_is_refused_and_creates_no_line() {
    let mut f = Fixture::new();
    f.turn("parent", 1000);
    f.delegate("edge", "parent", "grokbot", &f.conversation, 1);
    for (run, arguments) in [
        ("parent", json!({"question": "  "})),
        ("parent", json!({})),
        ("edge", json!({"question": "Puis-je ?"})),
    ] {
        assert!(matches!(
            f.call(run, "attendre_reponse", arguments),
            ToolOutcome::Failed { .. }
        ));
    }
    assert!(f.questions().is_empty());
}

#[test]
fn a_question_in_a_task_thread_follows_the_visibility_of_its_delegation_origin() {
    for target in ["conversation", "bot"] {
        let mut f = Fixture::new();
        f.post(
            "/api/bots",
            json!({"id":"custom","name":"Perso","color":"cyan"}),
        );
        let origin = f.post("/api/conversations", json!({"bot_id":"custom"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        f.run_in(
            "parent",
            "custom",
            &origin,
            Trigger::UserMessage {
                message_id: "message-parent".into(),
                attempt: 0,
            },
        );
        let task = f.delegate("edge", "parent", "custom", &origin, 1);
        f.run_in(
            "late",
            "grokbot",
            &task,
            Trigger::UserMessage {
                message_id: "message-late".into(),
                attempt: 0,
            },
        );
        f.ask("late", "Je continue ?");
        assert_eq!(f.questions().len(), 1, "{target}");
        if target == "conversation" {
            f.post("/api/conversations/delete", json!({"id":origin}));
        } else {
            f.conn
                .execute("UPDATE wrapper_bots SET deleted=1 WHERE id='custom'", [])
                .unwrap();
        }
        assert!(f.questions().is_empty(), "{target}");
    }
}

#[test]
fn a_refused_blank_call_neither_creates_nor_replaces_a_question() {
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    // Every character the tool treats as blank, since the query has its own list of them.
    for blank in (0..=0x10FFFF)
        .filter_map(char::from_u32)
        .filter(|c| c.is_whitespace())
    {
        let outcome = f.call(
            "turn",
            "attendre_reponse",
            json!({"question": format!(" {blank}{blank}")}),
        );
        assert!(matches!(outcome, ToolOutcome::Failed { .. }), "{blank:?}");
        assert!(f.questions().is_empty(), "{blank:?}");
    }
    f.ask("turn", "Quelle option ?");
    let outcome = f.call("turn", "attendre_reponse", json!({"question": "\n\u{a0}"}));
    assert!(matches!(outcome, ToolOutcome::Failed { .. }));
    assert_eq!(f.questions()[0]["question"], "Quelle option ?");
}

#[test]
fn a_question_longer_than_the_bound_is_refused_and_creates_or_replaces_nothing() {
    let mut f = Fixture::new();
    f.turn("turn", 1000);
    let refused = f.call(
        "turn",
        "attendre_reponse",
        json!({"question": "a".repeat(501)}),
    );
    assert!(matches!(refused, ToolOutcome::Failed { .. }));
    assert!(f.questions().is_empty());
    f.ask("turn", &"é".repeat(500));
    let refused = f.call(
        "turn",
        "attendre_reponse",
        json!({"question": "é".repeat(501)}),
    );
    assert!(matches!(refused, ToolOutcome::Failed { .. }));
    assert_eq!(
        f.questions()[0]["question"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        140
    );
}

#[test]
fn a_refused_call_neither_creates_nor_replaces_a_question_whatever_the_cause() {
    // 501 characters, but a nul in the second one: `length()` of SQLite stops there and counts 1.
    let nul_then_too_long = format!("A\0{}", "é".repeat(499));
    assert_eq!(nul_then_too_long.chars().count(), 501);
    let causes = [
        (
            "unicode blank",
            "parent",
            json!({"question": "\u{3000}\u{a0}\u{2028}"}),
        ),
        (
            "nul then too long",
            "parent",
            json!({"question": nul_then_too_long}),
        ),
        ("too long", "parent", json!({"question": "a".repeat(501)})),
        ("empty", "parent", json!({"question": ""})),
        ("not a text", "parent", json!({"question": 7})),
        ("missing", "parent", json!({})),
        ("delegated run", "edge", json!({"question": "Puis-je ?"})),
    ];
    for replacing in [false, true] {
        for (cause, run, arguments) in &causes {
            let mut f = Fixture::new();
            f.turn("parent", 1000);
            f.delegate("edge", "parent", "grokbot", &f.conversation, 1);
            if replacing {
                f.ask("parent", "Quelle option ?");
            }
            let outcome = f.call(run, "attendre_reponse", arguments.clone());
            assert!(matches!(outcome, ToolOutcome::Failed { .. }), "{cause}");
            let questions = f.questions();
            if replacing {
                assert_eq!(questions.len(), 1, "{cause}");
                assert_eq!(questions[0]["question"], "Quelle option ?", "{cause}");
            } else {
                assert!(questions.is_empty(), "{cause}");
            }
        }
    }
}

/// VM instructions of one `GET /api/taches` with an open question after `old` older entries of its thread.
fn thread_cost(old: usize) -> i32 {
    let f = Fixture::new();
    f.turn("turn", 1000);
    with_tx(&f.conn, |c| {
        for i in 0..old {
            let kind = if i % 2 == 0 { "submission" } else { "engine_tool" };
            c.execute("INSERT INTO wrapper_messages(account_id,conversation_id,message_id,kind,created_at) VALUES ('local-workspace',?1,?2,?3,1)", params![f.conversation, format!("old-{i}"), kind])?;
        }
        Ok::<_, grokbot_engine::contracts::Error>(())
    })
    .unwrap();
    f.ask("turn", "Quelle option ?");
    let mut statement = f.conn.prepare(tasks::SNAPSHOT).unwrap();
    let rows = statement
        .query_map(params!["local-workspace", 2_000_000_000i64], |_| Ok(()))
        .unwrap()
        .count();
    assert_eq!(rows, 2);
    statement.get_status(StatementStatus::VmStep)
}

#[test]
fn an_open_question_costs_the_same_whatever_the_age_of_its_thread() {
    let (small, large) = (thread_cost(1000), thread_cost(10_000));
    assert!(
        large <= 3 * small,
        "1000 older entries: {small} VM instructions, 10000: {large}"
    );
}
