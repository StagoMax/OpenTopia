use super::{
    nonredundant_tool_result_content, resource_fallback_text, ModelConversationRole,
    ModelInputContent, ModelRequest, ProviderToolCall, ProviderToolCandidate,
    ProviderToolDisclosure, ProviderToolResult,
};
use crate::model_context::{
    estimate_tokens, ContextCacheScope, TokenEstimateBreakdown, TokenEstimateDetail,
};
use crate::token_breakdown::{merge_sibling_details, reconcile_detail_children};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderItemBucket {
    LoadedToolSchema,
    ToolCall,
    TurnAssistantState,
    ProviderState,
}

impl ModelRequest {
    /// Estimates the provider-neutral logical input by harness module.
    /// Provider adapters add their own framing, so actual response usage remains
    /// authoritative and is used to report the estimate error.
    pub fn token_estimate_breakdown(&self) -> TokenEstimateBreakdown {
        let mut breakdown = if self.provider_transcript.is_some() {
            // The retained wire transcript already owns stable/thread
            // instructions. Count only the newly appended volatile context to
            // avoid charging the same prefix twice.
            let volatile_items = self
                .instructions
                .items
                .iter()
                .filter(|item| {
                    matches!(
                        item.cache_scope,
                        ContextCacheScope::Turn | ContextCacheScope::Round
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            TokenEstimateBreakdown::from_context_items(&volatile_items)
        } else {
            TokenEstimateBreakdown::from_context_items(&self.instructions.items)
        };

        let conversation_children = conversation_details(self);
        breakdown.conversation = conversation_children.iter().map(|child| child.tokens).sum();
        breakdown.set_detail_root(
            "conversation",
            "Conversation history",
            breakdown.conversation,
            conversation_children,
        );

        let current_user_children = current_user_details(self);
        breakdown.current_user = current_user_children.iter().map(|child| child.tokens).sum();
        breakdown.set_detail_root(
            "current_user",
            "Current user input",
            breakdown.current_user,
            current_user_children,
        );

        let native_tool_call_items =
            provider_items_for_bucket(&self.previous_response_items, ProviderItemBucket::ToolCall);
        let (tool_call_tokens, tool_call_children) = if native_tool_call_items.is_empty() {
            typed_tool_call_details(&self.input.tool_calls)
        } else {
            // Responses sends these native items instead of the provider-neutral
            // `input.tool_calls` ledger. Attribute the physical representation
            // once; keeping both was the source of the old double count.
            provider_item_details(&native_tool_call_items)
        };
        breakdown.tool_calls = tool_call_tokens;
        breakdown.set_detail_root(
            "tool_calls",
            "Tool calls",
            breakdown.tool_calls,
            tool_call_children,
        );

        // Results are always appended after the selected call representation.
        // Re-estimate from typed values so image byte buffers are not counted as
        // enormous JSON integer arrays.
        let (tool_result_tokens, tool_result_children) =
            named_tool_result_details(&self.input.tool_results);
        breakdown.tool_results = tool_result_tokens;
        breakdown.set_detail_root(
            "tool_results",
            "Tool results",
            breakdown.tool_results,
            tool_result_children,
        );

        let (direct_tokens, direct_children) = direct_tool_details(&self.tool_candidates);
        let (deferred_tokens, deferred_children) = deferred_tool_details(&self.tool_candidates);
        let loaded_items = provider_items_for_bucket(
            &self.previous_response_items,
            ProviderItemBucket::LoadedToolSchema,
        );
        let (loaded_tokens, loaded_children) = loaded_tool_details(&loaded_items);
        breakdown.direct_tool_schemas = direct_tokens;
        breakdown.deferred_tool_catalog = deferred_tokens;
        breakdown.loaded_tool_schemas = loaded_tokens;
        breakdown.tool_schemas = direct_tokens
            .saturating_add(deferred_tokens)
            .saturating_add(loaded_tokens);
        let tool_surface_children = [
            TokenEstimateDetail::branch(
                "direct_tool_schemas",
                "Direct tool schemas",
                direct_tokens,
                direct_children,
            ),
            TokenEstimateDetail::branch(
                "deferred_tool_catalog",
                "Deferred tool catalog",
                deferred_tokens,
                deferred_children,
            ),
            TokenEstimateDetail::branch(
                "loaded_tool_schemas",
                "Loaded tool schemas",
                loaded_tokens,
                loaded_children,
            ),
        ]
        .into_iter()
        .filter(|detail| detail.tokens > 0)
        .collect();
        breakdown.set_detail_root(
            "tool_schemas",
            "Tool surface",
            breakdown.tool_schemas,
            tool_surface_children,
        );

        breakdown.output_schema = self
            .final_output_json_schema
            .as_ref()
            .map(estimate_serialized_tokens)
            .unwrap_or_default();
        let output_children = (breakdown.output_schema > 0)
            .then(|| {
                vec![TokenEstimateDetail::leaf(
                    "structured_output_schema",
                    "Structured output schema",
                    breakdown.output_schema,
                )]
            })
            .unwrap_or_default();
        breakdown.set_detail_root(
            "output_schema",
            "Output schema",
            breakdown.output_schema,
            output_children,
        );

        let turn_assistant_items = provider_items_for_bucket(
            &self.previous_response_items,
            ProviderItemBucket::TurnAssistantState,
        );
        let (turn_assistant_tokens, turn_assistant_children) =
            provider_item_details(&turn_assistant_items);
        breakdown.turn_assistant_state = turn_assistant_tokens;
        breakdown.set_detail_root(
            "turn_assistant_state",
            "Current-turn assistant state",
            breakdown.turn_assistant_state,
            turn_assistant_children,
        );

        let provider_state_items = provider_items_for_bucket(
            &self.previous_response_items,
            ProviderItemBucket::ProviderState,
        );
        let (provider_state_tokens, provider_state_children) =
            provider_item_details(&provider_state_items);
        breakdown.provider_state = provider_state_tokens;
        breakdown.set_detail_root(
            "provider_state",
            "Opaque provider continuation state",
            breakdown.provider_state,
            provider_state_children,
        );
        breakdown.recalculate_total();
        breakdown
    }
}

fn conversation_details(request: &ModelRequest) -> Vec<TokenEstimateDetail> {
    if let Some(transcript) = request.provider_transcript.as_ref() {
        return vec![TokenEstimateDetail::leaf(
            "provider_wire_transcript",
            "Retained provider transcript",
            estimate_serialized_tokens(&transcript.items),
        )];
    }
    merge_sibling_details(
        request
            .input
            .conversation
            .iter()
            .map(|message| {
                let (role_id, role_label) = match message.role {
                    ModelConversationRole::System => ("system_messages", "System messages"),
                    ModelConversationRole::User => ("user_messages", "User messages"),
                    ModelConversationRole::Assistant => {
                        ("assistant_messages", "Assistant messages")
                    }
                    ModelConversationRole::Tool => ("tool_messages", "Tool messages"),
                };
                let mut children = Vec::new();
                push_nonzero_leaf(
                    &mut children,
                    "message_text",
                    "Message text",
                    estimate_tokens(&message.content),
                );
                push_nonzero_branch(
                    &mut children,
                    "typed_content",
                    "Typed content",
                    content_details(&message.content_parts),
                );
                let (call_tokens, call_children) = typed_tool_call_details(&message.tool_calls);
                push_nonzero_branch_with_total(
                    &mut children,
                    "historical_tool_calls",
                    "Historical tool calls",
                    call_tokens,
                    call_children,
                );
                let (result_tokens, result_children) =
                    named_tool_result_details(&message.tool_results);
                push_nonzero_branch_with_total(
                    &mut children,
                    "historical_tool_results",
                    "Historical tool results",
                    result_tokens,
                    result_children,
                );
                let tokens = children.iter().map(|child| child.tokens).sum();
                TokenEstimateDetail::branch(role_id, role_label, tokens, children)
            })
            .collect(),
    )
}

fn current_user_details(request: &ModelRequest) -> Vec<TokenEstimateDetail> {
    let mut children = Vec::new();
    push_nonzero_leaf(
        &mut children,
        "message_text",
        "Message text",
        estimate_tokens(&request.input.current_user.message),
    );
    push_nonzero_branch(
        &mut children,
        "typed_content",
        "Typed content",
        content_details(&request.input.current_user.content),
    );
    children
}

fn typed_tool_call_details(calls: &[ProviderToolCall]) -> (usize, Vec<TokenEstimateDetail>) {
    let children = merge_sibling_details(
        calls
            .iter()
            .map(|call| {
                let total = estimate_serialized_tokens(call);
                let children = complete_detail_children(
                    total,
                    vec![
                        TokenEstimateDetail::leaf(
                            "call_identity",
                            "Call ID and tool name",
                            estimate_tokens(&call.id).saturating_add(estimate_tokens(&call.name)),
                        ),
                        TokenEstimateDetail::leaf(
                            "call_arguments",
                            "Arguments",
                            estimate_serialized_tokens(&call.arguments),
                        ),
                    ],
                );
                TokenEstimateDetail::branch(&call.name, &call.name, total, children)
            })
            .collect(),
    );
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn named_tool_result_details(results: &[ProviderToolResult]) -> (usize, Vec<TokenEstimateDetail>) {
    let children = merge_sibling_details(
        results
            .iter()
            .map(|result| {
                let total = estimate_provider_tool_results(std::slice::from_ref(result));
                let typed_content = content_details(&nonredundant_tool_result_content(result));
                let mut children = vec![
                    TokenEstimateDetail::leaf(
                        "result_output",
                        "Output text",
                        estimate_tokens(&result.output),
                    ),
                    TokenEstimateDetail::leaf(
                        "result_metadata",
                        "Metadata and protocol fields",
                        estimate_tokens(&result.name)
                            .saturating_add(estimate_serialized_tokens(&result.metadata))
                            .saturating_add(32),
                    ),
                ];
                push_nonzero_branch(
                    &mut children,
                    "typed_content",
                    "Typed content",
                    typed_content,
                );
                let children = complete_detail_children(total, children);
                TokenEstimateDetail::branch(&result.name, &result.name, total, children)
            })
            .collect(),
    );
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn content_details(parts: &[ModelInputContent]) -> Vec<TokenEstimateDetail> {
    let mut totals = BTreeMap::<&'static str, usize>::new();
    for part in parts {
        let (id, tokens) = match part {
            ModelInputContent::Text { text } => ("content_text", estimate_tokens(text)),
            ModelInputContent::Json { value } => {
                ("content_json", estimate_tokens(&value.to_string()))
            }
            ModelInputContent::Image { data, .. } => {
                ("content_image", (data.len() / 16).max(1_024))
            }
            ModelInputContent::Resource {
                uri,
                content_type,
                name,
            } => (
                "content_resource",
                estimate_tokens(&resource_fallback_text(
                    uri,
                    content_type.as_deref(),
                    name.as_deref(),
                )),
            ),
        };
        let total = totals.entry(id).or_default();
        *total = total.saturating_add(tokens);
    }
    totals
        .into_iter()
        .map(|(id, tokens)| TokenEstimateDetail::leaf(id, id, tokens))
        .collect()
}

fn direct_tool_details(candidates: &[ProviderToolCandidate]) -> (usize, Vec<TokenEstimateDetail>) {
    let children = candidates
        .iter()
        .filter(|candidate| candidate.disclosure == ProviderToolDisclosure::Direct)
        .map(|candidate| {
            let tokens = estimate_serialized_tokens(&json!({
                "name": candidate.name,
                "description": candidate.description,
                "parameters": candidate.input_schema,
            }));
            TokenEstimateDetail::leaf(&candidate.name, &candidate.name, tokens)
        })
        .collect::<Vec<_>>();
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn deferred_tool_details(
    candidates: &[ProviderToolCandidate],
) -> (usize, Vec<TokenEstimateDetail>) {
    let mut children = candidates
        .iter()
        .filter(|candidate| candidate.disclosure == ProviderToolDisclosure::DeferredIndividual)
        .map(|candidate| {
            let tokens = estimate_serialized_tokens(&json!({
                "name": candidate.name,
                "description": candidate.description,
            }));
            TokenEstimateDetail::leaf(&candidate.name, &candidate.name, tokens)
        })
        .collect::<Vec<_>>();
    let namespaces = candidates
        .iter()
        .filter(|candidate| candidate.disclosure == ProviderToolDisclosure::DeferredNamespace)
        .filter_map(|candidate| candidate.namespace.as_ref())
        .map(|namespace| (namespace.name.as_str(), namespace.description.as_str()))
        .collect::<BTreeMap<_, _>>();
    children.extend(namespaces.into_iter().map(|(name, description)| {
        let tokens = estimate_serialized_tokens(&json!({
            "name": name,
            "description": description,
        }));
        TokenEstimateDetail::leaf(format!("namespace:{name}"), name, tokens)
    }));
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn provider_items_for_bucket(items: &[Value], bucket: ProviderItemBucket) -> Vec<&Value> {
    items
        .iter()
        .filter(|item| provider_item_bucket(item) == bucket)
        .collect()
}

fn provider_item_bucket(item: &Value) -> ProviderItemBucket {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if item_type == "tool_search_output" {
        ProviderItemBucket::LoadedToolSchema
    } else if item_type == "message" {
        ProviderItemBucket::TurnAssistantState
    } else if provider_item_is_tool_call(item_type) {
        ProviderItemBucket::ToolCall
    } else {
        // Reasoning, compaction, Chat assistant-state, and unknown future
        // provider items stay explicit instead of being mislabeled as history.
        ProviderItemBucket::ProviderState
    }
}

fn provider_item_is_tool_call(item_type: &str) -> bool {
    matches!(
        item_type,
        "function_call" | "custom_tool_call" | "apply_patch_call"
    ) || item_type.ends_with("_call")
}

fn provider_item_details(items: &[&Value]) -> (usize, Vec<TokenEstimateDetail>) {
    let children = merge_sibling_details(
        items
            .iter()
            .map(|item| {
                let item_type = item
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("provider_item");
                let label = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(item_type);
                let id = if provider_item_is_tool_call(item_type) {
                    label
                } else {
                    item_type
                };
                TokenEstimateDetail::leaf(id, label, estimate_serialized_tokens(item))
            })
            .collect(),
    );
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn loaded_tool_details(items: &[&Value]) -> (usize, Vec<TokenEstimateDetail>) {
    let children = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let label = item
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Tool Search output {}", index + 1));
            TokenEstimateDetail::leaf(
                format!("tool_search_output:{index}"),
                label,
                estimate_serialized_tokens(item),
            )
        })
        .collect::<Vec<_>>();
    (children.iter().map(|child| child.tokens).sum(), children)
}

fn push_nonzero_leaf(
    children: &mut Vec<TokenEstimateDetail>,
    id: &'static str,
    label: &'static str,
    tokens: usize,
) {
    if tokens > 0 {
        children.push(TokenEstimateDetail::leaf(id, label, tokens));
    }
}

fn push_nonzero_branch(
    children: &mut Vec<TokenEstimateDetail>,
    id: &'static str,
    label: &'static str,
    branch_children: Vec<TokenEstimateDetail>,
) {
    let tokens = branch_children.iter().map(|child| child.tokens).sum();
    push_nonzero_branch_with_total(children, id, label, tokens, branch_children);
}

fn push_nonzero_branch_with_total(
    children: &mut Vec<TokenEstimateDetail>,
    id: &'static str,
    label: &'static str,
    tokens: usize,
    branch_children: Vec<TokenEstimateDetail>,
) {
    if tokens > 0 {
        children.push(TokenEstimateDetail::branch(
            id,
            label,
            tokens,
            branch_children,
        ));
    }
}

fn complete_detail_children(
    target: usize,
    mut children: Vec<TokenEstimateDetail>,
) -> Vec<TokenEstimateDetail> {
    children.retain(|child| child.tokens > 0);
    let attributed = children.iter().map(|child| child.tokens).sum::<usize>();
    if attributed <= target {
        let framing = target - attributed;
        if framing > 0 {
            children.push(TokenEstimateDetail::leaf(
                "protocol_framing",
                "Protocol framing",
                framing,
            ));
        }
        children
    } else {
        reconcile_detail_children(target, children)
    }
}

fn estimate_serialized_tokens(value: &impl Serialize) -> usize {
    serde_json::to_string(value)
        .map(|value| estimate_tokens(&value))
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn estimate_serialized_slice(value: &[impl Serialize]) -> usize {
    if value.is_empty() {
        0
    } else {
        estimate_serialized_tokens(&value)
    }
}

pub(crate) fn estimate_provider_tool_results(results: &[ProviderToolResult]) -> usize {
    results
        .iter()
        .map(|result| {
            estimate_tokens(&result.name)
                .saturating_add(estimate_tokens(&result.output))
                .saturating_add(
                    content_details(&nonredundant_tool_result_content(result))
                        .iter()
                        .map(|detail| detail.tokens)
                        .sum::<usize>(),
                )
                .saturating_add(estimate_serialized_tokens(&result.metadata))
                .saturating_add(32)
        })
        .sum()
}

fn estimate_tool_surface(candidates: &[ProviderToolCandidate]) -> (usize, usize) {
    let (direct, _) = direct_tool_details(candidates);
    let (deferred, _) = deferred_tool_details(candidates);
    (direct, deferred)
}

pub fn estimate_provider_tool_surface_tokens(candidates: &[ProviderToolCandidate]) -> usize {
    let (direct, deferred) = estimate_tool_surface(candidates);
    direct.saturating_add(deferred)
}
