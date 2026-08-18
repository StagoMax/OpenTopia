use super::super::{
    redact_transport_value, ModelFinishReason, ModelResponse, ModelUsage, ProviderToolCall,
    ProviderToolCandidate,
};
use super::stream::{
    chat_finish_reason, extract_reasoning_value, openai_chat_assistant_state_item,
    responses_finish_reason,
};
use crate::model_context::content_fingerprint;
use serde_json::{json, Value};

pub(in crate::provider) fn model_response_observation(response: &ModelResponse) -> Value {
    json!({
        "responseId": response.response_id,
        "textChars": response.text.len(),
        "toolCalls": response.tool_calls,
        "finishReason": response.finish_reason,
        "usage": response.usage,
        "providerItems": redact_transport_value(&Value::Array(response.provider_items.clone())),
    })
}

#[cfg(test)]
pub(in crate::provider) fn parse_model_response_body(
    body: &Value,
) -> anyhow::Result<ModelResponse> {
    parse_model_response_body_with_tools(body, &[])
}

pub(in crate::provider) fn parse_model_response_body_with_tools(
    body: &Value,
    tool_candidates: &[ProviderToolCandidate],
) -> anyhow::Result<ModelResponse> {
    let tool_calls = extract_provider_tool_calls_with_candidates(body, tool_candidates)?;
    let mut provider_items = body
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if provider_items.is_empty() {
        let reasoning = body
            .pointer("/choices/0/message/reasoning_content")
            .or_else(|| body.pointer("/choices/0/message/reasoning"))
            .map(extract_reasoning_value);
        if let Some(item) = openai_chat_assistant_state_item(
            &extract_response_text(body),
            reasoning.as_deref(),
            &tool_calls,
        ) {
            provider_items.push(item);
        }
    }
    Ok(ModelResponse {
        text: extract_response_text(body),
        tool_calls,
        usage: parse_model_usage(body.get("usage")),
        // A Chat Completions `id` is an observable request identifier, not a
        // resumable conversation cursor. Treating it like a Responses API
        // `previous_response_id` drops the assistant-state items required to
        // replay tool-call grouping and reasoning on the next turn.
        response_id: None,
        provider_items,
        finish_reason: body
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(chat_finish_reason)
            .unwrap_or_else(|| responses_finish_reason(body, ModelFinishReason::StreamInterrupted)),
    })
}

pub(in crate::provider) fn parse_model_usage(value: Option<&Value>) -> Option<ModelUsage> {
    let usage = value?.as_object()?;
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(input_tokens.saturating_add(output_tokens));
    let cached_input_tokens = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);
    let cache_write_tokens = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"))
        .and_then(|details| details.get("cache_write_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("cache_write_tokens").and_then(Value::as_u64));
    let reasoning_tokens = usage
        .get("completion_tokens_details")
        .or_else(|| usage.get("output_tokens_details"))
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64);

    Some(ModelUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        cache_write_tokens,
        reasoning_tokens,
    })
}

pub(in crate::provider) fn extract_response_text(body: &Value) -> String {
    if let Some(text) = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        return text.to_string();
    }

    if let Some(parts) = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_array)
    {
        let text = parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.get("content").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }

    let response_messages = body
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .collect::<Vec<_>>();
    let has_final_phase = response_messages
        .iter()
        .any(|item| item.get("phase").and_then(Value::as_str) == Some("final_answer"));
    let responses_text = response_messages
        .iter()
        .copied()
        .filter(|item| {
            let phase = item.get("phase").and_then(Value::as_str);
            if has_final_phase {
                phase == Some("final_answer")
            } else {
                // Legacy providers omit phase. Commentary is never promoted
                // to the final answer even when no final item has arrived.
                phase != Some("commentary")
            }
        })
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(render_responses_output_text_part)
        .collect::<Vec<_>>()
        .join("");
    if !responses_text.is_empty() || !response_messages.is_empty() {
        return responses_text;
    }

    body.get("output_text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(super) fn render_responses_output_text_part(part: &Value) -> Option<String> {
    let text = part.get("text").and_then(Value::as_str)?;
    let annotations = part
        .get("annotations")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Some(apply_url_citations(text, annotations))
}

pub(super) fn apply_url_citations(text: &str, annotations: &[Value]) -> String {
    let mut ranges = Vec::new();
    let mut fallback_sources = Vec::new();
    for annotation in annotations {
        if annotation.get("type").and_then(Value::as_str) != Some("url_citation") {
            continue;
        }
        let citation = annotation.get("url_citation").unwrap_or(annotation);
        let Some(url) = citation.get("url").and_then(Value::as_str) else {
            continue;
        };
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            continue;
        }
        let title = citation
            .get("title")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Source")
            .to_string();
        match (
            citation.get("start_index").and_then(Value::as_u64),
            citation.get("end_index").and_then(Value::as_u64),
        ) {
            (Some(start), Some(end)) if start < end => {
                ranges.push((start as usize, end as usize, url.to_string(), title));
            }
            _ => fallback_sources.push((url.to_string(), title)),
        }
    }
    if ranges.is_empty() && fallback_sources.is_empty() {
        return text.to_string();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut char_boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    char_boundaries.push(text.len());
    ranges.sort_by_key(|(start, _, _, _)| std::cmp::Reverse(*start));
    let mut rendered = text.to_string();
    let mut upper_bound = char_boundaries.len().saturating_sub(1);
    for (mut start, mut end, url, title) in ranges {
        while start < end && chars.get(start).is_some_and(|value| value.is_whitespace()) {
            start += 1;
        }
        while end > start
            && chars
                .get(end.saturating_sub(1))
                .is_some_and(|value| value.is_whitespace())
        {
            end -= 1;
        }
        while end < chars.len()
            && chars[end].is_alphanumeric()
            && chars
                .get(end.saturating_sub(1))
                .is_some_and(|value| value.is_alphanumeric())
        {
            end += 1;
        }
        if end > upper_bound || start >= end {
            fallback_sources.push((url, title));
            continue;
        }
        let byte_start = char_boundaries[start];
        let byte_end = char_boundaries[end];
        let label = text[byte_start..byte_end].trim();
        let label = if label.is_empty() {
            title.as_str()
        } else {
            label
        };
        rendered.replace_range(
            byte_start..byte_end,
            &format!(
                "[{}]({})",
                escape_markdown_link_label(label),
                escape_markdown_link_url(&url)
            ),
        );
        upper_bound = start;
    }

    let mut seen = std::collections::HashSet::new();
    let fallback_sources = fallback_sources
        .into_iter()
        .filter(|(url, _)| seen.insert(url.clone()))
        .collect::<Vec<_>>();
    if !fallback_sources.is_empty() {
        rendered.push_str("\n\nSources:\n");
        for (url, title) in fallback_sources {
            rendered.push_str(&format!(
                "- [{}]({})\n",
                escape_markdown_link_label(&title),
                escape_markdown_link_url(&url)
            ));
        }
        rendered.pop();
    }
    rendered
}

pub(super) fn escape_markdown_link_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

pub(super) fn escape_markdown_link_url(value: &str) -> String {
    value.replace(' ', "%20").replace(')', "%29")
}

#[cfg(test)]
pub(in crate::provider) fn extract_provider_tool_calls(
    body: &Value,
) -> anyhow::Result<Vec<ProviderToolCall>> {
    extract_provider_tool_calls_with_candidates(body, &[])
}

pub(super) fn extract_provider_tool_calls_with_candidates(
    body: &Value,
    tool_candidates: &[ProviderToolCandidate],
) -> anyhow::Result<Vec<ProviderToolCall>> {
    let mut calls = Vec::new();

    if let Some(tool_calls) = body
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    {
        for (index, call) in tool_calls.iter().enumerate() {
            calls.push(parse_chat_tool_call(call, index, tool_candidates)?);
        }
    }

    if let Some(function_call) = body
        .pointer("/choices/0/message/function_call")
        .filter(|value| value.is_object())
    {
        calls.push(parse_legacy_function_call(function_call, calls.len())?);
    }

    if let Some(output) = body.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("function_call") => {
                    calls.push(parse_responses_function_call(item, calls.len())?)
                }
                Some("custom_tool_call") => {
                    calls.push(parse_responses_custom_tool_call(item, calls.len())?)
                }
                Some("apply_patch_call") => {
                    calls.push(parse_responses_apply_patch_call(item, calls.len())?)
                }
                _ => {}
            }
        }
    }

    Ok(calls)
}

pub(super) fn parse_chat_tool_call(
    value: &Value,
    index: usize,
    tool_candidates: &[ProviderToolCandidate],
) -> anyhow::Result<ProviderToolCall> {
    let function = value
        .get("function")
        .ok_or_else(|| anyhow::anyhow!("tool call missing function payload: {value}"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("tool call missing function name: {value}"))?;
    let _candidate = tool_candidates
        .iter()
        .find(|candidate| candidate.name == name);
    let arguments =
        parse_required_tool_arguments(function.get("arguments"), "function.arguments", Some(name))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{index}"));

    Ok(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments,
    })
}

pub(super) fn parse_legacy_function_call(
    value: &Value,
    index: usize,
) -> anyhow::Result<ProviderToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("function_call missing name: {value}"))?;
    Ok(ProviderToolCall {
        id: format!("call_{index}"),
        name: name.to_string(),
        arguments: parse_required_tool_arguments(
            value.get("arguments"),
            "legacy function_call.arguments",
            Some(name),
        )?,
    })
}

pub(super) fn parse_responses_function_call(
    value: &Value,
    index: usize,
) -> anyhow::Result<ProviderToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("function_call missing name: {value}"))?;
    let id = value
        .get("call_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{index}"));

    Ok(ProviderToolCall {
        id,
        name: name.to_string(),
        arguments: parse_required_tool_arguments(
            value.get("arguments"),
            "Responses function_call.arguments",
            Some(name),
        )?,
    })
}

pub(super) fn responses_call_id(value: &Value, index: usize) -> String {
    value
        .get("call_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("call_{index}"))
}

pub(super) fn parse_responses_custom_tool_call(
    value: &Value,
    index: usize,
) -> anyhow::Result<ProviderToolCall> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("custom_tool_call missing name: {value}"))?;
    let input = value
        .get("input")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let arguments = if name == "apply_patch" {
        json!({ "patch": input })
    } else {
        serde_json::from_str(input).unwrap_or_else(|_| json!({ "input": input }))
    };
    Ok(ProviderToolCall {
        id: responses_call_id(value, index),
        name: name.to_string(),
        arguments,
    })
}

pub(super) fn parse_responses_apply_patch_call(
    value: &Value,
    index: usize,
) -> anyhow::Result<ProviderToolCall> {
    let operation = value
        .get("operation")
        .filter(|operation| operation.is_object())
        .ok_or_else(|| anyhow::anyhow!("apply_patch_call missing operation: {value}"))?;
    let operation_type = operation
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("apply_patch_call operation missing type: {value}"))?;
    if !matches!(
        operation_type,
        "create_file" | "update_file" | "delete_file"
    ) {
        anyhow::bail!("unsupported apply_patch operation type: {operation_type}");
    }
    if operation.get("path").and_then(Value::as_str).is_none() {
        anyhow::bail!("apply_patch_call operation missing path: {value}");
    }
    if operation_type != "delete_file" && operation.get("diff").and_then(Value::as_str).is_none() {
        anyhow::bail!("apply_patch_call operation missing diff: {value}");
    }
    Ok(ProviderToolCall {
        id: responses_call_id(value, index),
        name: "apply_patch".to_string(),
        arguments: json!({ "operation": operation }),
    })
}

pub(in crate::provider) fn parse_required_tool_arguments(
    value: Option<&Value>,
    field: &str,
    tool_name: Option<&str>,
) -> anyhow::Result<Value> {
    match value {
        None | Some(Value::Null) => {
            anyhow::bail!("provider tool-call protocol error: {field} is missing")
        }
        Some(Value::String(arguments)) if arguments.trim().is_empty() => {
            anyhow::bail!("provider tool-call protocol error: {field} is empty")
        }
        Some(Value::String(arguments)) => serde_json::from_str(arguments).map_err(|source| {
            anyhow::Error::new(InvalidToolArgumentsJson::new(
                field, tool_name, arguments, source,
            ))
        }),
        Some(value) => Ok(value.clone()),
    }
}

pub(in crate::provider) const INVALID_TOOL_ARGUMENTS_JSON_KEY: &str =
    "$opentopiaInvalidToolArguments";

/// Legacy persisted turns may still contain the pre-v2 placeholder. New
/// provider responses never create it: malformed wire arguments fail at the
/// adapter boundary before a `ProviderToolCall` can be constructed.
pub(crate) fn invalid_tool_arguments_json_details(arguments: &Value) -> Option<&Value> {
    arguments
        .as_object()?
        .get(INVALID_TOOL_ARGUMENTS_JSON_KEY)
        .filter(|details| details.is_object())
}

pub(super) const INVALID_TOOL_ARGUMENTS_EXCERPT_RADIUS: usize = 32;

#[derive(Debug)]
pub(super) struct InvalidToolArgumentsJson {
    field: String,
    tool_name: Option<String>,
    source: serde_json::Error,
    argument_bytes: usize,
    fingerprint: String,
    error_offset: usize,
    redacted_excerpt: String,
}

impl InvalidToolArgumentsJson {
    fn new(
        field: &str,
        tool_name: Option<&str>,
        arguments: &str,
        source: serde_json::Error,
    ) -> Self {
        let error_offset = json_error_offset(arguments, source.line(), source.column());
        Self {
            field: field.to_string(),
            tool_name: tool_name.map(str::to_string),
            argument_bytes: arguments.len(),
            fingerprint: content_fingerprint(arguments.as_bytes()),
            redacted_excerpt: redacted_json_error_excerpt(arguments, error_offset),
            error_offset,
            source,
        }
    }

    fn observation(&self) -> Value {
        json!({
            "field": &self.field,
            "toolName": self.tool_name.as_deref(),
            "reason": self.source.to_string(),
            "argumentBytes": self.argument_bytes,
            "fingerprint": &self.fingerprint,
            "errorLine": self.source.line(),
            "errorColumn": self.source.column(),
            "errorOffset": self.error_offset,
            "redactedExcerpt": &self.redacted_excerpt,
        })
    }
}

impl std::fmt::Display for InvalidToolArgumentsJson {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "provider tool-call protocol error: {} is not valid JSON: {} (tool={}, argumentBytes={}, fingerprint={}, errorOffset={}, redactedExcerpt={})",
            self.field,
            self.source,
            self.tool_name.as_deref().unwrap_or("unknown"),
            self.argument_bytes,
            self.fingerprint,
            self.error_offset,
            serde_json::to_string(&self.redacted_excerpt)
                .unwrap_or_else(|_| "\"<unavailable>\"".to_string())
        )
    }
}

impl std::error::Error for InvalidToolArgumentsJson {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub(in crate::provider) fn tool_call_protocol_error_observation(
    error: &anyhow::Error,
    recovery: Option<&str>,
) -> Value {
    let mut observation = json!({ "providerProtocolError": error.to_string() });
    if let Some(details) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<InvalidToolArgumentsJson>())
    {
        observation["invalidToolArguments"] = details.observation();
    }
    if let Some(recovery) = recovery {
        observation["recovery"] = json!(recovery);
    }
    observation
}

pub(super) fn json_error_offset(input: &str, line: usize, column: usize) -> usize {
    let target_line = line.max(1);
    let target_column = column.max(1);
    let mut current_line = 1usize;
    let mut current_column = 1usize;
    for (offset, character) in input.char_indices() {
        if current_line == target_line && current_column == target_column {
            return offset;
        }
        if character == '\n' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }
    input.len()
}

pub(super) fn redacted_json_error_excerpt(input: &str, error_offset: usize) -> String {
    let start = input
        .char_indices()
        .map(|(offset, _)| offset)
        .filter(|offset| *offset <= error_offset)
        .rev()
        .nth(INVALID_TOOL_ARGUMENTS_EXCERPT_RADIUS)
        .unwrap_or(0);
    let end = input
        .char_indices()
        .map(|(offset, _)| offset)
        .find(|offset| {
            *offset >= error_offset.saturating_add(INVALID_TOOL_ARGUMENTS_EXCERPT_RADIUS)
        })
        .unwrap_or(input.len());
    let excerpt = &input[start..end];
    let mut redacted = String::with_capacity(excerpt.len());
    let (mut in_string, mut escaped) =
        input[..start]
            .chars()
            .fold((false, false), |(in_string, escaped), character| {
                if !in_string {
                    (character == '"', false)
                } else if escaped {
                    (true, false)
                } else if character == '\\' {
                    (true, true)
                } else {
                    (character != '"', false)
                }
            });
    let mut characters = excerpt.chars().peekable();
    while let Some(character) = characters.next() {
        if in_string {
            if escaped {
                escaped = false;
                redacted.push('*');
            } else if character == '\\' {
                escaped = true;
                redacted.push('*');
            } else if character == '"' {
                in_string = false;
                redacted.push('"');
            } else {
                redacted.push('*');
            }
        } else if character == '"' {
            in_string = true;
            redacted.push('"');
        } else if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            let mut token = String::from(character);
            while characters.peek().is_some_and(|next| {
                next.is_ascii_alphanumeric() || matches!(next, '_' | '-' | '.' | '+')
            }) {
                token.push(characters.next().expect("peeked token character"));
            }
            if matches!(token.as_str(), "true" | "false" | "null" | "none" | "all") {
                redacted.push_str(&token);
            } else {
                redacted.extend(std::iter::repeat('*').take(token.chars().count()));
            }
        } else if character.is_whitespace()
            || matches!(character, '{' | '}' | '[' | ']' | ':' | ',')
        {
            redacted.push(character);
        } else {
            redacted.push('*');
        }
    }
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        redacted,
        if end < input.len() { "…" } else { "" }
    )
}

pub(super) fn json_value_matches_schema_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => true,
    }
}

pub(crate) fn tool_input_schema_error(schema: &Value, value: &Value, path: &str) -> Option<String> {
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            if let Some(error) = tool_input_schema_error(branch, value, path) {
                return Some(error);
            }
        }
    }
    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
        let branch_errors = branches
            .iter()
            .map(|branch| tool_input_schema_error(branch, value, path))
            .collect::<Vec<_>>();
        if branch_errors.iter().all(Option::is_some) {
            let reasons = branch_errors
                .into_iter()
                .enumerate()
                .filter_map(|(index, error)| {
                    error.map(|error| format!("option {}: {error}", index + 1))
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Some(format!(
                "{path} does not match any allowed input shape ({reasons})"
            ));
        }
    }
    if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
        let branch_errors = branches
            .iter()
            .map(|branch| tool_input_schema_error(branch, value, path))
            .collect::<Vec<_>>();
        let matches = branch_errors.iter().filter(|error| error.is_none()).count();
        if matches != 1 {
            if matches == 0 {
                let reasons = branch_errors
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, error)| {
                        error.map(|error| format!("option {}: {error}", index + 1))
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                return Some(format!(
                    "{path} must match exactly one allowed input shape ({reasons})"
                ));
            }
            return Some(format!(
                "{path} must match exactly one allowed input shape ({matches} shapes matched)"
            ));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Some(format!("{path} is not one of the allowed values"));
        }
    }
    if let Some(constant) = schema.get("const") {
        if constant != value {
            return Some(format!("{path} does not match the required constant"));
        }
    }
    if let Some(types) = schema.get("type") {
        let matches = match types {
            Value::String(kind) => json_value_matches_schema_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| json_value_matches_schema_type(value, kind)),
            _ => true,
        };
        if !matches {
            let expected = match types {
                Value::String(kind) => kind.clone(),
                Value::Array(kinds) => kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" or "),
                _ => "the advertised JSON type".to_string(),
            };
            return Some(format!("{path} must be {expected}"));
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
            if number < minimum {
                return Some(format!("{path} must be at least {minimum}"));
            }
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
            if number > maximum {
                return Some(format!("{path} must be at most {maximum}"));
            }
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64) {
            if length < minimum {
                return Some(format!("{path} must contain at least {minimum} characters"));
            }
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64) {
            if length > maximum {
                return Some(format!("{path} must contain at most {maximum} characters"));
            }
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Some(format!("{path}.{key} is required"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, item) in object {
                if let Some(property_schema) = properties.get(key) {
                    if let Some(error) =
                        tool_input_schema_error(property_schema, item, &format!("{path}.{key}"))
                    {
                        return Some(error);
                    }
                } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                    return Some(format!("{path}.{key} is not allowed"));
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64) {
            if (values.len() as u64) < minimum {
                return Some(format!("{path} must contain at least {minimum} items"));
            }
        }
        if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64) {
            if (values.len() as u64) > maximum {
                return Some(format!("{path} must contain at most {maximum} items"));
            }
        }
        for (index, item) in values.iter().enumerate() {
            if let Some(error) = tool_input_schema_error(items, item, &format!("{path}[{index}]")) {
                return Some(error);
            }
        }
    }
    None
}
