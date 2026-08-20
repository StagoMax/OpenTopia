use super::{
    provider_compatibility_hash, AgentCore, AgentEventPayload, CompiledModelContext, ContextBudget,
    ModelContentPart, ModelConversationMessage, ModelRequest, ProviderToolCall,
    ProviderToolCandidate, ProviderToolResult, TurnEvents, TurnRuntimeState, Uuid, Value,
};
use crate::context_runtime::CanonicalModelRequest;
use crate::round_compaction::{
    context_compact_threshold_percent, RoundContextCompactionRequest, RoundContextCompactionResult,
};

impl AgentCore {
    /// Builds and admits one provider request. Every round, including the first
    /// round of a new turn, crosses this single pressure boundary.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn admitted_round_request(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        round: usize,
        model_context: &CompiledModelContext,
        round_model_context: &CompiledModelContext,
        context_summary: &mut Option<String>,
        conversation: &mut Vec<ModelConversationMessage>,
        budget: &mut Option<ContextBudget>,
        runtime_state: &mut TurnRuntimeState,
        model_user_message: &str,
        model_user_content: &[ModelContentPart],
        tool_candidates: &[ProviderToolCandidate],
        provider_tool_calls: &mut Vec<ProviderToolCall>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        compacted_tool_history: &mut String,
        provider_response_items: &mut Vec<Value>,
        previous_response_id: Option<String>,
        branch_developer_instructions: Option<&str>,
        compatibility_hash: &mut String,
        events: &mut TurnEvents,
    ) -> anyhow::Result<CanonicalModelRequest> {
        let admitted_model_context = refreshed_round_model_context(
            round_model_context,
            context_summary.as_deref(),
            tool_candidates,
        );
        let pressure_request = self.assemble_model_request(
            &admitted_model_context,
            context_summary.as_deref(),
            conversation.clone(),
            model_user_message.to_string(),
            model_user_content.to_vec(),
            tool_candidates.to_vec(),
            provider_tool_calls.clone(),
            provider_tool_results.clone(),
            provider_response_items.clone(),
            previous_response_id.clone(),
            branch_developer_instructions.map(str::to_string),
        )?;
        super::synchronize_context_budget(budget, pressure_request.logical());
        let before_tokens = pressure_request.logical().token_estimate_breakdown().total;
        let compacted = self
            .compact_context_for_round(
                thread_id,
                user_message_id,
                round,
                context_summary,
                conversation,
                budget,
                runtime_state,
                model_context,
                tool_candidates,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                compacted_tool_history,
                pressure_request.logical(),
                branch_developer_instructions,
                compatibility_hash,
                events,
            )
            .await;
        if let Some((compacted, dropped_tool_results)) = compacted {
            let rebuilt = self.rebuild_request_after_durable_checkpoint(
                round_model_context,
                context_summary,
                conversation,
                budget,
                model_user_message,
                model_user_content,
                tool_candidates,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                branch_developer_instructions,
            )?;
            self.record_round_context_compaction(
                round,
                compacted,
                dropped_tool_results,
                before_tokens,
                rebuilt.logical().token_estimate_breakdown().total,
                events,
            );
            return Ok(rebuilt);
        }

        Ok(pressure_request)
    }

    /// Provider token accounting remains authoritative. If it rejects an
    /// admitted request, both Round 0 and later rounds use this same durable
    /// recovery path and retry at most once.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn request_after_context_overflow(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        round: usize,
        model_context: &CompiledModelContext,
        round_model_context: &CompiledModelContext,
        context_summary: &mut Option<String>,
        conversation: &mut Vec<ModelConversationMessage>,
        budget: &mut Option<ContextBudget>,
        runtime_state: &mut TurnRuntimeState,
        model_user_message: &str,
        model_user_content: &[ModelContentPart],
        tool_candidates: &[ProviderToolCandidate],
        provider_tool_calls: &mut Vec<ProviderToolCall>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        compacted_tool_history: &mut String,
        provider_response_items: &mut Vec<Value>,
        rejected_request: &ModelRequest,
        branch_developer_instructions: Option<&str>,
        compatibility_hash: &mut String,
        events: &mut TurnEvents,
    ) -> anyhow::Result<Option<CanonicalModelRequest>> {
        if let Some(context_budget) = budget.as_mut() {
            context_budget.used_tokens = context_budget.max_tokens;
        }
        let compacted = self
            .compact_context_for_round(
                thread_id,
                user_message_id,
                round,
                context_summary,
                conversation,
                budget,
                runtime_state,
                model_context,
                tool_candidates,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                compacted_tool_history,
                rejected_request,
                branch_developer_instructions,
                compatibility_hash,
                events,
            )
            .await;
        let Some((compacted, dropped_tool_results)) = compacted else {
            return Ok(None);
        };
        events.push(AgentEventPayload::ContextWarning {
            stage: "provider_context_overflow_recovery".to_string(),
            message: format!(
                "Provider rejected round {round} as larger than its context window. A durable checkpoint replaced covered history and the request is being retried once."
            ),
        });
        let rebuilt = self.rebuild_request_after_durable_checkpoint(
            round_model_context,
            context_summary,
            conversation,
            budget,
            model_user_message,
            model_user_content,
            tool_candidates,
            provider_tool_calls,
            provider_tool_results,
            provider_response_items,
            branch_developer_instructions,
        )?;
        self.record_round_context_compaction(
            round,
            compacted,
            dropped_tool_results,
            rejected_request.token_estimate_breakdown().total,
            rebuilt.logical().token_estimate_breakdown().total,
            events,
        );
        Ok(Some(rebuilt))
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_request_after_durable_checkpoint(
        &self,
        round_model_context: &CompiledModelContext,
        context_summary: &Option<String>,
        conversation: &mut Vec<ModelConversationMessage>,
        budget: &mut Option<ContextBudget>,
        model_user_message: &str,
        model_user_content: &[ModelContentPart],
        tool_candidates: &[ProviderToolCandidate],
        provider_tool_calls: &[ProviderToolCall],
        provider_tool_results: &[ProviderToolResult],
        provider_response_items: &[Value],
        branch_developer_instructions: Option<&str>,
    ) -> anyhow::Result<CanonicalModelRequest> {
        // A local durable checkpoint starts a new request epoch. The prompt
        // cache lineage is refreshed and no response id from the old epoch is
        // allowed into the rebuilt request.
        let admitted_model_context = refreshed_round_model_context(
            round_model_context,
            context_summary.as_deref(),
            tool_candidates,
        );
        let projected = self.assemble_model_request(
            &admitted_model_context,
            context_summary.as_deref(),
            conversation.clone(),
            model_user_message.to_string(),
            model_user_content.to_vec(),
            tool_candidates.to_vec(),
            provider_tool_calls.to_vec(),
            provider_tool_results.to_vec(),
            provider_response_items.to_vec(),
            None,
            branch_developer_instructions.map(str::to_string),
        )?;
        super::synchronize_context_budget(budget, projected.logical());
        Ok(projected)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn compact_context_for_round(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        round: usize,
        context_summary: &mut Option<String>,
        conversation: &mut Vec<ModelConversationMessage>,
        budget: &mut Option<ContextBudget>,
        runtime_state: &mut TurnRuntimeState,
        model_context: &CompiledModelContext,
        tool_candidates: &[ProviderToolCandidate],
        provider_tool_calls: &mut Vec<ProviderToolCall>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        provider_response_items: &mut Vec<Value>,
        compacted_tool_history: &mut String,
        current_request: &ModelRequest,
        branch_developer_instructions: Option<&str>,
        compatibility_hash: &mut String,
        events: &mut TurnEvents,
    ) -> Option<(RoundContextCompactionResult, usize)> {
        let Some(context_budget) = budget.as_ref() else {
            return None;
        };
        let threshold = context_compact_threshold_percent();
        if !context_budget.requires_compaction(threshold)
            || !runtime_state.can_attempt_context_compaction(round)
        {
            return None;
        }
        if conversation.is_empty() && provider_tool_results.is_empty() {
            // Stable instructions, tool schemas, and the current user input
            // are not compressible history. Avoid an auxiliary model call that
            // cannot lower pressure; a provider overflow will remain explicit.
            return None;
        }
        let Some(compactor) = self.round_context_compactor.as_ref() else {
            events.push(AgentEventPayload::ContextWarning {
                stage: "round_context_compaction_unavailable".to_string(),
                message: format!(
                    "Round {round} reached {threshold}% context pressure, but no durable context compactor is installed."
                ),
            });
            runtime_state.record_context_compaction_attempt(round);
            return None;
        };

        runtime_state.record_context_compaction_attempt(round);
        let request = RoundContextCompactionRequest {
            thread_id,
            turn_id: self.turn_id(user_message_id),
            agent_path: self.agent_path.clone(),
            round,
            estimated_input_tokens: current_request.token_estimate_breakdown().total,
            reserved_generation_tokens: context_budget.reserved_generation_tokens,
            context_window_tokens: context_budget.max_tokens,
            model_request: current_request.clone(),
        };
        let compacted = match compactor.compact(request).await {
            Ok(compacted) => compacted,
            Err(error) => {
                events.push(AgentEventPayload::ContextWarning {
                    stage: "round_context_compaction".to_string(),
                    message: format!("Durable round context compaction failed: {error:#}"),
                });
                return None;
            }
        };

        let covered_call_ids = &compacted.covered_tool_call_ids;
        let dropped_call_ids = provider_tool_results
            .iter()
            .filter(|result| covered_call_ids.contains(&result.call_id))
            .map(|result| result.call_id.clone())
            .collect::<Vec<_>>();
        provider_tool_results.retain(|result| !covered_call_ids.contains(&result.call_id));
        provider_tool_calls.retain(|call| !covered_call_ids.contains(&call.id));
        provider_response_items
            .retain(|item| provider_item_survives_local_checkpoint(item, covered_call_ids));

        *context_summary = Some(compacted.summary.summary.clone());
        // The checkpoint was generated from the exact request, so every
        // historical conversation entry in that request is now represented by
        // the checkpoint. Current user input and live round state have separate
        // owners and remain in the rebuilt request.
        conversation.clear();
        compacted_tool_history.clear();
        *compatibility_hash = provider_compatibility_hash(
            model_context,
            context_summary.as_deref(),
            tool_candidates,
            branch_developer_instructions,
        );
        Some((compacted, dropped_call_ids.len()))
    }

    fn record_round_context_compaction(
        &self,
        round: usize,
        mut compacted: RoundContextCompactionResult,
        dropped_tool_results: usize,
        before_tokens: usize,
        after_tokens: usize,
        events: &mut TurnEvents,
    ) {
        let tokens_removed = before_tokens.saturating_sub(after_tokens);
        let remaining_percent = after_tokens.saturating_mul(100) / before_tokens.max(1);
        let token_reduction_percent = tokens_removed.saturating_mul(100) / before_tokens.max(1);
        if let Some(metadata) = compacted.summary.metadata.as_object_mut() {
            metadata.insert("inputTokens".to_string(), before_tokens.into());
            metadata.insert("postCompactionTokens".to_string(), after_tokens.into());
            metadata.insert("tokensRemoved".to_string(), tokens_removed.into());
            metadata.insert("remainingPercent".to_string(), remaining_percent.into());
            metadata.insert(
                "tokenReductionPercent".to_string(),
                token_reduction_percent.into(),
            );
        }
        if let Some(metrics) = compacted
            .details
            .as_mut()
            .and_then(|details| details.metrics.as_mut())
        {
            metrics.input_tokens = before_tokens;
            metrics.post_compaction_tokens = after_tokens;
            metrics.tokens_removed = tokens_removed;
            metrics.remaining_percent = remaining_percent;
            metrics.token_reduction_percent = token_reduction_percent;
        }
        events.push(AgentEventPayload::ContextCompacted {
            summary: compacted.summary,
            details: compacted.details,
        });
        events.push(AgentEventPayload::ContextWarning {
            stage: "round_context_compaction".to_string(),
            message: format!(
                "Durable checkpoint rebuilt round {round}; request context fell from {before_tokens} to {after_tokens} tokens ({token_reduction_percent}% reduction), and {dropped_tool_results} covered completed tool result(s) were removed.",
            ),
        });
    }
}

fn refreshed_round_model_context(
    round_model_context: &CompiledModelContext,
    context_summary: Option<&str>,
    tool_candidates: &[ProviderToolCandidate],
) -> CompiledModelContext {
    let mut refreshed = round_model_context.clone();
    refreshed.prompt_cache_key = Some(super::prompt_cache_lineage_key(
        &refreshed,
        context_summary,
        tool_candidates,
    ));
    refreshed
}

fn provider_item_survives_local_checkpoint(
    item: &Value,
    covered_call_ids: &std::collections::HashSet<String>,
) -> bool {
    if item
        .get("call_id")
        .and_then(Value::as_str)
        .is_some_and(|call_id| covered_call_ids.contains(call_id))
    {
        return false;
    }

    // These items are opaque continuations of the provider's old request
    // epoch. Once a provider-neutral checkpoint replaces that epoch, replaying
    // them would mix incompatible histories. Runtime observations and live
    // function calls remain because they are explicit, inspectable evidence.
    !matches!(
        item.get("type").and_then(Value::as_str),
        Some("compaction" | "reasoning" | "openai_chat_assistant_state")
    )
}

#[cfg(test)]
mod tests {
    use super::provider_item_survives_local_checkpoint;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn local_checkpoint_drops_opaque_lineage_and_only_covered_calls() {
        let covered = HashSet::from(["covered".to_string()]);
        assert!(!provider_item_survives_local_checkpoint(
            &json!({"type": "compaction", "id": "old"}),
            &covered,
        ));
        assert!(!provider_item_survives_local_checkpoint(
            &json!({"type": "reasoning", "encrypted_content": "opaque"}),
            &covered,
        ));
        assert!(!provider_item_survives_local_checkpoint(
            &json!({"type": "function_call", "call_id": "covered"}),
            &covered,
        ));
        assert!(provider_item_survives_local_checkpoint(
            &json!({"type": "function_call", "call_id": "live"}),
            &covered,
        ));
    }
}
