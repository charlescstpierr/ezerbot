use grokbot_engine::contracts::{
    with_tx, ApprovalCheck, AuthorizedBy, Error, ModelEvent, ModelGateway, ModelRequest, Result,
    RunBudgets, RunEvent, RunEventKind, RunKind, SuspendCause, ToolCall, ToolOutcome, ToolSpec,
    Trigger, TurnEnd, WakeCause,
};
use grokbot_engine::model::{self, Run, RunStatus};
use grokbot_engine::run_loop::{self, RunProgress, RunServices};
use grokbot_engine::store;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

const BLOCKED: &str = "Appel identique répété 3 fois : le résultat ne changera pas. Change d'approche ou termine la tâche en disant ce qui bloque.";
const STOPPED: &str = "Le bot tournait en rond (3 appels répétés bloqués).";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Internal,
    Admitted,
    Journaled,
}

const ROUTES: [Route; 3] = [Route::Internal, Route::Admitted, Route::Journaled];

struct ScriptedGateway {
    rounds: Mutex<VecDeque<Vec<ModelEvent>>>,
    requests: Mutex<Vec<ModelRequest>>,
}

impl ScriptedGateway {
    fn new(mut rounds: Vec<Vec<ModelEvent>>) -> Self {
        rounds.push(vec![ModelEvent::TextDelta {
            text: "Terminé.".into(),
        }]);
        Self {
            rounds: Mutex::new(rounds.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl ModelGateway for ScriptedGateway {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()> {
        self.requests.lock().unwrap().push(request.clone());
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected additional model request");
        for event in events {
            emit(event)?;
        }
        emit(ModelEvent::Completed)
    }
}

struct EffectServices {
    route: Route,
    outcome: ToolOutcome,
    external_effects: Vec<String>,
    protected: bool,
    gated_approval: Option<ApprovalCheck>,
    detached: bool,
    variable_output: bool,
    outcomes: VecDeque<ToolOutcome>,
    fail_third_trip: bool,
    handover: bool,
    interrupt_peer: Option<Connection>,
    interrupted_result_observed: bool,
}

impl EffectServices {
    fn new(route: Route, outcome: ToolOutcome) -> Self {
        Self {
            route,
            outcome,
            external_effects: Vec::new(),
            protected: route == Route::Admitted,
            gated_approval: None,
            detached: false,
            variable_output: false,
            outcomes: VecDeque::new(),
            fail_third_trip: false,
            handover: false,
            interrupt_peer: None,
            interrupted_result_observed: false,
        }
    }

    fn next_outcome(&mut self, call: &ToolCall) -> ToolOutcome {
        self.outcomes.pop_front().unwrap_or_else(|| {
            if self.variable_output {
                ToolOutcome::Done {
                    output: json!({"call":call.call_id}),
                }
            } else {
                self.outcome.clone()
            }
        })
    }

    fn effect_count(&self, conn: &Connection) -> Result<usize> {
        let internal: usize =
            conn.query_row("SELECT count(*) FROM fixture_effects", [], |row| row.get(0))?;
        Ok(internal + self.external_effects.len())
    }
}

impl RunServices for EffectServices {
    fn context(&mut self, _: &Run) -> Result<ModelRequest> {
        Ok(ModelRequest {
            model: "fixture".into(),
            instructions: String::new(),
            input: Vec::new(),
            tools: Vec::new(),
            effort: None,
            service_tier: None,
            prompt_cache_key: None,
        })
    }

    fn tools(&self) -> Vec<ToolSpec> {
        [
            "read_file",
            "read_other",
            "attendre_reponse",
            "request_form",
        ]
        .into_iter()
        .map(|name| ToolSpec {
            name: name.into(),
            description: "Fixture".into(),
            schema: json!({"type":"object"}),
            requires_approval: self.protected,
        })
        .collect()
    }

    fn is_wait_tool(&self, call: &ToolCall) -> bool {
        matches!(
            call.name.as_str(),
            "attendre_reponse" | "delegate" | "request_form"
        )
    }

    fn call_tool(&mut self, conn: &Connection, _: &Run, call: &ToolCall) -> Result<ToolOutcome> {
        assert_eq!(self.route, Route::Internal);
        assert!(
            !conn.is_autocommit(),
            "internal effects belong to the event transaction"
        );
        conn.execute(
            "INSERT INTO fixture_effects(call_id) VALUES (?1)",
            [&call.call_id],
        )?;
        Ok(self.next_outcome(call))
    }

    fn is_external_tool(&self, _: &ToolCall) -> bool {
        self.route == Route::Admitted
    }

    fn call_external_tool(
        &mut self,
        conn: &Connection,
        run: &Run,
        call: &ToolCall,
    ) -> Result<ToolOutcome> {
        assert_eq!(self.route, Route::Admitted);
        assert!(
            conn.is_autocommit(),
            "external effects must run after admission commits"
        );
        assert_eq!(
            run_loop::external_authorization(conn, &run.run_id, call)?,
            Some(AuthorizedBy::Rule {
                rule_id: "fixture-standing-rule".into()
            })
        );
        self.external_effects.push(call.call_id.clone());
        Ok(self.next_outcome(call))
    }

    fn is_journaled_external_tool(&self, _: &ToolCall) -> bool {
        self.route == Route::Journaled
    }

    fn call_journaled_external_tool(&mut self, _: &Run, call: &ToolCall) -> ToolOutcome {
        assert_eq!(self.route, Route::Journaled);
        self.external_effects.push(call.call_id.clone());
        if let Some(peer) = self.interrupt_peer.take() {
            with_tx(&peer, |conn| store::interrupt_running(conn).map(|_| ())).unwrap();
            self.handover = true;
        }
        self.next_outcome(call)
    }

    fn check_approval(
        &mut self,
        conn: &Connection,
        _: &Run,
        call: &ToolCall,
    ) -> Result<ApprovalCheck> {
        assert!(!conn.is_autocommit());
        if call.call_id == "gate" {
            if let Some(decision) = self.gated_approval.clone() {
                if matches!(decision, ApprovalCheck::Authorized { .. }) {
                    conn.execute("UPDATE fixture_authority SET remaining=remaining-1", [])?;
                    conn.execute("INSERT INTO fixture_audit VALUES ('consumed')", [])?;
                }
                return Ok(decision);
            }
        }
        Ok(ApprovalCheck::Authorized {
            by: AuthorizedBy::Rule {
                rule_id: "fixture-standing-rule".into(),
            },
        })
    }

    fn request_approval(
        &mut self,
        conn: &Connection,
        _: &Run,
        _: &ToolCall,
    ) -> Result<SuspendCause> {
        conn.execute("INSERT INTO fixture_requests VALUES ('request')", [])?;
        Ok(SuspendCause::HumanInput {
            id: "approval-fixture".into(),
        })
    }

    fn continue_without_approval(&mut self, _: &Connection, _: &Run) -> Result<bool> {
        Ok(self.detached)
    }

    fn is_interrupted(&self) -> bool {
        self.handover
    }

    fn is_cancelled(&self, _: &str) -> bool {
        false
    }

    fn on_run_event(&mut self, conn: &Connection, event: &RunEvent) -> Result<()> {
        assert!(!conn.is_autocommit());
        if self.fail_third_trip
            && event.kind == RunEventKind::LoopGuard
            && store::loop_guard_count(conn, "run")? == 3
        {
            return Err(Error::Conflict("fixture rollback of third trip".into()));
        }
        if event.kind == RunEventKind::ToolResult && self.handover {
            assert_eq!(store::required(conn, "run")?.status, RunStatus::Interrupted);
            let stored: String = conn.query_row(
                "SELECT outcome FROM engine_external_tools WHERE run_id='run' AND call_id=?1",
                [event.payload["call_id"].as_str().unwrap()],
                |row| row.get(0),
            )?;
            assert_eq!(
                serde_json::from_str::<Value>(&stored)?,
                event.payload["outcome"]
            );
            assert!(event.payload["effective_attempt"].is_object());
            self.interrupted_result_observed = true;
        }
        Ok(())
    }
}

fn database() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    store::migrate(&conn)?;
    initialize_database(&conn)?;
    Ok(conn)
}

fn initialize_database(conn: &Connection) -> Result<()> {
    store::migrate(conn)?;
    conn.execute_batch(
        "CREATE TABLE fixture_effects(call_id TEXT PRIMARY KEY);
        CREATE TABLE fixture_authority(remaining INTEGER); INSERT INTO fixture_authority VALUES (1);
        CREATE TABLE fixture_audit(action TEXT); CREATE TABLE fixture_requests(id TEXT)",
    )?;
    with_tx(&conn, |conn| {
        model::start(
            conn,
            &Run {
                run_id: "run".into(),
                bot_id: "bot".into(),
                conversation_id: "conversation".into(),
                trigger: Trigger::UserMessage {
                    message_id: "message".into(),
                    attempt: 0,
                },
                kind: RunKind::Chat,
                status: RunStatus::Queued,
                task_status: None,
                turn_end: None,
                budgets: RunBudgets {
                    model_calls: 32,
                    tool_calls: 64,
                    duration_ms: 600_000,
                },
                started_at: 0,
            },
        )
    })?;
    Ok(())
}

fn call(id: &str, name: &str, arguments: Value) -> ModelEvent {
    ModelEvent::ToolCall {
        call_id: id.into(),
        name: name.into(),
        arguments,
    }
}

fn done() -> ToolOutcome {
    ToolOutcome::Done {
        output: json!({"text":"fixture-result-private"}),
    }
}

fn guards(events: &[RunEvent]) -> Vec<&RunEvent> {
    events
        .iter()
        .filter(|event| serde_json::to_value(event.kind).unwrap() == json!("loop_guard"))
        .collect()
}

fn assert_blocked(events: &[RunEvent], call_id: &str) {
    let result = events
        .iter()
        .find(|event| event.kind == RunEventKind::ToolResult && event.payload["call_id"] == call_id)
        .expect("a blocked proposal must receive its final tool result");
    assert_eq!(
        result.payload["outcome"],
        json!({"status":"failed","reason":BLOCKED})
    );
}

fn assert_settled(events: &[RunEvent]) {
    let calls: HashSet<_> = events
        .iter()
        .filter(|event| event.kind == RunEventKind::ToolCall)
        .map(|event| event.payload["call_id"].as_str().unwrap())
        .collect();
    let results: Vec<_> = events
        .iter()
        .filter(|event| event.kind == RunEventKind::ToolResult)
        .map(|event| event.payload["call_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        calls,
        results.iter().copied().collect::<HashSet<_>>(),
        "every proposal must have a result"
    );
    assert_eq!(
        calls.len(),
        results.len(),
        "a proposal has exactly one final result"
    );
}

#[test]
fn loop_guard_blocks_third_identical_call() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let gateway = ScriptedGateway::new(
            (1..=3)
                .map(|n| {
                    vec![call(
                        &format!("call-{n}"),
                        "read_file",
                        json!({"path":"fixture-argument-private"}),
                    )]
                })
                .collect(),
        );
        let mut services = EffectServices::new(route, done());
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 2, "route {route:?}");
        let events = store::events(&conn, "run", 0)?;
        assert_blocked(&events, "call-3");
        assert_eq!(guards(&events).len(), 1, "route {route:?}");
        let payload = serde_json::to_string(&guards(&events)[0].payload)?;
        assert!(payload.contains("read_file"));
        assert!(!payload.contains("fixture-argument-private"));
        assert!(!payload.contains("fixture-result-private"));
        assert!(payload.len() < 512, "guard metadata must stay bounded");
        let metadata = guards(&events)[0].payload.as_object().unwrap();
        assert_eq!(metadata.len(), 3);
        let fingerprint = metadata["fingerprint"].as_str().unwrap();
        assert_eq!(fingerprint.len(), 12);
        assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(metadata["name"], "read_file");
        assert!(metadata["rule"].is_string());
        assert_settled(&events);
        assert_eq!(gateway.request_count(), 4);
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 2);
        assert_eq!(guards(&store::events(&conn, "run", 0)?).len(), 1);
        assert_eq!(
            gateway.request_count(),
            4,
            "replaying an ended run must not call the model"
        );
    }
    Ok(())
}

#[test]
fn loop_guard_fingerprint_ignores_key_order() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let arguments: Vec<Value> = [
            r#"{"a":1,"b":2,"nested":{"left":1,"right":[{"x":1,"y":2}]}}"#,
            r#"{"nested":{"right":[{"y":2,"x":1}],"left":1},"b":2,"a":1}"#,
            r#"{"b":2,"nested":{"left":1,"right":[{"y":2,"x":1}]},"a":1}"#,
        ]
        .into_iter()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
        let gateway = ScriptedGateway::new(
            arguments
                .into_iter()
                .enumerate()
                .map(|(n, args)| vec![call(&format!("ordered-{n}"), "read_file", args)])
                .collect(),
        );
        let mut services = EffectServices::new(route, done());
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 2, "route {route:?}");
        let events = store::events(&conn, "run", 0)?;
        assert_blocked(&events, "ordered-2");
        assert_eq!(guards(&events).len(), 1);
        assert_settled(&events);
    }
    Ok(())
}

#[test]
fn loop_guard_blocks_second_identical_failure() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let gateway = ScriptedGateway::new(vec![
            vec![call("failure-1", "read_file", json!({"path":"missing"}))],
            vec![call("failure-2", "read_file", json!({"path":"missing"}))],
        ]);
        let mut services = EffectServices::new(
            route,
            ToolOutcome::Failed {
                reason: "fixture failure".into(),
            },
        );
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 1, "route {route:?}");
        let events = store::events(&conn, "run", 0)?;
        assert_blocked(&events, "failure-2");
        assert_eq!(guards(&events).len(), 1);
        assert_settled(&events);
    }
    Ok(())
}

#[test]
fn loop_guard_stops_turn_after_three_trips() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let mut batch: Vec<_> = (1..=5)
            .map(|n| call(&format!("repeat-{n}"), "read_file", json!({"path":"a"})))
            .collect();
        batch.push(call(
            "remaining",
            "read_other",
            json!({"path":"must-not-run"}),
        ));
        let gateway = ScriptedGateway::new(vec![batch]);
        let mut services = EffectServices::new(route, done());
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Stopped)
        );
        assert_eq!(services.effect_count(&conn)?, 2, "route {route:?}");
        assert_eq!(
            gateway.request_count(),
            1,
            "the third trip must stop without another model request"
        );
        let events = store::events(&conn, "run", 0)?;
        assert_eq!(guards(&events).len(), 3);
        for id in ["repeat-3", "repeat-4", "repeat-5"] {
            assert_blocked(&events, id);
        }
        assert_settled(&events);
        let visible = run_loop::replay_items(&events)?
            .into_iter()
            .map(|item| item.text)
            .collect::<String>();
        assert_eq!(visible, STOPPED);
        let saved = store::get(&conn, "run")?.unwrap();
        assert!(saved.status.is_terminal());
        assert_eq!(saved.turn_end, Some(TurnEnd::Stopped));
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Stopped)
        );
        assert_eq!(gateway.request_count(), 1);
        assert_eq!(services.effect_count(&conn)?, 2);
        assert_eq!(guards(&store::events(&conn, "run", 0)?).len(), 3);
    }
    Ok(())
}

#[test]
fn loop_guard_allows_alternating_calls() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let gateway = ScriptedGateway::new(
            (0..6)
                .map(|n| {
                    vec![call(
                        &format!("alternate-{n}"),
                        "read_file",
                        json!({"path":if n % 2 == 0 { "a" } else { "b" }}),
                    )]
                })
                .collect(),
        );
        let mut services = EffectServices::new(route, done());
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 6, "route {route:?}");
        let events = store::events(&conn, "run", 0)?;
        assert!(guards(&events).is_empty());
        assert_settled(&events);
    }
    Ok(())
}

#[test]
fn loop_guard_preserves_json_types_array_order_and_tool_names() -> Result<()> {
    for route in ROUTES {
        for (first, different, name) in [
            (json!({"value":null}), json!({}), "read_file"),
            (json!({"value":1}), json!({"value":"1"}), "read_file"),
            (json!({"value":false}), json!({"value":0}), "read_file"),
            (json!({"value":[1,2]}), json!({"value":[2,1]}), "read_file"),
            (json!({"value":1}), json!({"value":1}), "read_other"),
        ] {
            let conn = database()?;
            let gateway = ScriptedGateway::new(vec![
                vec![call("first", "read_file", first.clone())],
                vec![call("second", "read_file", first)],
                vec![call("different", name, different)],
            ]);
            let mut services = EffectServices::new(route, done());
            assert_eq!(
                run_loop::run(&conn, &gateway, &mut services, "run")?,
                RunProgress::Ended(TurnEnd::Finished)
            );
            assert_eq!(
                services.effect_count(&conn)?,
                3,
                "route {route:?}, distinct name {name}"
            );
            let events = store::events(&conn, "run", 0)?;
            assert!(guards(&events).is_empty());
            assert_settled(&events);
        }
    }
    Ok(())
}

#[test]
fn loop_guard_exempts_attendre_reponse_even_when_it_returns_done() -> Result<()> {
    let conn = database()?;
    let gateway = ScriptedGateway::new(
        (0..5)
            .map(|n| {
                vec![call(
                    &format!("wait-{n}"),
                    "attendre_reponse",
                    json!({"message":"Please respond"}),
                )]
            })
            .collect(),
    );
    let mut services = EffectServices::new(
        Route::Internal,
        ToolOutcome::Done {
            output: json!({"signaled":true}),
        },
    );
    assert_eq!(
        run_loop::run(&conn, &gateway, &mut services, "run")?,
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(services.effect_count(&conn)?, 5);
    let events = store::events(&conn, "run", 0)?;
    assert!(guards(&events).is_empty());
    assert_settled(&events);
    Ok(())
}

#[test]
fn invalid_and_unknown_proposals_are_not_effective_failures() -> Result<()> {
    for route in ROUTES {
        let conn = database()?;
        let mut batch = Vec::new();
        for n in 0..3 {
            batch.push(call(&format!("invalid-{n}"), "read_file", json!("invalid")));
            batch.push(call(
                &format!("unknown-{n}"),
                "unknown_fixture_tool",
                json!({}),
            ));
        }
        for n in 0..3 {
            batch.push(call(&format!("valid-{n}"), "read_file", json!({})));
        }
        let gateway = ScriptedGateway::new(vec![batch]);
        let mut services = EffectServices::new(route, done());
        assert_eq!(
            run_loop::run(&conn, &gateway, &mut services, "run")?,
            RunProgress::Ended(TurnEnd::Finished)
        );
        assert_eq!(services.effect_count(&conn)?, 2, "route {route:?}");
        let events = store::events(&conn, "run", 0)?;
        assert_eq!(store::loop_guard_count(&conn, "run")?, 1);
        assert_blocked(&events, "valid-2");
        for result in events
            .iter()
            .filter(|event| event.kind == RunEventKind::ToolResult)
        {
            if result.payload["call_id"]
                .as_str()
                .unwrap()
                .starts_with("valid-")
            {
                continue;
            }
            assert!(result.payload.get("effective_attempt").is_none());
        }
        assert_settled(&events);
    }
    Ok(())
}

#[test]
fn variable_results_and_business_failure_done_use_the_third_call_threshold() -> Result<()> {
    for route in ROUTES {
        for variable in [false, true] {
            let conn = database()?;
            let gateway = ScriptedGateway::new(vec![(0..3)
                .map(|n| call(&format!("business-{n}"), "read_file", json!({})))
                .collect()]);
            let mut services = EffectServices::new(
                route,
                ToolOutcome::Done {
                    output: json!({"status":"échec","exit_code":1}),
                },
            );
            services.variable_output = variable;
            run_loop::run(&conn, &gateway, &mut services, "run")?;
            assert_eq!(services.effect_count(&conn)?, 2);
            let events = store::events(&conn, "run", 0)?;
            assert_blocked(&events, "business-2");
            let attempts: Vec<_> = events
                .iter()
                .filter(|e| e.payload["effective_attempt"].is_object())
                .collect();
            assert_eq!(attempts.len(), 2);
            assert!(attempts
                .iter()
                .all(|e| e.payload["effective_attempt"]["outcome"] == "done"));
            assert_eq!(
                attempts[0].payload["effective_attempt"]["fingerprint"],
                attempts[1].payload["effective_attempt"]["fingerprint"]
            );
            if variable {
                assert_ne!(
                    attempts[0].payload["outcome"],
                    attempts[1].payload["outcome"]
                );
            }
        }
    }
    Ok(())
}

#[test]
fn approval_authority_precedes_guard_and_block_returns_single_use_consumption() -> Result<()> {
    for route in ROUTES {
        for decision in [
            ApprovalCheck::Ask,
            ApprovalCheck::Denied {
                reason: "fixture denied".into(),
            },
            ApprovalCheck::Authorized {
                by: AuthorizedBy::AllowOnce {
                    request_id: "single-use".into(),
                },
            },
        ] {
            for detached in [false, true] {
                let conn = database()?;
                let gateway = ScriptedGateway::new(vec![vec![
                    call("first", "read_file", json!({})),
                    call("second", "read_file", json!({})),
                    call("gate", "read_file", json!({})),
                ]]);
                let mut services = EffectServices::new(route, done());
                services.protected = true;
                services.detached = detached;
                services.gated_approval = Some(decision.clone());
                let progress = run_loop::run(&conn, &gateway, &mut services, "run")?;
                assert_eq!(services.effect_count(&conn)?, 2);
                let events = store::events(&conn, "run", 0)?;
                let count = store::loop_guard_count(&conn, "run")?;
                match decision {
                    ApprovalCheck::Authorized { .. } => {
                        assert_eq!(count, 1);
                        assert_blocked(&events, "gate");
                        assert_eq!(
                            conn.query_row("SELECT remaining FROM fixture_authority", [], |r| r
                                .get::<_, i64>(
                                0
                            ))?,
                            1
                        );
                        assert_eq!(
                            conn.query_row("SELECT count(*) FROM fixture_audit", [], |r| r
                                .get::<_, i64>(0))?,
                            0
                        );
                    }
                    ApprovalCheck::Ask if !detached => {
                        assert!(matches!(progress, RunProgress::Suspended(_)));
                        assert_eq!(count, 0);
                        assert_eq!(
                            conn.query_row("SELECT count(*) FROM fixture_requests", [], |r| r
                                .get::<_, i64>(0))?,
                            1
                        );
                    }
                    _ => {
                        assert_eq!(count, 0);
                        let result = events
                            .iter()
                            .find(|e| {
                                e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "gate"
                            })
                            .unwrap();
                        assert!(result.payload.get("effective_attempt").is_none());
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn done_waits_and_suspension_do_not_replace_effect_tail_across_wake() -> Result<()> {
    let conn = database()?;
    let gateway = ScriptedGateway::new(vec![vec![
        call("first", "read_file", json!({})),
        call("second", "read_file", json!({})),
        call("wait-one", "request_form", json!({})),
        call("wait-two", "request_form", json!({})),
        call("wait-three", "request_form", json!({})),
        call("third", "read_file", json!({})),
    ]]);
    let mut services = EffectServices::new(Route::Internal, done());
    services.outcomes = [
        done(),
        done(),
        done(),
        done(),
        ToolOutcome::Suspend {
            cause: SuspendCause::HumanInput {
                id: "form-fixture".into(),
            },
        },
    ]
    .into();
    assert!(matches!(
        run_loop::run(&conn, &gateway, &mut services, "run")?,
        RunProgress::Suspended(_)
    ));
    assert_eq!(services.effect_count(&conn)?, 5);
    assert_eq!(store::loop_guard_count(&conn, "run")?, 0);
    assert!(matches!(
        run_loop::run(&conn, &gateway, &mut services, "run")?,
        RunProgress::Suspended(_)
    ));
    assert_eq!(services.effect_count(&conn)?, 5);
    with_tx(&conn, |conn| {
        store::enqueue_wake(conn, "run", WakeCause::Form, "form-fixture").map(|_| ())
    })?;
    assert_eq!(
        run_loop::run(&conn, &gateway, &mut services, "run")?,
        RunProgress::Ended(TurnEnd::Finished)
    );
    assert_eq!(services.effect_count(&conn)?, 5);
    let events = store::events(&conn, "run", 0)?;
    assert_blocked(&events, "third");
    assert_eq!(
        events
            .iter()
            .filter(|e| e.payload["effective_attempt"].is_object())
            .count(),
        2
    );
    assert_settled(&events);
    Ok(())
}

#[test]
fn third_trip_and_terminal_text_roll_back_together_then_resume_once() -> Result<()> {
    let conn = database()?;
    let gateway = ScriptedGateway::new(vec![(0..5)
        .map(|n| call(&format!("rollback-{n}"), "read_file", json!({})))
        .collect()]);
    let mut services = EffectServices::new(Route::Internal, done());
    services.fail_third_trip = true;
    assert!(run_loop::run(&conn, &gateway, &mut services, "run").is_err());
    assert_eq!(store::loop_guard_count(&conn, "run")?, 2);
    assert_eq!(
        store::required(&conn, "run")?.status,
        RunStatus::Interrupted
    );
    assert_eq!(services.effect_count(&conn)?, 2);
    let before = store::events(&conn, "run", 0)?;
    assert!(!before.iter().any(|e| e.kind == RunEventKind::Cancelled));
    assert!(!before.iter().any(|e| e.payload["text"]
        .as_str()
        .is_some_and(|s| s.contains(STOPPED))));
    services.fail_third_trip = false;
    assert_eq!(
        run_loop::run(&conn, &gateway, &mut services, "run")?,
        RunProgress::Ended(TurnEnd::Stopped)
    );
    assert_eq!(store::loop_guard_count(&conn, "run")?, 3);
    let after = store::events(&conn, "run", 0)?;
    assert_settled(&after);
    assert_eq!(
        after
            .iter()
            .filter(|e| e.kind == RunEventKind::Cancelled)
            .count(),
        1
    );
    assert!(!after.iter().any(|e| e.kind == RunEventKind::TurnEnded));
    assert_eq!(
        run_loop::replay_items(&after)?
            .iter()
            .map(|i| i.text.as_str())
            .collect::<String>(),
        STOPPED
    );
    assert_eq!(services.effect_count(&conn)?, 2);
    assert_eq!(gateway.request_count(), 1);
    Ok(())
}

#[test]
fn recovered_external_results_and_uncertain_admissions_never_create_attempt_proof() -> Result<()> {
    for route in [Route::Admitted, Route::Journaled] {
        for known in [false, true] {
            let conn = database()?;
            with_tx(&conn, |conn| -> Result<()> {
                model::set_running_at(
                    conn,
                    "run",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as i64,
                )?;
                let recovered = ToolCall {
                    call_id: "recovered".into(),
                    name: "read_file".into(),
                    arguments: json!({}),
                };
                store::append_event(
                    conn,
                    "run",
                    RunEventKind::ToolCall,
                    &serde_json::to_value(&recovered)?,
                )?;
                if route == Route::Journaled {
                    conn.execute("INSERT INTO engine_external_tools(run_id,call_id,outcome) VALUES ('run','recovered',?1)",
                        [if known {Some(serde_json::to_string(&done())?)} else {None}])?;
                } else {
                    conn.execute("INSERT INTO engine_tool_admissions VALUES ('run','recovered','legacy',NULL)",[])?;
                }
                Ok(())
            })?;
            let gateway = ScriptedGateway::new(vec![vec![
                call("new-one", "read_file", json!({})),
                call("new-two", "read_file", json!({})),
                call("new-three", "read_file", json!({})),
            ]]);
            let mut services = EffectServices::new(route, done());
            run_loop::run(&conn, &gateway, &mut services, "run")?;
            assert_eq!(services.effect_count(&conn)?, 2);
            let events = store::events(&conn, "run", 0)?;
            let restored = events
                .iter()
                .find(|e| e.kind == RunEventKind::ToolResult && e.payload["call_id"] == "recovered")
                .unwrap();
            assert!(restored.payload.get("effective_attempt").is_none());
            assert_blocked(&events, "new-three");
            assert_settled(&events);
        }
    }
    Ok(())
}

#[test]
fn journaled_outcome_and_attempt_publish_atomically_while_interrupted() -> Result<()> {
    let dir = grokbot_test_support::TestDir::new("loop-guard-interrupted");
    let path = dir.to_path_buf().join("workspace.sqlite");
    let conn = Connection::open(&path)?;
    initialize_database(&conn)?;
    let gateway = ScriptedGateway::new(vec![vec![
        call("first", "read_file", json!({})),
        call("second", "read_file", json!({})),
        call("third", "read_file", json!({})),
    ]]);
    let mut services = EffectServices::new(Route::Journaled, done());
    services.interrupt_peer = Some(Connection::open(&path)?);
    assert!(run_loop::run(&conn, &gateway, &mut services, "run").is_err());
    assert!(services.interrupted_result_observed);
    assert_eq!(services.effect_count(&conn)?, 1);
    let events = store::events(&conn, "run", 0)?;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == RunEventKind::ToolResult)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.payload["effective_attempt"].is_object())
            .count(),
        1
    );
    services.handover = false;
    run_loop::run(&conn, &gateway, &mut services, "run")?;
    assert_eq!(services.effect_count(&conn)?, 2);
    let events = store::events(&conn, "run", 0)?;
    assert_blocked(&events, "third");
    assert_settled(&events);
    Ok(())
}
