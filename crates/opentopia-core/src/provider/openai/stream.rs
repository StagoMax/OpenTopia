use super::super::{
    AssistantOutputPhase, ModelFinishReason, ModelResponse, ModelStreamCallback, ModelStreamDelta,
    ModelUsage, ProviderToolCall, ProviderToolCandidate,
};
use super::codec::OPENAI_CHAT_ASSISTANT_STATE_TYPE;
use super::decode::{
    extract_provider_tool_calls_with_candidates, extract_response_text, parse_model_usage,
    parse_required_tool_arguments, parse_responses_apply_patch_call, responses_call_id,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Default)]
pub(in crate::provider) struct StreamingToolCall {
    pub(in crate::provider) id: String,
    pub(in crate::provider) name: String,
    pub(in crate::provider) arguments: String,
    pub(in crate::provider) arguments_present: bool,
    pub(in crate::provider) argument_wire_types: HashSet<&'static str>,
}

#[derive(Debug, Default)]
pub(in crate::provider) struct OpenAiStreamAccumulator {
    text: String,
    reasoning: String,
    reasoning_present: bool,
    tool_calls: BTreeMap<usize, StreamingToolCall>,
    usage: Option<ModelUsage>,
    finish_reason: Option<ModelFinishReason>,
}

impl OpenAiStreamAccumulator {
    pub(in crate::provider) fn apply_tool_call_deltas(
        &mut self,
        tool_calls: &[Value],
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        for (fallback_index, value) in tool_calls.iter().enumerate() {
            let index = value
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(fallback_index);
            let id_delta = value.get("id").and_then(Value::as_str);
            let name_delta = value
                .pointer("/function/name")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str);
            let arguments = value
                .pointer("/function/arguments")
                .or_else(|| value.get("arguments"))
                .or_else(|| value.get("input"));
            let arguments_delta = match arguments {
                Some(Value::String(arguments)) => arguments.clone(),
                Some(Value::Null) | None => String::new(),
                Some(arguments) => arguments.to_string(),
            };
            let call = self.tool_calls.entry(index).or_default();
            if let Some(id) = id_delta {
                call.id.push_str(id);
            }
            if let Some(name) = name_delta {
                call.name.push_str(name);
            }
            match arguments {
                // Standard OpenAI streams split a JSON string across deltas.
                Some(Value::String(_)) => {
                    call.arguments_present = true;
                    call.argument_wire_types.insert("string");
                    call.arguments.push_str(&arguments_delta);
                }
                // Compatible gateways sometimes send the completed argument
                // object directly. Treat that as a snapshot, not a fragment.
                Some(Value::Null) => {
                    call.argument_wire_types.insert("null");
                }
                None => {}
                Some(value) => {
                    call.arguments_present = true;
                    call.argument_wire_types.insert(match value {
                        Value::Object(_) => "object",
                        Value::Array(_) => "array",
                        Value::Bool(_) => "boolean",
                        Value::Number(_) => "number",
                        _ => "other",
                    });
                    call.arguments.clone_from(&arguments_delta);
                }
            }
            on_delta(ModelStreamDelta::ToolCall {
                index,
                id: id_delta.map(str::to_string),
                name: name_delta.map(str::to_string),
                arguments_delta,
            })?;
        }
        Ok(())
    }

    pub(in crate::provider) fn apply_tool_call_snapshots(&mut self, tool_calls: &[Value]) {
        for (fallback_index, value) in tool_calls.iter().enumerate() {
            let index = value
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(fallback_index);
            let call = self.tool_calls.entry(index).or_default();
            if let Some(id) = value
                .get("id")
                .or_else(|| value.get("call_id"))
                .and_then(Value::as_str)
            {
                call.id = id.to_string();
            }
            if let Some(name) = value
                .pointer("/function/name")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
            {
                call.name = name.to_string();
            }
            if let Some(arguments) = value
                .pointer("/function/arguments")
                .or_else(|| value.get("arguments"))
                .or_else(|| value.get("input"))
            {
                call.argument_wire_types.insert(match arguments {
                    Value::String(_) => "string",
                    Value::Null => "null",
                    Value::Object(_) => "object",
                    Value::Array(_) => "array",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                });
                let arguments = match arguments {
                    Value::String(arguments) => arguments.clone(),
                    Value::Null => String::new(),
                    arguments => arguments.to_string(),
                };
                if !arguments.trim().is_empty() {
                    call.arguments_present = true;
                    call.arguments = arguments;
                }
            }
        }
    }

    pub(in crate::provider) fn apply(
        &mut self,
        event: &Value,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        if let Some(error) = event.get("error") {
            anyhow::bail!("provider stream returned an error: {error}");
        }

        if let Some(usage) = parse_model_usage(event.get("usage")) {
            self.usage = Some(usage.clone());
            on_delta(ModelStreamDelta::Usage { usage })?;
        }

        if let Some(reason) = event
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            self.finish_reason = Some(chat_finish_reason(reason));
        }

        if let Some(delta) = event.pointer("/choices/0/delta") {
            if let Some(reasoning) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
            {
                self.reasoning_present = true;
                let reasoning = extract_reasoning_value(reasoning);
                if !reasoning.is_empty() {
                    self.reasoning.push_str(&reasoning);
                    on_delta(ModelStreamDelta::Reasoning { text: reasoning })?;
                }
            }
            let text = extract_stream_text(delta.get("content"));
            if !text.is_empty() {
                self.text.push_str(&text);
                on_delta(ModelStreamDelta::Text { text })?;
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                self.apply_tool_call_deltas(tool_calls, on_delta)?;
            }
        }

        // Some OpenAI-compatible gateways put the completed tool call on the
        // final `message` even when the preceding deltas only carried its name.
        if let Some(message) = event.pointer("/choices/0/message") {
            if let Some(reasoning) = message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
            {
                self.reasoning_present = true;
                if self.reasoning.is_empty() {
                    self.reasoning = extract_reasoning_value(reasoning);
                }
            }
            if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
                self.apply_tool_call_snapshots(tool_calls);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::provider) fn finish(self) -> anyhow::Result<ModelResponse> {
        self.finish_with_tools(&[])
    }

    pub(in crate::provider) fn finish_with_tools(
        self,
        tool_candidates: &[ProviderToolCandidate],
    ) -> anyhow::Result<ModelResponse> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                if call.name.is_empty() {
                    anyhow::bail!("streamed tool call {index} was missing a function name");
                }
                let id = if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                };
                if !call.arguments_present {
                    let wire_types = if call.argument_wire_types.is_empty() {
                        "missing".to_string()
                    } else {
                        call.argument_wire_types.into_iter().collect::<Vec<_>>().join(",")
                    };
                    anyhow::bail!(
                        "provider tool-call protocol error: function.arguments was absent for call '{id}' ({}, wire types: {wire_types})",
                        call.name
                    );
                }
                let _candidate = tool_candidates
                    .iter()
                    .find(|candidate| candidate.name == call.name);
                let arguments = parse_required_tool_arguments(
                    Some(&Value::String(call.arguments)),
                    "streamed function.arguments",
                    Some(&call.name),
                )?;
                Ok(ProviderToolCall {
                    id,
                    name: call.name,
                    arguments,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let provider_items = openai_chat_assistant_state_item(
            &self.text,
            self.reasoning_present.then_some(self.reasoning.as_str()),
            &tool_calls,
        )
        .into_iter()
        .collect();
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            usage: self.usage,
            response_id: None,
            provider_items,
            finish_reason: self
                .finish_reason
                .unwrap_or(ModelFinishReason::StreamInterrupted),
        })
    }
}

#[derive(Debug, Default)]
pub(in crate::provider) struct ResponsesStreamAccumulator {
    text: String,
    tool_calls: BTreeMap<usize, StreamingToolCall>,
    provider_items: BTreeMap<usize, Value>,
    output_phases: BTreeMap<usize, AssistantOutputPhase>,
    output_item_indices: HashMap<String, usize>,
    usage: Option<ModelUsage>,
    response_id: Option<String>,
    completed_response: Option<Value>,
    finish_reason: Option<ModelFinishReason>,
}

impl ResponsesStreamAccumulator {
    pub(in crate::provider) fn apply(
        &mut self,
        event: &Value,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(event_type, "error" | "response.failed") {
            anyhow::bail!("Responses stream returned an error: {event}");
        }
        if let Some(response_id) = event
            .get("response_id")
            .or_else(|| event.pointer("/response/id"))
            .and_then(Value::as_str)
        {
            self.response_id = Some(response_id.to_string());
        }
        if let Some(usage) = parse_model_usage(
            event
                .get("usage")
                .or_else(|| event.pointer("/response/usage")),
        ) {
            self.usage = Some(usage.clone());
            on_delta(ModelStreamDelta::Usage { usage })?;
        }

        match event_type {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    let index = event
                        .get("output_index")
                        .and_then(Value::as_u64)
                        .map(|index| index as usize)
                        .or_else(|| {
                            event
                                .get("item_id")
                                .and_then(Value::as_str)
                                .and_then(|item_id| self.output_item_indices.get(item_id).copied())
                        })
                        .unwrap_or(0);
                    let phase = event
                        .get("phase")
                        .and_then(Value::as_str)
                        .and_then(AssistantOutputPhase::from_wire)
                        .or_else(|| self.output_phases.get(&index).copied());
                    if phase != Some(AssistantOutputPhase::Commentary) {
                        self.text.push_str(delta);
                        on_delta(ModelStreamDelta::Text {
                            text: delta.to_string(),
                        })?;
                    }
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    on_delta(ModelStreamDelta::Reasoning {
                        text: delta.to_string(),
                    })?;
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.provider_items.len() as u64)
                    as usize;
                if let Some(item) = event.get("item") {
                    self.provider_items.insert(index, item.clone());
                    if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                        self.output_item_indices.insert(item_id.to_string(), index);
                    }
                    if item.get("type").and_then(Value::as_str) == Some("message") {
                        if let Some(phase) = item
                            .get("phase")
                            .and_then(Value::as_str)
                            .and_then(AssistantOutputPhase::from_wire)
                        {
                            self.output_phases.insert(index, phase);
                        }
                    }
                    if item.get("type").and_then(Value::as_str) == Some("function_call") {
                        let call = self.tool_calls.entry(index).or_default();
                        if let Some(id) = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                        {
                            call.id = id.to_string();
                        }
                        if let Some(name) = item.get("name").and_then(Value::as_str) {
                            call.name = name.to_string();
                        }
                        if event_type == "response.output_item.done" {
                            if let Some(arguments) = item.get("arguments").and_then(Value::as_str) {
                                call.arguments = arguments.to_string();
                            }
                        }
                    } else if item.get("type").and_then(Value::as_str) == Some("custom_tool_call") {
                        let call = self.tool_calls.entry(index).or_default();
                        if let Some(id) = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(Value::as_str)
                        {
                            call.id = id.to_string();
                        }
                        if let Some(name) = item.get("name").and_then(Value::as_str) {
                            call.name = name.to_string();
                        }
                        if event_type == "response.output_item.done" {
                            let input = item
                                .get("input")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            call.arguments = if call.name == "apply_patch" {
                                json!({ "patch": input }).to_string()
                            } else {
                                serde_json::from_str::<Value>(input)
                                    .unwrap_or_else(|_| json!({ "input": input }))
                                    .to_string()
                            };
                        }
                    } else if item.get("type").and_then(Value::as_str) == Some("apply_patch_call") {
                        if event_type == "response.output_item.done" {
                            let parsed = parse_responses_apply_patch_call(item, index)?;
                            self.tool_calls.insert(
                                index,
                                StreamingToolCall {
                                    id: parsed.id,
                                    name: parsed.name,
                                    arguments: parsed.arguments.to_string(),
                                    arguments_present: true,
                                    ..StreamingToolCall::default()
                                },
                            );
                        } else {
                            let call = self.tool_calls.entry(index).or_default();
                            call.id = responses_call_id(item, index);
                            call.name = "apply_patch".to_string();
                        }
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let delta = event.get("delta").and_then(Value::as_str).unwrap_or("");
                let call = self.tool_calls.entry(index).or_default();
                call.arguments.push_str(delta);
                on_delta(ModelStreamDelta::ToolCall {
                    index,
                    id: (!call.id.is_empty()).then(|| call.id.clone()),
                    name: (!call.name.is_empty()).then(|| call.name.clone()),
                    arguments_delta: delta.to_string(),
                })?;
            }
            "response.function_call_arguments.done" => {
                let index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                if let Some(arguments) = event.get("arguments").and_then(Value::as_str) {
                    self.tool_calls.entry(index).or_default().arguments = arguments.to_string();
                }
            }
            "response.completed" | "response.incomplete" => {
                if let Some(response) = event.get("response") {
                    self.completed_response = Some(response.clone());
                    self.finish_reason = Some(if event_type == "response.completed" {
                        responses_finish_reason(response, ModelFinishReason::Completed)
                    } else {
                        responses_finish_reason(
                            response,
                            ModelFinishReason::Incomplete("response.incomplete".to_string()),
                        )
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::provider) fn finish(self) -> anyhow::Result<ModelResponse> {
        self.finish_with_tools(&[])
    }

    pub(in crate::provider) fn finish_with_tools(
        mut self,
        tool_candidates: &[ProviderToolCandidate],
    ) -> anyhow::Result<ModelResponse> {
        if let Some(completed) = self.completed_response.as_ref() {
            let completed_text = extract_response_text(completed);
            let completed_has_messages = completed
                .get("output")
                .and_then(Value::as_array)
                .is_some_and(|output| {
                    output
                        .iter()
                        .any(|item| item.get("type").and_then(Value::as_str) == Some("message"))
                });
            if !completed_text.is_empty() || completed_has_messages {
                self.text = completed_text;
            }
            let completed_calls =
                extract_provider_tool_calls_with_candidates(completed, tool_candidates)?;
            if !completed_calls.is_empty() {
                self.tool_calls = completed_calls
                    .into_iter()
                    .enumerate()
                    .map(|(index, call)| {
                        (
                            index,
                            StreamingToolCall {
                                id: call.id,
                                name: call.name,
                                arguments: call.arguments.to_string(),
                                arguments_present: true,
                                ..StreamingToolCall::default()
                            },
                        )
                    })
                    .collect();
            }
            if let Some(output) = completed.get("output").and_then(Value::as_array) {
                self.provider_items = output
                    .iter()
                    .cloned()
                    .enumerate()
                    .collect::<BTreeMap<_, _>>();
            }
            self.usage = parse_model_usage(completed.get("usage")).or(self.usage);
            self.response_id = completed
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(self.response_id);
        }
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                if call.name.is_empty() {
                    anyhow::bail!("Responses tool call {index} was missing a function name");
                }
                let id = if call.id.is_empty() {
                    format!("call_{index}")
                } else {
                    call.id
                };
                let arguments = parse_required_tool_arguments(
                    Some(&Value::String(call.arguments)),
                    "streamed Responses function_call.arguments",
                    Some(&call.name),
                )?;
                Ok(ProviderToolCall {
                    id,
                    name: call.name,
                    arguments,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            usage: self.usage,
            response_id: self.response_id,
            provider_items: self.provider_items.into_values().collect(),
            finish_reason: self
                .finish_reason
                .unwrap_or(ModelFinishReason::StreamInterrupted),
        })
    }
}

pub(in crate::provider) fn chat_finish_reason(reason: &str) -> ModelFinishReason {
    match reason {
        "stop" | "end_turn" => ModelFinishReason::Stop,
        "tool_calls" | "function_call" | "tool_use" => ModelFinishReason::ToolCalls,
        "length" | "max_tokens" | "max_output_tokens" => ModelFinishReason::Length,
        "content_filter" => ModelFinishReason::ContentFilter,
        other => ModelFinishReason::Incomplete(other.to_string()),
    }
}

pub(in crate::provider) fn responses_finish_reason(
    response: &Value,
    fallback: ModelFinishReason,
) -> ModelFinishReason {
    match response.get("status").and_then(Value::as_str) {
        Some("completed") => ModelFinishReason::Completed,
        Some("incomplete") => response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .map(chat_finish_reason)
            .unwrap_or_else(|| ModelFinishReason::Incomplete("response incomplete".to_string())),
        Some(status) => ModelFinishReason::Incomplete(status.to_string()),
        None => fallback,
    }
}

pub(in crate::provider) fn extract_stream_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

pub(in crate::provider) fn openai_chat_assistant_state_item(
    content: &str,
    reasoning: Option<&str>,
    tool_calls: &[ProviderToolCall],
) -> Option<Value> {
    if tool_calls.is_empty() {
        return None;
    }
    let mut item = json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": content,
        "tool_call_ids": tool_calls.iter().map(|call| &call.id).collect::<Vec<_>>(),
    });
    if let Some(reasoning) = reasoning {
        item["reasoning_content"] = json!(reasoning);
    }
    Some(item)
}

pub(in crate::provider) fn extract_reasoning_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(extract_reasoning_value)
            .collect::<Vec<_>>()
            .join(""),
        Value::Object(fields) => ["text", "content", "summary", "output_text"]
            .into_iter()
            .find_map(|key| fields.get(key))
            .map(extract_reasoning_value)
            .unwrap_or_default(),
        _ => String::new(),
    }
}
