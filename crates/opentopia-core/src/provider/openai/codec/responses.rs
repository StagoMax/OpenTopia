use super::chat::{legacy_tool_observation, openai_conversation_role, CompiledProviderTools};
use super::shared::responses_tool_result_output;
use super::tool_schema::compile_openai_function_candidate;
use crate::model_context::{ContextCacheScope, ContextRole};
use crate::provider::{
    encode_base64, provider_item_is_internal_transcript, provider_wire_transcript,
    resource_fallback_text, CompiledToolContract, ModelConversationMessage, ModelConversationRole,
    ModelInputContent, ModelRequest, PromptCacheBreakpointPolicy, ProviderToolCandidate,
    ProviderToolDefinition, ProviderToolDisclosure, ProviderToolResult,
};
use crate::settings::{ProviderFeatureSupport, ProviderToolProtocolCapabilities};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub(in crate::provider) const OPENAI_RESPONSES_REQUEST_TRANSCRIPT_FORMAT: &str =
    "openai_responses_request_input_v1";
pub(in crate::provider) const OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT: &str =
    "openai_responses_completed_input_v1";

pub(in crate::provider) struct CompiledResponseToolDefinitions {
    definitions: Vec<ProviderToolDefinition>,
    contracts: Vec<CompiledToolContract>,
}

pub(in crate::provider) fn compile_responses_tool_definitions(
    candidates: &[ProviderToolCandidate],
    capabilities: ProviderToolProtocolCapabilities,
) -> CompiledResponseToolDefinitions {
    let mut definitions = Vec::with_capacity(candidates.len());
    let mut contracts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let is_apply_patch = candidate.name == "apply_patch";
        // The local executor advertises native operation support through
        // its schema. This second gate prevents an endpoint capability
        // from selecting a wire format the Harness cannot yet execute.
        let accepts_native_operation = candidate
            .input_schema
            .pointer("/properties/operation")
            .is_some();
        let accepts_freeform_patch = candidate
            .input_schema
            .pointer("/properties/patch/type")
            .and_then(Value::as_str)
            == Some("string");

        if is_apply_patch
            && accepts_native_operation
            && capabilities.hosted_apply_patch == ProviderFeatureSupport::Supported
        {
            definitions.push(ProviderToolDefinition::Hosted {
                kind: "apply_patch".to_string(),
            });
        } else if is_apply_patch
            && accepts_freeform_patch
            && capabilities.freeform_tools == ProviderFeatureSupport::Supported
        {
            definitions.push(ProviderToolDefinition::Freeform {
                name: candidate.name.clone(),
                description: format!(
                    "{} Return only the raw unified diff accepted by this tool; do not wrap it in JSON or Markdown.",
                    candidate.description
                ),
            });
        } else {
            let compiled = compile_openai_function_candidate(candidate, capabilities);
            let candidate = compiled.candidate;
            let input_schema = compiled.contract.wire_input_schema.clone();
            contracts.push(compiled.contract);
            definitions.push(ProviderToolDefinition::Function {
                name: candidate.name,
                description: candidate.description,
                input_schema,
                strict: compiled.strict,
            });
        }
    }
    CompiledResponseToolDefinitions {
        definitions,
        contracts,
    }
}

#[cfg(test)]
pub(in crate::provider) fn responses_tools(
    candidates: &[ProviderToolCandidate],
    capabilities: ProviderToolProtocolCapabilities,
) -> Vec<Value> {
    compile_responses_tools(candidates, capabilities).tools
}

pub(in crate::provider) fn compile_responses_tools(
    candidates: &[ProviderToolCandidate],
    capabilities: ProviderToolProtocolCapabilities,
) -> CompiledProviderTools {
    fn lower_definition(definition: ProviderToolDefinition) -> Value {
        match definition {
            ProviderToolDefinition::Function {
                name,
                description,
                input_schema,
                strict,
            } => json!({
                "type": "function",
                "name": name,
                "description": description,
                "parameters": input_schema,
                "strict": strict,
            }),
            ProviderToolDefinition::Freeform { name, description } => json!({
                "type": "custom",
                "name": name,
                "description": description,
            }),
            ProviderToolDefinition::Hosted { kind } => json!({ "type": kind }),
        }
    }

    let compiled = compile_responses_tool_definitions(candidates, capabilities);
    let mut tools = Vec::new();
    let mut namespaces: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    let native_deferred = capabilities.deferred_tool_loading == ProviderFeatureSupport::Supported
        && capabilities.hosted_tool_search == ProviderFeatureSupport::Supported;

    for (candidate, definition) in candidates.iter().zip(compiled.definitions) {
        let mut tool = lower_definition(definition);
        match candidate.disclosure {
            ProviderToolDisclosure::Direct if native_deferred => tools.push(tool),
            ProviderToolDisclosure::DeferredIndividual if native_deferred => {
                tool["defer_loading"] = json!(true);
                tools.push(tool);
            }
            ProviderToolDisclosure::DeferredNamespace
                if native_deferred
                    && capabilities.namespace_tools == ProviderFeatureSupport::Supported =>
            {
                tool["defer_loading"] = json!(true);
                if let Some(namespace) = candidate.namespace.as_ref() {
                    namespaces
                        .entry((namespace.name.clone(), namespace.description.clone()))
                        .or_default()
                        .push(tool);
                } else {
                    tools.push(tool);
                }
            }
            _ => tools.push(tool),
        }
    }

    let has_deferred = candidates
        .iter()
        .any(|candidate| candidate.disclosure != ProviderToolDisclosure::Direct);
    for ((name, description), namespace_tools) in namespaces {
        tools.push(json!({
            "type": "namespace",
            "name": name,
            "description": description,
            "tools": namespace_tools,
        }));
    }
    if native_deferred && has_deferred {
        tools.push(json!({ "type": "tool_search" }));
    }
    CompiledProviderTools {
        tools,
        contracts: compiled.contracts,
    }
}

pub(in crate::provider) fn responses_input(request: &ModelRequest) -> Vec<Value> {
    let replay_full_prefix = request.previous_response_id.is_none();
    let cached_transcript = replay_full_prefix
        .then_some(request.provider_transcript.as_ref())
        .flatten()
        .filter(|transcript| {
            matches!(
                transcript.format.as_str(),
                OPENAI_RESPONSES_REQUEST_TRANSCRIPT_FORMAT
                    | OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT
            ) && !transcript.items.is_empty()
        });
    let mut input = cached_transcript
        .map(|transcript| transcript.items.clone())
        .unwrap_or_default();
    if replay_full_prefix && cached_transcript.is_none() {
        input.extend(responses_scoped_instruction_input(request, true));
        input.extend(
            request
                .input
                .conversation
                .iter()
                .enumerate()
                .flat_map(|(index, message)| responses_conversation_items(message, index)),
        );
    }

    // Only a completed provider response with live calls continues the same
    // model turn. A request checkpoint represents an interrupted/failed prior
    // request; the next turn must keep it as the exact prefix and append the
    // new user message after it.
    let continues_active_turn = cached_transcript.is_some_and(|transcript| {
        transcript.format == OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT
            && !request.input.tool_calls.is_empty()
    });
    if !continues_active_turn {
        input.push(json!({
            "role": "user",
            "content": responses_message_content(
                ModelConversationRole::User,
                &request.input.current_user.message,
                &request.input.current_user.content,
            ),
        }));
    }

    // Volatile turn/round context is emitted for every model round. With an
    // exact transcript it extends the prior request instead of being rebuilt
    // inside it, so updated runtime guidance remains visible without splitting
    // the cached prefix.
    input.extend(responses_scoped_instruction_input(request, false));

    // A transcript candidate already embeds every provider item that preceded
    // it. Runtime-owned observations may have been appended afterwards, while
    // a durable cursor can retain encrypted reasoning alongside the transcript.
    // Extend only with items that are not already present so replay stays
    // byte-for-byte append-only.
    for item in request
        .previous_response_items
        .iter()
        .filter(|item| !provider_item_is_internal_transcript(item))
    {
        if !input.iter().any(|existing| existing == item) {
            input.push(item.clone());
        }
    }

    // Provider output items normally own the exact native call representation.
    // Runtime observations and legacy recovery state may only have the logical
    // call ledger, so append a function call when no native item represents it.
    for call in &request.input.tool_calls {
        if responses_tool_call_kind(&input, &call.id).is_none() {
            input.push(json!({
                "type": "function_call",
                "call_id": &call.id,
                "name": &call.name,
                "arguments": call.arguments.to_string(),
            }));
        }
    }

    let mut emitted_results = input
        .iter()
        .filter_map(responses_tool_output_call_id)
        .map(str::to_string)
        .collect::<HashSet<_>>();
    for result in &request.input.tool_results {
        if emitted_results.insert(result.call_id.clone()) {
            let item = responses_tool_result_item(&input, result);
            input.push(item);
        }
    }
    input
}

fn responses_tool_output_call_id(item: &Value) -> Option<&str> {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("function_call_output" | "custom_tool_call_output" | "apply_patch_call_output")
    )
    .then(|| item.get("call_id").and_then(Value::as_str))
    .flatten()
}

fn responses_tool_result_item(items: &[Value], result: &ProviderToolResult) -> Value {
    let output = responses_tool_result_output(result);
    match responses_tool_call_kind(items, &result.call_id) {
        Some(ResponsesToolCallKind::ApplyPatch) => json!({
            "type": "apply_patch_call_output",
            "call_id": &result.call_id,
            "status": if result.is_error { "failed" } else { "completed" },
            "output": output,
        }),
        Some(ResponsesToolCallKind::Custom) => json!({
            "type": "custom_tool_call_output",
            "call_id": &result.call_id,
            "output": output,
        }),
        Some(ResponsesToolCallKind::Function) | None => json!({
            "type": "function_call_output",
            "call_id": &result.call_id,
            "output": output,
        }),
    }
}

pub(in crate::provider) fn responses_conversation_items(
    message: &ModelConversationMessage,
    index: usize,
) -> Vec<Value> {
    if message.role == ModelConversationRole::Tool
        && message.tool_calls.is_empty()
        && message.tool_results.is_empty()
    {
        let (call, result) = legacy_tool_observation(message, index);
        return vec![
            json!({
                "type": "function_call",
                "call_id": &call.id,
                "name": &call.name,
                "arguments": call.arguments.to_string(),
            }),
            json!({
                "type": "function_call_output",
                "call_id": &result.call_id,
                "output": responses_tool_result_output(&result),
            }),
        ];
    }
    if !message.tool_calls.is_empty() {
        let mut items = message
            .tool_calls
            .iter()
            .map(|call| {
                json!({
                    "type": "function_call",
                    "call_id": &call.id,
                    "name": &call.name,
                    "arguments": call.arguments.to_string(),
                })
            })
            .collect::<Vec<_>>();
        items.extend(message.tool_results.iter().map(|result| {
            json!({
                "type": "function_call_output",
                "call_id": &result.call_id,
                "output": responses_tool_result_output(result),
            })
        }));
        return items;
    }
    if !message.tool_results.is_empty() {
        return message
            .tool_results
            .iter()
            .map(|result| {
                json!({
                    "type": "function_call_output",
                    "call_id": &result.call_id,
                    "output": responses_tool_result_output(result),
                })
            })
            .collect();
    }
    vec![json!({
        "role": openai_conversation_role(message.role),
        "content": responses_message_content(
            message.role,
            &message.content,
            &message.content_parts,
        ),
    })]
}

pub(in crate::provider) fn responses_scoped_instruction_input(
    request: &ModelRequest,
    lineage_prefix: bool,
) -> Vec<Value> {
    if request.instructions.items.is_empty() {
        return Vec::new();
    }
    request
        .instructions
        .instruction_messages_with_scope()
        .into_iter()
        .filter_map(|(role, scope, content)| {
            let belongs_to_prefix =
                matches!(scope, ContextCacheScope::Stable | ContextCacheScope::Thread);
            if belongs_to_prefix != lineage_prefix {
                return None;
            }
            let role = match role {
                ContextRole::Developer => "developer",
                ContextRole::User => "user",
                // Stable system instructions live in the top-level `instructions`
                // field. A volatile system item must instead remain behind the
                // current user cache anchor, alongside volatile developer context.
                ContextRole::System if !lineage_prefix => "system",
                _ => return None,
            };
            Some(json!({
                "role": role,
                "content": content,
            }))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::provider) enum ResponsesToolCallKind {
    Function,
    Custom,
    ApplyPatch,
}

pub(in crate::provider) fn responses_tool_call_kind(
    items: &[Value],
    call_id: &str,
) -> Option<ResponsesToolCallKind> {
    items.iter().find_map(|item| {
        if let Some(transcript) = provider_wire_transcript(item) {
            if let Some(kind) = responses_tool_call_kind(&transcript.items, call_id) {
                return Some(kind);
            }
        }
        let matches_call = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            == Some(call_id);
        if !matches_call {
            return None;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") => Some(ResponsesToolCallKind::Function),
            Some("custom_tool_call") => Some(ResponsesToolCallKind::Custom),
            Some("apply_patch_call") => Some(ResponsesToolCallKind::ApplyPatch),
            _ => None,
        }
    })
}

pub(in crate::provider) fn add_responses_prompt_cache_breakpoint(
    input: &mut Value,
    request: &ModelRequest,
) {
    if request.instructions.items.is_empty() {
        return;
    }

    let Some(items) = input.as_array_mut() else {
        return;
    };
    let lineage_developer_count = if request.previous_response_id.is_none() {
        responses_scoped_instruction_input(request, true).len()
    } else {
        0
    };
    if request.previous_response_id.is_none() {
        if let Some(index) = lineage_developer_count.checked_sub(1) {
            if let Some(message) = items.get_mut(index) {
                mark_responses_message_cache_breakpoint(message);
            }
        }
    }

    if request.prompt_cache_breakpoint_policy == PromptCacheBreakpointPolicy::StableOnly {
        return;
    }

    for message in items
        .iter_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    {
        mark_responses_message_cache_breakpoint(message);
    }
}

pub(in crate::provider) fn mark_responses_message_cache_breakpoint(message: &mut Value) {
    let Some(content) = message.get_mut("content") else {
        return;
    };
    if let Some(text) = content.as_str().map(str::to_string) {
        *content = json!([{
            "type": "input_text",
            "text": text,
            "prompt_cache_breakpoint": { "mode": "explicit" },
        }]);
        return;
    }
    let Some(parts) = content.as_array_mut() else {
        return;
    };
    if let Some(part) = parts.iter_mut().rev().find(|part| {
        matches!(
            part.get("type").and_then(Value::as_str),
            Some("input_text") | Some("input_image") | Some("input_file")
        )
    }) {
        part["prompt_cache_breakpoint"] = json!({ "mode": "explicit" });
    }
}

pub(in crate::provider) fn responses_message_content(
    role: ModelConversationRole,
    legacy_text: &str,
    parts: &[ModelInputContent],
) -> Value {
    if parts.is_empty() {
        return Value::String(legacy_text.to_string());
    }
    let text_type = if role == ModelConversationRole::Assistant {
        "output_text"
    } else {
        "input_text"
    };
    let mut content = Vec::new();
    if !legacy_text.is_empty() {
        content.push(json!({ "type": text_type, "text": legacy_text }));
    }
    content.extend(parts.iter().map(|part| match part {
        ModelInputContent::Text { text } => json!({ "type": text_type, "text": text }),
        ModelInputContent::Json { value } => {
            json!({ "type": text_type, "text": value.to_string() })
        }
        ModelInputContent::Image { content_type, data } => json!({
            "type": "input_image",
            "image_url": format!("data:{content_type};base64,{}", encode_base64(data)),
        }),
        ModelInputContent::Resource {
            uri,
            content_type,
            name,
        } => json!({
            "type": text_type,
            "text": resource_fallback_text(uri, content_type.as_deref(), name.as_deref()),
        }),
    }));
    Value::Array(content)
}
