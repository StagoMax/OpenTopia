use crate::model::{
    ProviderCacheTrace, ProviderCacheTraceProperty, ProviderCacheTraceSegment,
    ProviderCacheTraceSegmentKind,
};
use crate::model_context::{content_fingerprint, estimate_tokens};
use serde_json::Value;
use std::collections::HashMap;

const TRACE_SCHEMA_VERSION: u8 = 1;
const SEGMENT_FRAMING_TOKEN_ESTIMATE: usize = 4;

pub(crate) fn build_provider_cache_trace(
    body: &Value,
    prompt_cache_key: Option<&str>,
    previous_response_id_present: bool,
) -> Option<ProviderCacheTrace> {
    let object = body.as_object()?;
    let mut segments = Vec::new();
    let call_names = provider_tool_call_names(object);

    if let Some(messages) = object.get("messages").and_then(Value::as_array) {
        if let Some(system) = object.get("system") {
            segments.push(trace_segment(
                ProviderCacheTraceSegmentKind::Instructions,
                "system",
                None,
                system,
            ));
        }
        segments.extend(
            messages
                .iter()
                .enumerate()
                .map(|(index, message)| message_segment("messages", index, message, &call_names)),
        );
    } else if let Some(input) = object.get("input") {
        if let Some(instructions) = object.get("instructions") {
            segments.push(trace_segment(
                ProviderCacheTraceSegmentKind::Instructions,
                "instructions",
                None,
                instructions,
            ));
        }
        append_sequence_segments(&mut segments, "input", input, &call_names);
    } else if let Some(contents) = object.get("contents") {
        if let Some(instructions) = object
            .get("system_instruction")
            .or_else(|| object.get("systemInstruction"))
        {
            segments.push(trace_segment(
                ProviderCacheTraceSegmentKind::Instructions,
                "systemInstruction",
                None,
                instructions,
            ));
        }
        append_sequence_segments(&mut segments, "contents", contents, &call_names);
    }

    let tool_catalog_hash = object
        .get("tools")
        .or_else(|| object.get("functions"))
        .or_else(|| object.get("toolConfig"))
        .map(value_fingerprint);
    let prompt_cache_key_hash = prompt_cache_key
        .map(|value| content_fingerprint(value.as_bytes()))
        .or_else(|| {
            object
                .get("prompt_cache_key")
                .or_else(|| object.get("promptCacheKey"))
                .and_then(Value::as_str)
                .map(|value| content_fingerprint(value.as_bytes()))
        });
    let previous_response_id_present = previous_response_id_present
        || object
            .get("previous_response_id")
            .or_else(|| object.get("previousResponseId"))
            .is_some_and(|value| !value.is_null());
    let configuration = configuration_trace(object);

    if segments.is_empty() && tool_catalog_hash.is_none() && configuration.is_empty() {
        return None;
    }
    let prefix_hash = content_fingerprint(
        segments
            .iter()
            .map(|segment| segment.content_hash.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    );
    Some(ProviderCacheTrace {
        schema_version: TRACE_SCHEMA_VERSION,
        prefix_hash,
        segments,
        tool_catalog_hash,
        prompt_cache_key_hash,
        previous_response_id_present,
        configuration,
    })
}

fn append_sequence_segments(
    target: &mut Vec<ProviderCacheTraceSegment>,
    source: &str,
    value: &Value,
    call_names: &HashMap<String, String>,
) {
    if let Some(items) = value.as_array() {
        target.extend(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| message_segment(source, index, item, call_names)),
        );
    } else {
        target.push(trace_segment(
            ProviderCacheTraceSegmentKind::InputItem,
            source,
            None,
            value,
        ));
    }
}

fn message_segment(
    collection: &str,
    index: usize,
    value: &Value,
    call_names: &HashMap<String, String>,
) -> ProviderCacheTraceSegment {
    let object = value.as_object();
    let role = object
        .and_then(|value| value.get("role"))
        .and_then(Value::as_str);
    let item_type = object
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str);
    let tool_calls = object
        .and_then(|value| value.get("tool_calls"))
        .or_else(|| object.and_then(|value| value.get("toolCalls")))
        .and_then(Value::as_array);
    let tool_call_id = object
        .and_then(|value| value.get("tool_call_id"))
        .or_else(|| object.and_then(|value| value.get("call_id")))
        .or_else(|| object.and_then(|value| value.get("callId")))
        .and_then(Value::as_str);

    let (kind, name) = if tool_image_companion(value) {
        (ProviderCacheTraceSegmentKind::ToolImage, None)
    } else if let Some(calls) = tool_calls.filter(|calls| !calls.is_empty()) {
        (
            ProviderCacheTraceSegmentKind::ToolCall,
            joined_tool_names(calls),
        )
    } else if matches!(role, Some("tool"))
        || matches!(item_type, Some("function_call_output" | "tool_result"))
    {
        (
            ProviderCacheTraceSegmentKind::ToolResult,
            tool_call_id.and_then(|id| call_names.get(id).cloned()),
        )
    } else {
        let kind = match role {
            Some("system") => ProviderCacheTraceSegmentKind::SystemMessage,
            Some("developer") => ProviderCacheTraceSegmentKind::DeveloperMessage,
            Some("user") => ProviderCacheTraceSegmentKind::UserMessage,
            Some("assistant") => ProviderCacheTraceSegmentKind::AssistantMessage,
            _ if matches!(item_type, Some("function_call" | "tool_call")) => {
                ProviderCacheTraceSegmentKind::ToolCall
            }
            _ if item_type.is_some() => ProviderCacheTraceSegmentKind::InputItem,
            _ => ProviderCacheTraceSegmentKind::Unknown,
        };
        let name = object
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(safe_name);
        (kind, name)
    };

    trace_segment(kind, format!("{collection}[{index}]"), name, value)
}

fn provider_tool_call_names(object: &serde_json::Map<String, Value>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for collection in ["messages", "input", "contents"] {
        let Some(items) = object.get(collection).and_then(Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(item) = item.as_object() else {
                continue;
            };
            if let Some(calls) = item
                .get("tool_calls")
                .or_else(|| item.get("toolCalls"))
                .and_then(Value::as_array)
            {
                for call in calls {
                    let Some(id) = call.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let name = call
                        .get("function")
                        .and_then(|value| value.get("name"))
                        .or_else(|| call.get("name"))
                        .and_then(Value::as_str)
                        .map(safe_name);
                    if let Some(name) = name {
                        result.insert(id.to_string(), name);
                    }
                }
            }
            if matches!(
                item.get("type").and_then(Value::as_str),
                Some("function_call")
            ) {
                if let (Some(id), Some(name)) = (
                    item.get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str),
                    item.get("name").and_then(Value::as_str),
                ) {
                    result.insert(id.to_string(), safe_name(name));
                }
            }
        }
    }
    result
}

fn joined_tool_names(calls: &[Value]) -> Option<String> {
    let mut names = calls
        .iter()
        .filter_map(|call| {
            call.get("function")
                .and_then(|value| value.get("name"))
                .or_else(|| call.get("name"))
                .and_then(Value::as_str)
                .map(safe_name)
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    (!names.is_empty()).then(|| names.into_iter().take(4).collect::<Vec<_>>().join(", "))
}

fn tool_image_companion(value: &Value) -> bool {
    let Some(content) = value.get("content").and_then(Value::as_array) else {
        return false;
    };
    let has_image = content.iter().any(|part| {
        part.get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.contains("image"))
    });
    let has_marker = content.iter().any(|part| {
        part.get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.starts_with("Tool image output:"))
    });
    has_image && has_marker
}

fn configuration_trace(object: &serde_json::Map<String, Value>) -> Vec<ProviderCacheTraceProperty> {
    const EXCLUDED: &[&str] = &[
        "messages",
        "input",
        "contents",
        "system",
        "system_instruction",
        "systemInstruction",
        "instructions",
        "tools",
        "functions",
        "toolConfig",
        "model",
        "prompt_cache_key",
        "promptCacheKey",
        "previous_response_id",
        "previousResponseId",
        "stream",
        "stream_options",
    ];
    object
        .iter()
        .filter(|(name, _)| !EXCLUDED.contains(&name.as_str()))
        .map(|(name, value)| ProviderCacheTraceProperty {
            name: safe_name(name),
            value_hash: value_fingerprint(value),
        })
        .collect()
}

fn trace_segment(
    kind: ProviderCacheTraceSegmentKind,
    source: impl Into<String>,
    name: Option<String>,
    value: &Value,
) -> ProviderCacheTraceSegment {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    let token_estimate = std::str::from_utf8(&encoded)
        .map(estimate_tokens)
        .unwrap_or_else(|_| encoded.len().div_ceil(4))
        .saturating_add(SEGMENT_FRAMING_TOKEN_ESTIMATE);
    ProviderCacheTraceSegment {
        kind,
        source: source.into(),
        name,
        content_hash: content_fingerprint(&encoded),
        token_estimate,
    }
}

fn value_fingerprint(value: &Value) -> String {
    serde_json::to_vec(value)
        .map(|encoded| content_fingerprint(&encoded))
        .unwrap_or_else(|_| content_fingerprint(b"serialization_error"))
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | ':' | '/' | ' ')
        })
        .take(80)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn trace_identifies_tool_messages_and_never_keeps_content() {
        let body = json!({
            "model": "example",
            "messages": [
                {"role": "system", "content": "private instructions"},
                {"role": "assistant", "content": "", "tool_calls": [{
                    "id": "call-1",
                    "function": {"name": "filesystem", "arguments": "{}"},
                    "type": "function"
                }]},
                {"role": "tool", "tool_call_id": "call-1", "content": "private output"},
                {"role": "user", "content": [
                    {"type": "text", "text": "Tool image output: view_attachment (call call-1)."},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,secret"}}
                ]}
            ],
            "tools": [{"type": "function", "function": {"name": "filesystem"}}],
            "reasoning_effort": "high",
            "stream": true
        });

        let trace =
            build_provider_cache_trace(&body, Some("cache-secret"), false).expect("cache trace");
        assert_eq!(trace.segments.len(), 4);
        assert_eq!(
            trace.segments[1].kind,
            ProviderCacheTraceSegmentKind::ToolCall
        );
        assert_eq!(trace.segments[1].name.as_deref(), Some("filesystem"));
        assert_eq!(
            trace.segments[2].kind,
            ProviderCacheTraceSegmentKind::ToolResult
        );
        assert_eq!(trace.segments[2].name.as_deref(), Some("filesystem"));
        assert_eq!(
            trace.segments[3].kind,
            ProviderCacheTraceSegmentKind::ToolImage
        );
        assert!(trace
            .configuration
            .iter()
            .any(|property| property.name == "reasoning_effort"));
        let encoded = serde_json::to_string(&trace).expect("serialize trace");
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains("cache-secret"));
        assert!(!encoded.contains("base64"));
    }
}
