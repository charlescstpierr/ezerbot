//! Engine gateway against the shared, scripted Responses server (no live account).
mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use grokbot_engine::contracts::{
    Error, HistoryItem, ImageInput, ModelEvent, ModelGateway, ModelRequest, Role, ToolCall,
    ToolItem, ToolSpec,
};
use grokbot_wrapper::{gateway::SimulatedGateway, Provider};
use serde_json::{json, Value};
use support::{FakeOpenAi, ScriptedResponse, Seen, CLIENT};

struct Fixture {
    provider: Provider,
    server: FakeOpenAi,
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "grokbot-gateway-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let server = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        // Seed a disposable, already-authorized account; OAuth itself is covered in chatgpt.rs.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            directory.join("app.chatgpt-auth.json"),
            json!({
                "host_id": "test-host", "client_id": CLIENT,
                "access_token": "access-2", "refresh_token": "refresh-2",
                "expires_at": now + 3600, "scopes": "chatgpt.tokens.use.direct",
            })
            .to_string(),
        )
        .unwrap();
        let provider =
            Provider::with_endpoints(&directory.join("app.sqlite"), server.endpoints(), false);
        Self {
            provider,
            server,
            directory,
        }
    }

    fn events(&self, events: &[Value]) {
        self.server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(events));
    }

    fn collect(&self, request: &ModelRequest) -> Vec<ModelEvent> {
        let mut events = Vec::new();
        let gateway: &dyn ModelGateway = &self.provider;
        gateway
            .stream(request, &mut |event| {
                events.push(event);
                Ok(())
            })
            .unwrap();
        events
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn history(role: Role, text: &str) -> HistoryItem {
    HistoryItem {
        entry_id: "entry-1".into(),
        role,
        text: text.into(),
        images: vec![],
        tool_items: vec![],
    }
}

fn request() -> ModelRequest {
    ModelRequest {
        model: "gpt-test".into(),
        instructions: "App contract".into(),
        input: vec![history(Role::User, "Read the review skill")],
        tools: vec![ToolSpec {
            name: "read_skill".into(),
            schema: json!({"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}),
            requires_approval: true,
        }],
        effort: Some("high".into()),
        service_tier: Some("priority".into()),
    }
}

fn item(kind: &str, id: &str, call_id: &str, name: &str, arguments: &str) -> Value {
    json!({"type": kind, "item": {
        "id": id, "type": "function_call", "call_id": call_id, "name": name, "arguments": arguments,
    }})
}

fn tool_call(call_id: &str, name: &str, arguments: Value) -> ModelEvent {
    ModelEvent::ToolCall {
        call_id: call_id.into(),
        name: name.into(),
        arguments,
    }
}

#[test]
fn tool_call_then_output_round_trip() {
    let fixture = Fixture::new();
    fixture.events(&[
        item("response.output_item.added", "fc_1", "call_1", "read_skill", ""),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"name\":"}),
        json!({"type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": "{\"name\":\"review\"}"}),
        item("response.output_item.done", "fc_1", "call_1", "read_skill", "{\"name\":\"review\"}"),
        json!({"type": "response.completed"}),
    ]);
    let mut request = request();
    let events = fixture.collect(&request);
    assert_eq!(
        events,
        [
            tool_call("call_1", "read_skill", json!({"name": "review"})),
            ModelEvent::Completed
        ]
    );
    let ModelEvent::ToolCall {
        call_id,
        name,
        arguments,
    } = events[0].clone()
    else {
        panic!("call")
    };
    let mut assistant = history(Role::Assistant, "Reading it now");
    assistant.tool_items.push(ToolItem::Call {
        call: ToolCall {
            call_id: call_id.clone(),
            name,
            arguments,
        },
    });
    request.input.push(assistant);
    let mut output = history(Role::Tool, "");
    output.tool_items.push(ToolItem::Output {
        call_id,
        output: json!({"body": "Review checklist"}),
    });
    request.input.push(output);
    fixture.events(&[
        json!({"type": "response.output_text.delta", "delta": "Checklist received"}),
        json!({"type": "response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request),
        [
            ModelEvent::TextDelta {
                text: "Checklist received".into()
            },
            ModelEvent::Completed
        ]
    );
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(seen.bodies.len(), 2);
    let first = &seen.bodies[0];
    let second = &seen.bodies[1];
    assert_eq!(second["input"][0], first["input"][0]);
    assert_eq!(
        second["input"][1]["content"][0],
        json!({"type": "input_text", "text": "Reading it now"})
    );
    assert_eq!(
        second["input"][2],
        json!({
            "type": "function_call", "call_id": "call_1", "name": "read_skill", "arguments": "{\"name\":\"review\"}",
        })
    );
    assert_eq!(
        second["input"][3],
        json!({
            "type": "function_call_output", "call_id": "call_1", "output": "{\"body\":\"Review checklist\"}",
        })
    );
    assert_eq!(first["tools"][0]["parameters"], request.tools[0].schema);
    assert_eq!(first["tools"][0]["name"], "read_skill");
    assert!(first["tools"][0].get("requires_approval").is_none());
    assert_eq!(first["reasoning"]["effort"], "high");
    assert_eq!(first["service_tier"], "priority");
    assert!(seen.bodies.iter().all(|body| body["store"] == false
        && body["stream"] == true
        && body.get("previous_response_id").is_none()));
    assert!(seen.bearer.iter().all(|token| token == "Bearer access-2"));
}

#[test]
fn parallel_tool_calls_keep_call_ids() {
    let fixture = Fixture::new();
    fixture.events(&[
        item("response.output_item.added", "fc_a", "call_a", "first", ""),
        item("response.output_item.added", "fc_b", "call_b", "second", ""),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_a", "delta": "{\"n\":"}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_b", "delta": "{\"n\":2}"}),
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_a", "delta": "1}"}),
        json!({"type": "response.function_call_arguments.done", "item_id": "fc_b", "arguments": "{\"n\":2}"}),
        item("response.output_item.done", "fc_a", "call_a", "first", ""),
        item("response.output_item.done", "fc_b", "call_b", "second", "{\"n\":2}"),
        json!({"type": "response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            tool_call("call_b", "second", json!({"n": 2})),
            tool_call("call_a", "first", json!({"n": 1})),
            ModelEvent::Completed,
        ]
    );
}

#[test]
fn image_input_is_serialized() {
    let fixture = Fixture::new();
    fixture.events(&[json!({"type": "response.completed"})]);
    let mut request = request();
    request.tools.clear();
    request.effort = None;
    request.service_tier = None;
    request.input[0].images.push(ImageInput {
        attachment_id: "old-image".into(),
        url: None,
        caption: Some("Old caption only".into()),
    });
    let mut image_only = history(Role::User, "");
    image_only.images.push(ImageInput {
        attachment_id: "new-image".into(),
        url: Some("data:image/png;base64,AA==".into()),
        caption: Some("A diagram".into()),
    });
    request.input.push(image_only);
    fixture.collect(&request);
    let seen = fixture.server.seen.lock().unwrap();
    let body = &seen.bodies[0];
    assert_eq!(
        body["input"][0]["content"][1],
        json!({"type": "input_text", "text": "Old caption only"})
    );
    assert_eq!(
        body["input"][1]["content"],
        json!([
            {"type": "input_text", "text": "A diagram"},
            {"type": "input_image", "image_url": "data:image/png;base64,AA=="},
        ])
    );
    for field in ["tools", "reasoning", "service_tier", "previous_response_id"] {
        assert!(body.get(field).is_none());
    }
    assert!(!body.to_string().contains("attachment_id"));
}

#[test]
fn stream_failure_maps_to_failed_event() {
    let fixture = Fixture::new();
    for terminal in [
        json!({"type": "response.failed", "response": {"error": {"message": "Model unavailable"}}}),
        json!({"type": "error", "message": "Model unavailable"}),
    ] {
        fixture.events(&[
            json!({"type": "response.output_text.delta", "delta": "Partial"}),
            terminal,
            json!({"type": "response.completed"}),
        ]);
        assert_eq!(
            fixture.collect(&request()),
            [
                ModelEvent::TextDelta {
                    text: "Partial".into()
                },
                ModelEvent::Failed {
                    reason: "Model unavailable".into()
                },
            ]
        );
    }
}

#[test]
fn incomplete_is_not_completed_and_eof_is_not_success() {
    let fixture = Fixture::new();
    fixture.events(&[json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}})]);
    assert_eq!(
        fixture.collect(&request()),
        [ModelEvent::Incomplete {
            reason: "max_output_tokens".into()
        }]
    );
    fixture.events(&[json!({"type": "response.output_text.delta", "delta": "Partial"})]);
    let events = fixture.collect(&request());
    assert!(matches!(
        events.as_slice(),
        [ModelEvent::TextDelta { .. }, ModelEvent::Failed { .. }]
    ));
}

#[test]
fn done_item_alone_and_added_arguments_done_both_complete_one_call() {
    let fixture = Fixture::new();
    for mut events in [
        vec![item(
            "response.output_item.done",
            "fc_1",
            "call_1",
            "read_skill",
            "{}",
        )],
        vec![
            item(
                "response.output_item.added",
                "fc_1",
                "call_1",
                "read_skill",
                "",
            ),
            json!({"type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": "{}"}),
        ],
    ] {
        events.push(json!({"type": "response.completed"}));
        fixture.events(&events);
        assert_eq!(
            fixture.collect(&request()),
            [
                tool_call("call_1", "read_skill", json!({})),
                ModelEvent::Completed
            ]
        );
    }
}

#[test]
fn malformed_and_unfinished_calls_fail_without_becoming_executable() {
    let fixture = Fixture::new();
    for event in [
        item(
            "response.output_item.done",
            "fc_1",
            "call_1",
            "read_skill",
            "not JSON",
        ),
        item(
            "response.output_item.done",
            "fc_1",
            "call_1",
            "read_skill",
            "[]",
        ),
        item(
            "response.output_item.added",
            "fc_1",
            "call_1",
            "read_skill",
            "{}",
        ),
        item("response.output_item.done", "fc_1", "", "read_skill", "{}"),
    ] {
        fixture.events(&[event, json!({"type": "response.completed"})]);
        assert!(matches!(
            fixture.collect(&request()).as_slice(),
            [ModelEvent::Failed { .. }]
        ));
    }
}

#[test]
fn duplicate_call_ids_across_different_items_are_refused() {
    let fixture = Fixture::new();
    fixture.events(&[
        item("response.output_item.done", "fc_a", "call_1", "first", "{}"),
        item(
            "response.output_item.done",
            "fc_b",
            "call_1",
            "second",
            "{}",
        ),
        json!({"type": "response.completed"}),
    ]);
    assert!(matches!(fixture.collect(&request()).as_slice(), [
        ModelEvent::ToolCall { name, .. }, ModelEvent::Failed { .. }
    ] if name == "first"));
}

#[test]
fn malformed_sse_and_done_sentinel_do_not_fake_completion() {
    let fixture = Fixture::new();
    for body in ["data: not JSON\n\n", "data: [DONE]\n\n", ""] {
        fixture
            .server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse {
                status: 200,
                content_type: "text/event-stream".into(),
                body: body.into(),
            });
        assert!(matches!(
            fixture.collect(&request()).as_slice(),
            [ModelEvent::Failed { .. }]
        ));
    }
}

#[test]
fn sse_multiline_crlf_comments_and_final_event_without_blank_line() {
    let fixture = Fixture::new();
    fixture.server.seen.lock().unwrap().responses.push_back(ScriptedResponse {
        status: 200, content_type: "text/event-stream".into(),
        body: ": keepalive\r\nevent: response.output_text.delta\r\ndata: {\"type\": \"response.output_text.delta\",\r\ndata: \"delta\": \"Bonjour\"}\r\n\r\ndata: {\"type\":\"response.completed\"}".into(),
    });
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "Bonjour".into()
            },
            ModelEvent::Completed
        ]
    );
}

#[test]
fn callback_error_stops_emission_without_fabricating_failure() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type": "response.output_text.delta", "delta": "First"}),
        json!({"type": "response.output_text.delta", "delta": "Second"}),
        json!({"type": "response.completed"}),
    ]);
    let gateway: &dyn ModelGateway = &fixture.provider;
    let mut seen = Vec::new();
    let error = gateway
        .stream(&request(), &mut |event| {
            seen.push(event);
            Err(Error::InvalidInput("cancelled".into()))
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "cancelled");
    assert_eq!(
        seen,
        [ModelEvent::TextDelta {
            text: "First".into()
        }]
    );
}

#[test]
fn unauthorized_refreshes_once_and_http_errors_become_failed_events() {
    let fixture = Fixture::new();
    fixture
        .server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            401,
            json!({"error": {"message": "expired"}}),
        ));
    fixture.events(&[json!({"type": "response.completed"})]);
    assert_eq!(fixture.collect(&request()), [ModelEvent::Completed]);
    {
        let seen = fixture.server.seen.lock().unwrap();
        assert_eq!(seen.bodies.len(), 2);
        assert_eq!(seen.bodies[0], seen.bodies[1]);
        assert_eq!(seen.forms.len(), 1);
        assert_eq!(seen.forms[0]["grant_type"], "refresh_token");
    }
    fixture
        .server
        .seen
        .lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            429,
            json!({"error": {"code": "subscription_sharing_usage_limit_exceeded"}}),
        ));
    assert!(matches!(
        fixture.collect(&request()).as_slice(),
        [ModelEvent::Failed { .. }]
    ));
}

#[test]
fn repeated_unauthorized_is_not_retried_forever() {
    let fixture = Fixture::new();
    for _ in 0..2 {
        fixture
            .server
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::json(
                401,
                json!({"error": {"message": "still expired"}}),
            ));
    }
    assert!(matches!(
        fixture.collect(&request()).as_slice(),
        [ModelEvent::Failed { .. }]
    ));
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(seen.bodies.len(), 2);
    assert_eq!(seen.forms.len(), 1);
}

#[test]
fn simulated_gateway_keeps_existing_rich_reply_and_stops_on_callback_error() {
    let mut request = request();
    request.input[0].text = "Un diagramme".into();
    let mut events = Vec::new();
    SimulatedGateway
        .stream(&request, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
    assert!(matches!(&events[0], ModelEvent::TextDelta { text } if text.contains("```mermaid")));
    assert_eq!(events[1], ModelEvent::Completed);
    let mut count = 0;
    assert!(SimulatedGateway
        .stream(&request, &mut |_| {
            count += 1;
            Err(Error::InvalidInput("cancelled".into()))
        })
        .is_err());
    assert_eq!(count, 1);
}
