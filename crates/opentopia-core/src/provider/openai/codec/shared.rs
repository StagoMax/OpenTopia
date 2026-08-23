use crate::provider::{
    encode_base64, resource_fallback_text, ModelInputContent, ProviderToolCall, ProviderToolResult,
};
use serde_json::{json, Value};

pub(in crate::provider) fn openai_tool_call_message(call: &ProviderToolCall) -> Value {
    json!({
        "id": &call.id,
        "type": "function",
        "function": {
            "name": &call.name,
            "arguments": call.arguments.to_string()
        }
    })
}

pub(in crate::provider) fn provider_tool_result_content(result: &ProviderToolResult) -> String {
    let mut payload = json!({
        "output": &result.output,
        "isError": result.is_error,
        "metadata": &result.metadata
    });
    let content = nonredundant_tool_result_content(result);
    if !content.is_empty() {
        payload["content"] = json!(content
            .iter()
            .map(openai_tool_result_part)
            .collect::<Vec<_>>());
    }
    payload.to_string()
}

/// `ToolResult::text` stores the same text in both the legacy `output` field
/// and the typed content list. Provider envelopes retain `output` for wire
/// compatibility, so omit only the exact duplicate typed text part.
pub(in crate::provider) fn nonredundant_tool_result_content(
    result: &ProviderToolResult,
) -> Vec<ModelInputContent> {
    result
        .content
        .iter()
        .filter(|part| !matches!(part, ModelInputContent::Text { text } if text == &result.output))
        .cloned()
        .collect()
}

/// Responses API accepts typed input content in a function-call output. Keep image bytes under
/// the tool call's provenance instead of synthesizing a new user message.
pub(in crate::provider) fn responses_tool_result_output(result: &ProviderToolResult) -> Value {
    let has_typed_media = result
        .content
        .iter()
        .any(|part| matches!(part, ModelInputContent::Image { .. }));
    if !has_typed_media {
        return Value::String(provider_tool_result_content(result));
    }

    let mut content = vec![json!({
        "type": "input_text",
        "text": provider_tool_result_content(result),
    })];
    content.extend(result.content.iter().filter_map(|part| match part {
        ModelInputContent::Image { content_type, data } => Some(json!({
            "type": "input_image",
            "image_url": format!("data:{content_type};base64,{}", encode_base64(data)),
            "detail": "original",
        })),
        _ => None,
    }));
    Value::Array(content)
}

/// Chat Completions accepts native image content on user/assistant messages.
/// Resources and JSON have no portable Chat Completions content-part analogue,
/// so they remain explicit text/JSON representations instead of being dropped.
pub(in crate::provider) fn openai_message_content(
    legacy_text: &str,
    parts: &[ModelInputContent],
) -> Value {
    if parts.is_empty() {
        return Value::String(legacy_text.to_string());
    }

    let mut content = Vec::new();
    if !legacy_text.is_empty() {
        content.push(json!({ "type": "text", "text": legacy_text }));
    }
    content.extend(parts.iter().map(openai_input_part));
    Value::Array(content)
}

pub(in crate::provider) fn openai_input_part(part: &ModelInputContent) -> Value {
    match part {
        ModelInputContent::Text { text } => json!({ "type": "text", "text": text }),
        ModelInputContent::Json { value } => json!({
            "type": "text",
            "text": value.to_string()
        }),
        ModelInputContent::Image { content_type, data } => json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{content_type};base64,{}", encode_base64(data))
            }
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": "text",
            "text": resource_fallback_text(uri, content_type.as_deref(), name.as_deref())
        }),
    }
}

pub(in crate::provider) fn openai_tool_result_message(result: &ProviderToolResult) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": &result.call_id,
        "content": provider_tool_result_content(result)
    })
}

// A Chat Completions tool message is text-only across OpenAI-compatible APIs.
// Keep image metadata in its JSON envelope while the bytes travel in a native
// multimodal companion message after every tool result has been acknowledged.
pub(in crate::provider) fn openai_tool_result_part(part: &ModelInputContent) -> Value {
    match part {
        ModelInputContent::Text { text } => json!({ "type": "text", "text": text }),
        ModelInputContent::Json { value } => json!({ "type": "json", "value": value }),
        ModelInputContent::Image { content_type, data } => json!({
            "type": "image",
            "contentType": content_type,
            "bytes": data.len(),
            "delivery": "native_companion"
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": "resource",
            "uri": uri,
            "contentType": content_type,
            "name": name
        }),
    }
}

pub(in crate::provider) fn openai_tool_image_companion<'a>(
    results: impl IntoIterator<Item = &'a ProviderToolResult>,
) -> Option<Value> {
    let mut content = Vec::new();
    for result in results {
        for part in &result.content {
            if matches!(part, ModelInputContent::Image { .. }) {
                content.push(json!({
                    "type": "text",
                    "text": format!(
                        "Tool image output: {} (call {}).",
                        result.name, result.call_id
                    )
                }));
                content.push(openai_input_part(part));
            }
        }
    }

    (!content.is_empty()).then(|| json!({ "role": "user", "content": content }))
}
