use super::super::{
    encode_base64, legacy_tool_observation, nonredundant_tool_result_content,
    resource_fallback_text, scoped_instruction_messages, ModelConversationRole, ModelInputContent,
    ModelRequest, ProviderToolCandidate, ProviderToolResult,
};
use serde_json::{json, Value};
pub(in crate::provider) fn anthropic_system_instructions(request: &ModelRequest) -> String {
    scoped_instruction_messages(request, true)
        .into_iter()
        .map(|(_, content)| content)
        .filter(|content| !content.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn anthropic_messages(request: &ModelRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    for (index, message) in request.input.conversation.iter().enumerate() {
        if message.role == ModelConversationRole::System {
            continue;
        }
        if message.role == ModelConversationRole::Tool
            && message.tool_calls.is_empty()
            && message.tool_results.is_empty()
        {
            let (call, result) = legacy_tool_observation(message, index);
            push_anthropic_message(
                &mut messages,
                "assistant",
                vec![json!({
                    "type": "tool_use",
                    "id": &call.id,
                    "name": &call.name,
                    "input": &call.arguments,
                })],
            );
            push_anthropic_message(&mut messages, "user", vec![anthropic_tool_result(&result)]);
            continue;
        }
        if !message.tool_calls.is_empty() {
            let mut content = anthropic_content_parts(&message.content, &message.content_parts);
            content.extend(message.tool_calls.iter().map(|call| {
                json!({
                    "type": "tool_use",
                    "id": &call.id,
                    "name": &call.name,
                    "input": &call.arguments,
                })
            }));
            push_anthropic_message(&mut messages, "assistant", content);
            if !message.tool_results.is_empty() {
                push_anthropic_message(
                    &mut messages,
                    "user",
                    message
                        .tool_results
                        .iter()
                        .map(anthropic_tool_result)
                        .collect(),
                );
            }
            continue;
        }
        if !message.tool_results.is_empty() {
            push_anthropic_message(
                &mut messages,
                "user",
                message
                    .tool_results
                    .iter()
                    .map(anthropic_tool_result)
                    .collect(),
            );
            continue;
        }
        let role = if message.role == ModelConversationRole::Assistant {
            "assistant"
        } else {
            "user"
        };
        push_anthropic_message(
            &mut messages,
            role,
            anthropic_content_parts(&message.content, &message.content_parts),
        );
    }
    push_anthropic_message(
        &mut messages,
        "user",
        anthropic_content_parts(
            &request.input.current_user.message,
            &request.input.current_user.content,
        ),
    );
    let runtime_context = scoped_instruction_messages(request, false)
        .into_iter()
        .map(|(_, content)| content)
        .collect::<Vec<_>>()
        .join("\n\n");
    if !runtime_context.trim().is_empty() {
        push_anthropic_message(
            &mut messages,
            "user",
            vec![json!({
                "type": "text",
                "text": format!("<runtime_context>\n{runtime_context}\n</runtime_context>"),
            })],
        );
    }
    for call in &request.input.tool_calls {
        push_anthropic_message(
            &mut messages,
            "assistant",
            vec![json!({
                "type": "tool_use",
                "id": &call.id,
                "name": &call.name,
                "input": &call.arguments,
            })],
        );
        let results = request
            .input
            .tool_results
            .iter()
            .filter(|result| result.call_id == call.id)
            .map(anthropic_tool_result)
            .collect::<Vec<_>>();
        if !results.is_empty() {
            push_anthropic_message(&mut messages, "user", results);
        }
    }
    for result in &request.input.tool_results {
        if !request
            .input
            .tool_calls
            .iter()
            .any(|call| call.id == result.call_id)
        {
            push_anthropic_message(&mut messages, "user", vec![anthropic_tool_result(result)]);
        }
    }
    if messages.is_empty() {
        messages.push(json!({ "role": "user", "content": [{ "type": "text", "text": "" }] }));
    }
    messages
}

fn push_anthropic_message(messages: &mut Vec<Value>, role: &str, content: Vec<Value>) {
    if content.is_empty() {
        return;
    }
    if let Some(last) = messages
        .last_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some(role))
    {
        if let Some(parts) = last.get_mut("content").and_then(Value::as_array_mut) {
            parts.extend(content);
            return;
        }
    }
    messages.push(json!({ "role": role, "content": content }));
}

fn anthropic_content_parts(legacy_text: &str, parts: &[ModelInputContent]) -> Vec<Value> {
    let mut content = Vec::new();
    if !legacy_text.is_empty() {
        content.push(json!({ "type": "text", "text": legacy_text }));
    }
    content.extend(parts.iter().map(|part| match part {
        ModelInputContent::Text { text } => json!({ "type": "text", "text": text }),
        ModelInputContent::Json { value } => json!({ "type": "text", "text": value.to_string() }),
        ModelInputContent::Image { content_type, data } => json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": content_type,
                "data": encode_base64(data),
            }
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": "text",
            "text": resource_fallback_text(uri, content_type.as_deref(), name.as_deref()),
        }),
    }));
    content
}

pub(in crate::provider) fn anthropic_tool_result(result: &ProviderToolResult) -> Value {
    let content_parts = nonredundant_tool_result_content(result);
    let mut content = anthropic_content_parts(&result.output, &content_parts);
    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }
    json!({
        "type": "tool_result",
        "tool_use_id": &result.call_id,
        "content": content,
        "is_error": result.is_error,
    })
}

pub(in crate::provider) fn anthropic_tools(candidates: &[ProviderToolCandidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|candidate| {
            json!({
                "name": &candidate.name,
                "description": &candidate.description,
                "input_schema": &candidate.input_schema,
            })
        })
        .collect()
}
