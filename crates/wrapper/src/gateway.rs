//! Responses adapter for the engine's synchronous model boundary.
//!
//! REA `reports/goal-execution-autonomy.md`, §§11 and 14: function results retain
//! their call ID, and response completion is not task completion. The text engine
//! on Grok's server is not observable; Responses framing here is our local adapter.
//! With `store:false`, callers supply the complete history on *every* invocation.
//! Neither this adapter nor the simulated gateway executes tools or grants approval.
//!
//! Contract debts for engine cutover: ModelEvent has no notice, error-kind or idle
//! progress/cancellation channel. Capability downgrades cannot yet show a notice,
//! Failed loses the provider's reconnect/usage action, and ignored SSE lines cannot
//! checkpoint cancellation. Reasoning items are not replayed without an encrypted
//! reasoning field in ModelRequest; only the complete representable history is sent.

use std::collections::{HashMap, HashSet};
use std::io::BufRead;

use grokbot_engine::contracts::{
    Error, ModelEvent, ModelGateway, ModelRequest, Result, Role, ToolItem,
};
use serde_json::{json, Value};

pub(crate) fn request_body(request: &ModelRequest) -> Result<Value> {
    let mut input = Vec::new();
    for entry in &request.input {
        if entry.role != Role::User && !entry.images.is_empty() {
            return Err(Error::InvalidInput(
                "Seuls les messages utilisateur peuvent contenir des images.".into(),
            ));
        }
        let role = match entry.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => {
                if !entry.text.is_empty() {
                    return Err(Error::InvalidInput(
                        "Un résultat d’outil doit utiliser function_call_output avec un call_id."
                            .into(),
                    ));
                }
                "tool"
            }
        };
        if entry.role == Role::Assistant {
            // Keep the legacy, accepted string representation. Responses rejects
            // input_text (and input_image) as assistant output-message content.
            if !entry.text.is_empty() {
                input.push(json!({"role": role, "content": entry.text}));
            }
        } else {
            let mut content = Vec::new();
            if !entry.text.is_empty() {
                content.push(json!({"type": "input_text", "text": entry.text}));
            }
            for image in &entry.images {
                if let Some(caption) = &image.caption {
                    content.push(json!({"type": "input_text", "text": caption}));
                }
                if let Some(url) = &image.url {
                    content.push(json!({"type": "input_image", "image_url": url}));
                }
            }
            if !content.is_empty() {
                input.push(json!({"role": role, "content": content}));
            }
        }
        for item in &entry.tool_items {
            input.push(match item {
                ToolItem::Call { call } => json!({
                    "type": "function_call", "call_id": call.call_id,
                    "name": call.name, "arguments": serde_json::to_string(&call.arguments)?,
                }),
                ToolItem::Output { call_id, output } => json!({
                    "type": "function_call_output", "call_id": call_id,
                    "output": match output {
                        Value::String(text) => text.clone(),
                        other => serde_json::to_string(other)?,
                    },
                }),
            });
        }
    }
    let mut body = json!({
        "model": request.model, "instructions": request.instructions,
        "input": input, "store": false, "stream": true,
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    let mut function = json!({
                        "type": "function", "name": tool.name, "parameters": tool.schema,
                        // ToolSpec permits ordinary JSON schemas, not only OpenAI's strict subset.
                        "strict": false,
                    });
                    // ToolSpec has no dedicated description field yet.
                    if let Some(description) = tool.schema["description"].as_str() {
                        function["description"] = json!(description);
                    }
                    function
                })
                .collect(),
        );
    }
    if let Some(effort) = &request.effort {
        body["reasoning"] = json!({"effort": effort});
    }
    if let Some(tier) = &request.service_tier {
        body["service_tier"] = json!(tier);
    }
    Ok(body)
}

#[derive(Default)]
struct PendingCall {
    call_id: String,
    name: String,
    arguments: String,
    ready: bool,
    emitted: bool,
}

#[derive(Default)]
struct Events {
    calls: HashMap<String, PendingCall>,
    emitted_ids: HashSet<String>,
}

const INTERRUPTED: &str = "La réponse a été interrompue avant la fin. Vous pouvez la relancer.";

fn failed(reason: impl Into<String>) -> ModelEvent {
    ModelEvent::Failed {
        reason: reason.into(),
    }
}

impl Events {
    /// Returns true only on a terminal event. Callback errors propagate unchanged.
    fn dispatch(
        &mut self,
        data: &str,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<bool> {
        if data == "[DONE]" {
            emit(failed(INTERRUPTED))?;
            return Ok(true);
        }
        let event: Value = match serde_json::from_str(data) {
            Ok(event) => event,
            Err(_) => {
                emit(failed(
                    "La réponse d’OpenAI contient un événement JSON illisible.",
                ))?;
                return Ok(true);
            }
        };
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "response.output_text.delta" => {
                if let Some(text) = event["delta"].as_str() {
                    emit(ModelEvent::TextDelta { text: text.into() })?;
                }
            }
            "response.output_item.added" | "response.output_item.done"
                if event["item"]["type"] == "function_call" =>
            {
                let item = &event["item"];
                let Some(id) = item["id"].as_str().filter(|id| !id.is_empty()) else {
                    emit(failed(
                        "OpenAI a envoyé un appel d’outil sans identifiant d’élément.",
                    ))?;
                    return Ok(true);
                };
                let call = self.calls.entry(id.into()).or_default();
                if let Some(call_id) = item["call_id"].as_str() {
                    call.call_id = call_id.into();
                }
                if let Some(name) = item["name"].as_str() {
                    call.name = name.into();
                }
                if let Some(arguments) = item["arguments"].as_str().filter(|args| !args.is_empty())
                {
                    call.arguments = arguments.into();
                }
                call.ready |= kind == "response.output_item.done";
                if self.emit_call(id, emit)? {
                    return Ok(true);
                }
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                let Some(id) = event["item_id"].as_str().filter(|id| !id.is_empty()) else {
                    emit(failed(
                        "OpenAI a envoyé des arguments d’outil sans identifiant d’élément.",
                    ))?;
                    return Ok(true);
                };
                let call = self.calls.entry(id.into()).or_default();
                if !call.emitted {
                    if kind == "response.function_call_arguments.delta" {
                        if let Some(delta) = event["delta"].as_str() {
                            call.arguments.push_str(delta);
                        }
                    } else {
                        if let Some(arguments) = event["arguments"].as_str() {
                            call.arguments = arguments.into();
                        }
                        call.ready = true;
                    }
                    if self.emit_call(id, emit)? {
                        return Ok(true);
                    }
                }
            }
            "response.completed" => {
                if self.calls.values().any(|call| !call.emitted) {
                    emit(failed(
                        "OpenAI a terminé la réponse avec un appel d’outil incomplet.",
                    ))?;
                } else {
                    emit(ModelEvent::Completed)?;
                }
                return Ok(true);
            }
            "response.incomplete" => {
                emit(ModelEvent::Incomplete {
                    reason: event["response"]["incomplete_details"]["reason"]
                        .as_str()
                        .unwrap_or("OpenAI a arrêté cette réponse avant la fin.")
                        .into(),
                })?;
                return Ok(true);
            }
            "response.failed" | "error" => {
                let source = if kind == "response.failed" {
                    &event["response"]
                } else {
                    &event
                };
                let (code, message, _) = crate::provider::error_parts(source);
                let failure = crate::provider::failure(0, &code, &message);
                emit(failed(failure.message))?;
                return Ok(true);
            }
            _ => {} // Reasoning, usage and non-function items are not executable tool calls.
        }
        Ok(false)
    }

    fn emit_call(
        &mut self,
        id: &str,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<bool> {
        let call = self.calls.get_mut(id).expect("registered above");
        if !call.ready || call.emitted || call.call_id.is_empty() || call.name.is_empty() {
            return Ok(false);
        }
        let arguments = match serde_json::from_str::<Value>(&call.arguments) {
            Ok(value) if value.is_object() => value,
            _ => {
                emit(failed(
                    "Les arguments de l’appel d’outil doivent être un objet JSON valide.",
                ))?;
                return Ok(true);
            }
        };
        if !self.emitted_ids.insert(call.call_id.clone()) {
            emit(failed(
                "OpenAI a réutilisé le même identifiant pour deux appels d’outil.",
            ))?;
            return Ok(true);
        }
        emit(ModelEvent::ToolCall {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            arguments,
        })?;
        call.emitted = true;
        Ok(false)
    }
}

pub(crate) fn stream_response(
    reader: impl BufRead,
    emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
) -> Result<()> {
    let mut events = Events::default();
    let mut data = Vec::new();
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => return emit(failed(INTERRUPTED)),
        };
        if line.is_empty() {
            if !data.is_empty() {
                if events.dispatch(&data.join("\n"), emit)? {
                    return Ok(());
                }
                data.clear();
            }
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
        }
    }
    // Accept a final event without a trailing blank line, but never mistake EOF for Completed.
    if !data.is_empty() && events.dispatch(&data.join("\n"), emit)? {
        return Ok(());
    }
    emit(failed(INTERRUPTED))
}

/// Offline mode keeps the existing canned rich-text replies, without inventing tool calls.
#[derive(Clone, Copy, Default)]
pub struct SimulatedGateway;

impl ModelGateway for SimulatedGateway {
    fn stream(
        &self,
        request: &ModelRequest,
        emit: &mut dyn FnMut(ModelEvent) -> Result<()>,
    ) -> Result<()> {
        let prompt = request
            .input
            .iter()
            .rev()
            .find(|entry| entry.role == Role::User)
            .map_or("", |entry| entry.text.as_str());
        emit(ModelEvent::TextDelta {
            text: crate::demo_reply(prompt).into(),
        })?;
        emit(ModelEvent::Completed)
    }
}
