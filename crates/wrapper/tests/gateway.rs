//! Engine gateway against the shared, scripted Responses server (no live account).
mod support;

use std::collections::HashSet;
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
    _dir: grokbot_test_support::TestDir,
}

impl Fixture {
    fn new() -> Self {
        let guard = grokbot_test_support::TestDir::new("grokbot-gateway");
        let directory = guard.to_path_buf();
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
            _dir: guard,
        }
    }

    fn reply(&self, reply: ScriptedResponse) {
        self.server.seen.lock().unwrap().responses.push_back(reply);
    }

    fn events(&self, events: &[Value]) {
        self.reply(ScriptedResponse::events(events));
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
        // The shared fake server intentionally accepts arbitrary JSON. Validate
        // the received wire format here rather than treating HTTP 200 as proof.
        for body in &self.server.seen.lock().unwrap().bodies {
            validate_input(body);
        }
        events
    }
}

fn validate_input(body: &Value) {
    let mut calls = HashSet::new();
    for item in body["input"].as_array().expect("complete input array") {
        match item["type"].as_str() {
            Some("function_call") => {
                calls.insert(item["call_id"].as_str().expect("call ID"));
                assert!(item["arguments"].is_string());
            }
            Some("function_call_output") => {
                assert!(
                    calls.contains(item["call_id"].as_str().expect("output call ID")),
                    "output has no preceding function call: {item}"
                );
                assert!(item["output"].is_string());
            }
            Some("reasoning") => {
                assert!(!item["id"].as_str().expect("reasoning ID").is_empty());
                assert!(!item["encrypted_content"]
                    .as_str()
                    .expect("opaque reasoning")
                    .is_empty());
                assert_eq!(item["summary"], json!([]));
                assert!(item.get("role").is_none() && item.get("content").is_none());
            }
            _ => {
                let role = item["role"].as_str().expect("message role");
                let content = &item["content"];
                assert!(["system", "user", "assistant"].contains(&role));
                if !content.is_string() {
                    for part in content.as_array().expect("message content") {
                        let kind = part["type"].as_str().expect("content type");
                        if role == "assistant" {
                            assert!(
                                ["output_text", "refusal"].contains(&kind),
                                "assistant input_* content rejected by Responses: {item}"
                            );
                        } else {
                            assert!(
                                kind == "input_text" || (role == "user" && kind == "input_image")
                            );
                        }
                    }
                }
            }
        }
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
            description: "Reads one skill body".into(),
            schema: json!({"type": "object", "description": "Reads one skill body", "properties": {"name": {"type": "string"}}, "required": ["name"]}),
            requires_approval: true,
        }],
        effort: Some("high".into()),
        service_tier: Some("priority".into()),
        prompt_cache_key: Some("test-conversation".into()),
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
fn completed_usage_is_optional_validated_and_precedes_completion() {
    use grokbot_engine::contracts::TokenUsage;
    let fixture = Fixture::new();
    for (usage, expected) in [
        (
            json!({"input_tokens":12340,"output_tokens":100,"input_tokens_details":{"cached_tokens":11008}}),
            Some((12340, 100, 11008)),
        ),
        (
            json!({"input_tokens":12,"output_tokens":3}),
            Some((12, 3, 0)),
        ),
        (
            json!({"input_tokens":0,"output_tokens":0,"input_tokens_details":{"cached_tokens":0}}),
            Some((0, 0, 0)),
        ),
        (Value::Null, None),
        (
            json!({"input_tokens":12,"output_tokens":3,"input_tokens_details":{"cached_tokens":-1}}),
            None,
        ),
        (
            json!({"input_tokens":12,"output_tokens":3,"input_tokens_details":{"cached_tokens":13}}),
            None,
        ),
        (json!({"input_tokens":"12","output_tokens":3}), None),
    ] {
        fixture.events(&[json!({"type":"response.completed","response":{"usage":usage}})]);
        let events = fixture.collect(&request());
        let mut expected_events = vec![];
        if let Some((input_tokens, output_tokens, cached_input_tokens)) = expected {
            expected_events.push(ModelEvent::Usage {
                usage: TokenUsage {
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                },
            });
        }
        expected_events.push(ModelEvent::Completed);
        assert_eq!(events, expected_events);
    }
    assert!(fixture
        .server
        .seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .all(|b| b["prompt_cache_key"] == "test-conversation" && b["store"] == false));
}

#[test]
fn tools_have_canonical_wire_order_independent_of_registry_order() {
    let fixture = Fixture::new();
    let mut request = request();
    let mut other = request.tools[0].clone();
    other.name = "alpha".into();
    request.tools.push(other);
    fixture.events(&[json!({"type":"response.completed"})]);
    fixture.collect(&request);
    request.tools.reverse();
    fixture.events(&[json!({"type":"response.completed"})]);
    fixture.collect(&request);
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(
        serde_json::to_vec(&seen.bodies[0]["tools"]).unwrap(),
        serde_json::to_vec(&seen.bodies[1]["tools"]).unwrap()
    );
    assert_eq!(seen.bodies[0]["tools"][0]["name"], "alpha");
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
    assert_eq!(second["input"][1]["content"], "Reading it now");
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
    assert_eq!(first["tools"][0]["description"], "Reads one skill body");
    assert!(first["tools"][0].get("requires_approval").is_none());
    assert_eq!(first["reasoning"]["effort"], "high");
    assert_eq!(first["service_tier"], "priority");
    assert!(seen.bodies.iter().all(|body| body["store"] == false
        && body["stream"] == true
        && body.get("previous_response_id").is_none()));
    assert!(seen.bearer.iter().all(|token| token == "Bearer access-2"));
}

#[test]
fn dedicated_tool_description_overrides_legacy_schema_and_empty_default_falls_back() {
    let fixture = Fixture::new();
    let mut request = request();
    request.tools[0].description = "Dedicated description".into();
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request);
    request.tools[0].description.clear();
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request);
    request.tools[0]
        .schema
        .as_object_mut()
        .unwrap()
        .remove("description");
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request);
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(
        seen.bodies[0]["tools"][0]["description"],
        "Dedicated description"
    );
    assert_eq!(
        seen.bodies[1]["tools"][0]["description"],
        "Reads one skill body"
    );
    assert!(seen.bodies[2]["tools"][0].get("description").is_none());
}

#[test]
fn reasoning_round_trip_stays_opaque_before_its_tool_call() {
    let fixture = Fixture::new();
    let opaque = "encrypted-provider-state-<not-assistant-text>";
    let done = json!({"type": "response.output_item.done", "item": {
        "type": "reasoning", "id": "rs_1", "encrypted_content": opaque, "summary": [],
    }});
    fixture.events(&[
        json!({"type": "response.output_item.added", "item": {"type": "reasoning", "id": "rs_1", "summary": []}}),
        done.clone(), done,
        item("response.output_item.done", "fc_1", "call_1", "read_skill", "{\"name\":\"review\"}"),
        json!({"type": "response.completed"}),
    ]);
    let mut request = request();
    assert_eq!(
        fixture.collect(&request),
        [
            ModelEvent::Reasoning {
                item_id: "rs_1".into(),
                encrypted_content: opaque.into()
            },
            tool_call("call_1", "read_skill", json!({"name": "review"})),
            ModelEvent::Completed,
        ]
    );
    let mut assistant = history(Role::Assistant, "");
    assistant.tool_items = vec![
        ToolItem::Reasoning {
            item_id: "rs_1".into(),
            encrypted_content: opaque.into(),
        },
        ToolItem::Call {
            call: ToolCall {
                call_id: "call_1".into(),
                name: "read_skill".into(),
                arguments: json!({"name": "review"}),
            },
        },
    ];
    let mut output = history(Role::Tool, "");
    output.tool_items.push(ToolItem::Output {
        call_id: "call_1".into(),
        output: json!("skill body"),
    });
    request.input.extend([assistant, output]);
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request);
    let seen = fixture.server.seen.lock().unwrap();
    for body in &seen.bodies {
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["store"], false);
    }
    let input = &seen.bodies[1]["input"];
    assert_eq!(
        input[1],
        json!({"type": "reasoning", "id": "rs_1", "encrypted_content": opaque, "summary": []})
    );
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "call_1");
    assert_eq!(input[3]["type"], "function_call_output");
    assert!(input.as_array().unwrap().iter().all(|item| item
        .get("content")
        .is_none_or(|content| !content.to_string().contains(opaque))));
}

#[test]
fn reasoning_summary_is_not_text_and_ciphertext_without_id_fails_closed() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type": "response.output_item.done", "item": {"type": "reasoning", "id": "rs_1", "summary": [{"text": "not assistant text"}]}}),
        json!({"type": "response.output_text.delta", "delta": "visible answer"}),
        json!({"type": "response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "visible answer".into()
            },
            ModelEvent::Completed
        ]
    );
    fixture.events(&[
        json!({"type": "response.output_item.done", "item": {"type": "reasoning", "encrypted_content": "opaque"}}),
        json!({"type": "response.completed"}),
    ]);
    assert!(matches!(
        fixture.collect(&request()).as_slice(),
        [ModelEvent::Failed { .. }]
    ));
}

#[test]
fn reasoning_callback_error_stops_stream_without_fabricated_failure() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type": "response.output_item.done", "item": {"type": "reasoning", "id": "rs_1", "encrypted_content": "opaque"}}),
        json!({"type": "response.output_text.delta", "delta": "must not emit"}),
        json!({"type": "response.completed"}),
    ]);
    let mut seen = Vec::new();
    let error = fixture
        .provider
        .stream(&request(), &mut |event| {
            seen.push(event);
            Err(Error::InvalidInput("cancelled".into()))
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "cancelled");
    assert_eq!(
        seen,
        [ModelEvent::Reasoning {
            item_id: "rs_1".into(),
            encrypted_content: "opaque".into()
        }]
    );
}

#[test]
fn invalid_arguments_keep_raw_transport_and_call_ids_for_per_call_failure() {
    let fixture = Fixture::new();
    for raw in ["not JSON", "[]", "", "\"string\"", "null", "1"] {
        fixture.events(&[
            item(
                "response.output_item.done",
                "fc_bad",
                "bad",
                "read_skill",
                raw,
            ),
            item(
                "response.output_item.done",
                "fc_good",
                "good",
                "read_skill",
                "{}",
            ),
            json!({"type": "response.completed"}),
        ]);
        let mut request = request();
        let events = fixture.collect(&request);
        assert_eq!(
            events,
            [
                tool_call("bad", "read_skill", json!(raw)),
                tool_call("good", "read_skill", json!({})),
                ModelEvent::Completed,
            ]
        );
        let mut assistant = history(Role::Assistant, "");
        for event in events {
            if let ModelEvent::ToolCall {
                call_id,
                name,
                arguments,
            } = event
            {
                assistant.tool_items.push(ToolItem::Call {
                    call: ToolCall {
                        call_id,
                        name,
                        arguments,
                    },
                });
            }
        }
        let mut output = history(Role::Tool, "");
        output.tool_items = vec![
            ToolItem::Output {
                call_id: "bad".into(),
                output: json!("invalid arguments"),
            },
            ToolItem::Output {
                call_id: "good".into(),
                output: json!("result"),
            },
        ];
        request.input.extend([assistant, output]);
        fixture.events(&[json!({"type": "response.completed"})]);
        fixture.collect(&request);
        assert_eq!(
            fixture.server.seen.lock().unwrap().bodies.last().unwrap()["input"][1]["arguments"],
            raw
        );
    }
    fixture.events(&[
        item("response.output_item.done", "fc_partial", "partial", "read_skill", "{"),
        json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            tool_call("partial", "read_skill", json!("{")),
            ModelEvent::Incomplete {
                reason: "max_output_tokens".into()
            },
        ]
    );
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
fn a_local_endpoint_asks_for_one_tool_call_at_a_time() {
    let fixture = Fixture::new();
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request());
    let body = &fixture.server.seen.lock().unwrap().bodies[0];
    assert_eq!(body["parallel_tool_calls"], false, "{body}");
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
    for field in [
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning",
        "service_tier",
        "previous_response_id",
    ] {
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
                    reason: "OpenAI a signalé une erreur pendant la réponse : Model unavailable"
                        .into()
                },
            ]
        );
    }
}

#[test]
fn incomplete_reports_usage_before_its_terminal_event() {
    use grokbot_engine::contracts::TokenUsage;
    let fixture = Fixture::new();
    fixture.events(&[json!({"type":"response.incomplete","response":{
        "usage":{"input_tokens":12340,"output_tokens":100,"input_tokens_details":{"cached_tokens":11008}},
        "incomplete_details":{"reason":"max_output_tokens"}
    }})]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::Usage {
                usage: TokenUsage {
                    input_tokens: 12340,
                    output_tokens: 100,
                    cached_input_tokens: 11008
                }
            },
            ModelEvent::Incomplete {
                reason: "max_output_tokens".into()
            },
        ]
    );
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
    assert_eq!(
        events,
        [
            ModelEvent::TextDelta {
                text: "Partial".into()
            },
            ModelEvent::Failed {
                reason: "La réponse a été interrompue avant la fin. Vous pouvez la relancer."
                    .into()
            },
        ]
    );
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
fn unfinished_calls_fail_without_becoming_executable() {
    let fixture = Fixture::new();
    for event in [
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
fn replay_content_respects_message_roles() {
    let fixture = Fixture::new();
    let mut request = request();
    request.input = vec![
        history(Role::System, "System policy"),
        history(Role::Assistant, "Earlier reply"),
        history(Role::User, "Continue"),
    ];
    fixture.events(&[json!({"type": "response.completed"})]);
    assert_eq!(fixture.collect(&request), [ModelEvent::Completed]);
    let seen = fixture.server.seen.lock().unwrap();
    let input = &seen.bodies[0]["input"];
    assert_eq!(
        input[0]["content"],
        json!([{"type": "input_text", "text": "System policy"}])
    );
    assert_eq!(
        input[1],
        json!({"role": "assistant", "content": "Earlier reply"})
    );
    assert_eq!(
        input[2]["content"],
        json!([{"type": "input_text", "text": "Continue"}])
    );
}

#[test]
fn non_user_images_are_rejected_before_http() {
    let fixture = Fixture::new();
    for role in [Role::Assistant, Role::System, Role::Tool] {
        let mut request = request();
        let mut entry = history(role, "");
        entry.images.push(ImageInput {
            attachment_id: "invalid-role".into(),
            url: Some("data:image/png;base64,AA==".into()),
            caption: Some("An image".into()),
        });
        request.input = vec![entry];
        let gateway: &dyn ModelGateway = &fixture.provider;
        let mut events = Vec::new();
        let error = gateway
            .stream(&request, &mut |event| {
                events.push(event);
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
        assert!(events.is_empty());
    }
    assert!(fixture.server.seen.lock().unwrap().bodies.is_empty());
}

#[test]
fn tool_string_output_is_not_json_quoted() {
    let fixture = Fixture::new();
    let mut request = request();
    let mut call = history(Role::Assistant, "");
    call.tool_items.push(ToolItem::Call {
        call: ToolCall {
            call_id: "call_1".into(),
            name: "read_skill".into(),
            arguments: json!({"name": "review"}),
        },
    });
    let mut result = history(Role::Tool, "");
    let text = "<external_data>\nReview checklist with \"quotes\" and é\n</external_data>";
    result.tool_items.push(ToolItem::Output {
        call_id: "call_1".into(),
        output: json!(text),
    });
    request.input.extend([call, result]);
    fixture.events(&[json!({"type": "response.completed"})]);
    fixture.collect(&request);
    assert_eq!(
        fixture.server.seen.lock().unwrap().bodies[0]["input"][2]["output"],
        text
    );
}

#[test]
fn service_tier_rejection_retries_without_tier() {
    for error in [
        json!({"error": {"param": "service_tier", "message": "Unsupported value"}}),
        json!({"error": {"message": "Unsupported service_tier"}}),
    ] {
        let fixture = Fixture::new();
        fixture.reply(ScriptedResponse::json(400, error));
        fixture.events(&[json!({"type": "response.completed"})]);
        assert_eq!(fixture.collect(&request()), [ModelEvent::Completed]);
        let seen = fixture.server.seen.lock().unwrap();
        assert_eq!(seen.bodies.len(), 2);
        let mut downgraded = seen.bodies[0].clone();
        assert!(downgraded
            .as_object_mut()
            .unwrap()
            .remove("service_tier")
            .is_some());
        assert_eq!(seen.bodies[1], downgraded);
    }
}

#[test]
fn effort_rejection_retries_without_reasoning() {
    for error in [
        json!({"error": {"param": "reasoning.effort", "message": "Unsupported value"}}),
        json!({"error": {"message": "Unsupported reasoning effort"}}),
    ] {
        let fixture = Fixture::new();
        fixture.reply(ScriptedResponse::json(400, error));
        fixture.events(&[json!({"type": "response.completed"})]);
        assert_eq!(fixture.collect(&request()), [ModelEvent::Completed]);
        let seen = fixture.server.seen.lock().unwrap();
        assert_eq!(seen.bodies.len(), 2);
        let mut downgraded = seen.bodies[0].clone();
        assert!(downgraded
            .as_object_mut()
            .unwrap()
            .remove("reasoning")
            .is_some());
        assert_eq!(seen.bodies[1], downgraded);
    }
}

#[test]
fn capability_fallbacks_share_one_authentication_refresh() {
    let fixture = Fixture::new();
    fixture.reply(ScriptedResponse::json(401, json!({"error": "expired"})));
    fixture.reply(ScriptedResponse::json(
        400,
        json!({"error": {"param": "service_tier"}}),
    ));
    fixture.reply(ScriptedResponse::json(
        400,
        json!({"error": {"param": "reasoning.effort"}}),
    ));
    fixture.events(&[json!({"type": "response.completed"})]);
    assert_eq!(fixture.collect(&request()), [ModelEvent::Completed]);
    let seen = fixture.server.seen.lock().unwrap();
    assert_eq!(seen.bodies.len(), 4);
    assert_eq!(seen.bodies[0], seen.bodies[1]);
    assert!(seen.bodies[2].get("service_tier").is_none());
    assert!(seen.bodies[2].get("reasoning").is_some());
    assert!(seen.bodies[3].get("reasoning").is_none());
    assert_eq!(seen.forms.len(), 1);
}

#[test]
fn fallback_attempts_are_bounded_and_ignore_unrelated_refusals() {
    for (status, error) in [
        (
            400,
            json!({"error": {"param": "input", "message": "Invalid content"}}),
        ),
        (
            429,
            json!({"error": {"param": "service_tier", "message": "Rate limited"}}),
        ),
    ] {
        let fixture = Fixture::new();
        fixture.reply(ScriptedResponse::json(status, error));
        assert!(matches!(
            fixture.collect(&request()).as_slice(),
            [ModelEvent::Failed { .. }]
        ));
        assert_eq!(fixture.server.seen.lock().unwrap().bodies.len(), 1);
    }
    let fixture = Fixture::new();
    for _ in 0..2 {
        fixture.reply(ScriptedResponse::json(
            400,
            json!({"error": {"param": "service_tier"}}),
        ));
    }
    assert!(matches!(
        fixture.collect(&request()).as_slice(),
        [ModelEvent::Failed { .. }]
    ));
    assert_eq!(fixture.server.seen.lock().unwrap().bodies.len(), 2);
}

#[test]
fn raw_responses_keeps_body_unmodified_and_does_not_downgrade_settings() {
    let fixture = Fixture::new();
    let body =
        json!({"model": "gpt-test", "stream": true, "service_tier": "priority", "input": []});
    fixture.reply(ScriptedResponse::json(
        400,
        json!({"error": {"param": "service_tier"}}),
    ));
    assert!(fixture.provider.responses(body.clone()).is_err());
    assert_eq!(fixture.server.seen.lock().unwrap().bodies, [body]);
}

#[test]
fn stream_failures_keep_french_provider_mapping_and_bound_remote_details() {
    let fixture = Fixture::new();
    for event in [
        json!({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_usage_limit_exceeded"}}}),
        json!({"type": "error", "error": "subscription_sharing_usage_limit_exceeded"}),
    ] {
        fixture.events(&[event]);
        assert!(
            matches!(fixture.collect(&request()).as_slice(), [ModelEvent::Failed { reason }] if reason.starts_with("Ezerbot a atteint la limite d’utilisation"))
        );
    }
    fixture.events(&[json!({"type": "response.failed", "response": {"error": {"code": "unknown", "message": "é".repeat(5000)}}})]);
    assert_eq!(
        fixture.collect(&request()),
        [ModelEvent::Failed {
            reason: format!(
                "OpenAI a signalé une erreur pendant la réponse : {}",
                "é".repeat(300)
            ),
        }]
    );
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

#[test]
fn completed_envelope_recovers_the_full_final_text_without_repeating_deltas() {
    let fixture = Fixture::new();
    let item = json!({"id":"final","type":"message","phase":"final_answer","content":[{"type":"output_text","text":"Réponse complète, pas un résumé."}]});
    fixture.events(&[json!({"type":"response.completed","response":{"output":[item.clone()]}})]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "Réponse complète, pas un résumé.".into()
            },
            ModelEvent::Completed
        ]
    );
    fixture.events(&[
        json!({"type":"response.output_text.delta","item_id":"final","delta":"Réponse complète, pas un résumé."}),
        json!({"type":"response.completed","response":{"output":[item]}})
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "Réponse complète, pas un résumé.".into()
            },
            ModelEvent::Completed
        ]
    );
}

#[test]
fn commentary_phase_is_not_concatenated_into_the_visible_answer() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type":"response.output_item.added","item":{"id":"msg_c","type":"message","role":"assistant","phase":"commentary"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_c","delta":"je réfléchis"}),
        json!({"type":"response.output_text.done","item_id":"msg_c","text":"je réfléchis"}),
        json!({"type":"response.output_item.done","item":{"id":"msg_c","type":"message","phase":"commentary","content":[{"type":"output_text","text":"je réfléchis"}]}}),
        json!({"type":"response.output_item.added","item":{"id":"msg_f","type":"message","role":"assistant","phase":"final_answer"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_f","delta":"17 × 23 = "}),
        json!({"type":"response.output_text.delta","item_id":"msg_f","delta":"391."}),
        json!({"type":"response.output_text.done","item_id":"msg_f","text":"17 × 23 = 391."}),
        json!({"type":"response.output_item.done","item":{"id":"msg_f","type":"message","phase":"final_answer","content":[{"type":"output_text","text":"17 × 23 = 391."}]}}),
        json!({"type":"response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "17 × 23 = ".into()
            },
            ModelEvent::TextDelta {
                text: "391.".into()
            },
            ModelEvent::Completed,
        ]
    );
}

#[test]
fn message_text_present_only_in_done_events_is_emitted_once() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type":"response.output_item.added","item":{"id":"msg_f","type":"message","role":"assistant"}}),
        json!({"type":"response.output_text.done","item_id":"msg_f","text":"seulement la fin"}),
        json!({"type":"response.output_item.done","item":{"id":"msg_f","type":"message","content":[{"type":"output_text","text":"seulement la fin"}]}}),
        json!({"type":"response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "seulement la fin".into()
            },
            ModelEvent::Completed,
        ]
    );
}

#[test]
fn done_text_does_not_repeat_deltas_and_commentary_only_is_one_reply() {
    let fixture = Fixture::new();
    fixture.events(&[
        json!({"type":"response.output_item.added","item":{"id":"msg","type":"message","role":"assistant"}}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"Bon"}),
        json!({"type":"response.output_text.delta","item_id":"msg","delta":"jour"}),
        json!({"type":"response.output_text.done","item_id":"msg","text":"Bonjour"}),
        json!({"type":"response.output_item.done","item":{"id":"msg","type":"message","content":[{"type":"output_text","text":"Bonjour"}]}}),
        json!({"type":"response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta { text: "Bon".into() },
            ModelEvent::TextDelta {
                text: "jour".into()
            },
            ModelEvent::Completed,
        ]
    );
    fixture.events(&[
        json!({"type":"response.output_item.added","item":{"id":"msg_c","type":"message","phase":"commentary"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_c","delta":"brouillon "}),
        json!({"type":"response.output_text.delta","item_id":"msg_c","delta":"inutile"}),
        json!({"type":"response.output_text.done","item_id":"msg_c","text":"brouillon inutile"}),
        json!({"type":"response.output_item.done","item":{"id":"msg_c","type":"message","phase":"commentary","content":[{"type":"output_text","text":"brouillon inutile"}]}}),
        json!({"type":"response.completed"}),
    ]);
    assert_eq!(
        fixture.collect(&request()),
        [
            ModelEvent::TextDelta {
                text: "brouillon inutile".into()
            },
            ModelEvent::Completed,
        ]
    );
}
