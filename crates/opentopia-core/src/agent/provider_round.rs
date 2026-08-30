use super::{
    finalization_outcome, finalize_inbox_cancelled_turn, finalize_provider_turn,
    finalize_rollout_hard_limit_turn, incomplete_model_response, latest_work_form,
    provider_context_window_exceeded, record_rollout_usage, repeated_invalid_tool_call_error,
    rollout_checkpoint_due, truncate_for_summary, AgentCompletionGuardDelivery, AgentCore,
    AgentEventPayload, AgentTurnResult, Arc, CancellationToken, CompiledModelContext,
    ContextBudget, ModelContentPart, ModelConversationMessage, ModelDecision, ProviderToolCall,
    ProviderToolCandidate, ProviderToolResult, RolloutBudget, RolloutCheckpointObservation,
    SessionStore, TurnEvents, TurnRuntimeState, Uuid, BACKGROUND_COMMAND_REMINDER_STAGE,
    MAX_ROLLOUT_MODEL_ROUNDS,
};

pub(super) enum ProviderRoundOutcome {
    Continue {
        model_rounds: usize,
        rollout_reviews: usize,
    },
    Finished(AgentTurnResult),
}

impl AgentCore {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn complete_provider_round(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        workspace_root: &std::path::Path,
        permission_mode: super::PermissionMode,
        context_summary: &mut Option<String>,
        conversation: &mut Vec<ModelConversationMessage>,
        budget: &mut Option<ContextBudget>,
        rollout_budget: &mut Option<RolloutBudget>,
        mut model_rounds: usize,
        mut rollout_reviews: usize,
        runtime_state: &mut TurnRuntimeState,
        model_context: &CompiledModelContext,
        store: Option<&Arc<dyn SessionStore>>,
        cancellation: Option<&CancellationToken>,
        model_user_message: &str,
        model_user_content: &[ModelContentPart],
        tool_candidates: &mut Vec<ProviderToolCandidate>,
        provider_tool_calls: &mut Vec<ProviderToolCall>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        pending_tool_calls: &mut Vec<ProviderToolCall>,
        compacted_tool_history: &mut String,
        provider_response_items: &mut Vec<serde_json::Value>,
        branch_developer_instructions: Option<&str>,
        compatibility_hash: &mut String,
        completion_guard_delivery: &mut Option<AgentCompletionGuardDelivery>,
        events: &mut TurnEvents,
    ) -> anyhow::Result<ProviderRoundOutcome> {
        if model_rounds >= MAX_ROLLOUT_MODEL_ROUNDS {
            return Ok(ProviderRoundOutcome::Finished(
                finalize_rollout_hard_limit_turn(
                    thread_id,
                    model_rounds,
                    std::mem::replace(events, TurnEvents::new(None)),
                ),
            ));
        }

        if rollout_checkpoint_due(model_rounds, rollout_reviews) {
            rollout_reviews = rollout_reviews.saturating_add(1);
            let latest_form = latest_work_form(events, &provider_tool_results);
            events.push(AgentEventPayload::ContextWarning {
                stage: "rollout_self_review_checkpoint".to_string(),
                message: format!(
                    "Main-model self-review checkpoint after {model_rounds} completed rounds; the runtime supplied counters without making a progress decision."
                ),
            });
            self.apply_rollout_checkpoint_observation(
                RolloutCheckpointObservation {
                    model_rounds,
                    remaining_budget_tokens: rollout_budget
                        .as_ref()
                        .map(RolloutBudget::remaining_tokens),
                    work_form: latest_form.as_ref(),
                },
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                events,
            )?;
        }

        if rollout_budget
            .as_ref()
            .is_some_and(RolloutBudget::is_exhausted)
        {
            anyhow::bail!("shared rollout token budget exhausted");
        }
        // Everything the runtime noticed since the previous round reaches the
        // model here as evidence rather than as control flow.
        let step_reminders = self.collect_step_reminders(
            thread_id,
            user_message_id,
            model_rounds,
            rollout_budget.as_ref(),
            runtime_state,
        );
        if step_reminders.cancelled {
            return Ok(ProviderRoundOutcome::Finished(
                finalize_inbox_cancelled_turn(
                    thread_id,
                    std::mem::replace(events, TurnEvents::new(None)),
                ),
            ));
        }
        let round_model_context = model_context.clone();
        for reminder in &step_reminders.reminders {
            events.push(AgentEventPayload::ContextWarning {
                stage: format!("step_reminder.{}", reminder.stage),
                message: truncate_for_summary(&reminder.content, 400),
            });
            if reminder.stage != BACKGROUND_COMMAND_REMINDER_STAGE {
                self.append_step_reminder_observation(
                    &reminder.stage,
                    &reminder.content,
                    reminder.observation_id.as_deref(),
                    provider_tool_calls,
                    provider_tool_results,
                    provider_response_items,
                    events,
                );
            }
        }
        for async_result in &step_reminders.async_tool_results {
            self.append_background_completion_observation(
                async_result,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                events,
            );
        }
        let request = self
            .admitted_round_request(
                thread_id,
                user_message_id,
                model_rounds,
                model_context,
                &round_model_context,
                context_summary,
                conversation,
                budget,
                runtime_state,
                model_user_message,
                model_user_content,
                tool_candidates,
                provider_tool_calls,
                provider_tool_results,
                compacted_tool_history,
                provider_response_items,
                None,
                branch_developer_instructions,
                compatibility_hash,
                events,
            )
            .await?;
        let rejected_request = request.logical().clone();
        let mut streaming_tools = super::streaming_tool_execution::StreamingToolExecution::new(
            self,
            thread_id,
            user_message_id,
            workspace_root.to_path_buf(),
            permission_mode,
            runtime_state,
            store.cloned(),
            cancellation.cloned(),
            conversation,
            model_user_message,
            model_user_content,
            model_context,
            events,
        )?;
        let response_result = self
            .complete_model(
                request,
                model_rounds.saturating_add(1),
                compatibility_hash,
                &mut streaming_tools,
                events,
                cancellation,
            )
            .await;
        let tool_items_committed = streaming_tools.committed_any();
        let streaming_execution = streaming_tools.finish().await?;
        if let Ok(response) = response_result.as_ref() {
            streaming_execution.validate_terminal_calls(&response.tool_calls)?;
        }
        let mut completed_streaming_call_ids = self.commit_streaming_tool_execution(
            streaming_execution,
            budget,
            tool_candidates,
            provider_tool_results,
            events,
        )?;
        let response = match response_result {
            Ok(response) => response,
            Err(_) if cancellation.is_some_and(CancellationToken::is_cancelled) => {
                return Ok(ProviderRoundOutcome::Finished(
                    finalize_inbox_cancelled_turn(
                        thread_id,
                        std::mem::replace(events, TurnEvents::new(None)),
                    ),
                ));
            }
            Err(error) if !tool_items_committed && provider_context_window_exceeded(&error) => {
                let Some(retry_request) = self
                    .request_after_context_overflow(
                        thread_id,
                        user_message_id,
                        model_rounds,
                        model_context,
                        &round_model_context,
                        context_summary,
                        conversation,
                        budget,
                        runtime_state,
                        model_user_message,
                        model_user_content,
                        tool_candidates,
                        provider_tool_calls,
                        provider_tool_results,
                        compacted_tool_history,
                        provider_response_items,
                        &rejected_request,
                        branch_developer_instructions,
                        compatibility_hash,
                        events,
                    )
                    .await?
                else {
                    return Err(error);
                };
                let mut retry_streaming_tools =
                    super::streaming_tool_execution::StreamingToolExecution::new(
                        self,
                        thread_id,
                        user_message_id,
                        workspace_root.to_path_buf(),
                        permission_mode,
                        runtime_state,
                        store.cloned(),
                        cancellation.cloned(),
                        conversation,
                        model_user_message,
                        model_user_content,
                        model_context,
                        events,
                    )?;
                let retry = self
                    .complete_model(
                        retry_request,
                        model_rounds.saturating_add(1),
                        compatibility_hash,
                        &mut retry_streaming_tools,
                        events,
                        cancellation,
                    )
                    .await;
                let retry_streaming_execution = retry_streaming_tools.finish().await?;
                if let Ok(response) = retry.as_ref() {
                    retry_streaming_execution.validate_terminal_calls(&response.tool_calls)?;
                }
                completed_streaming_call_ids.extend(self.commit_streaming_tool_execution(
                    retry_streaming_execution,
                    budget,
                    tool_candidates,
                    provider_tool_results,
                    events,
                )?);
                if cancellation.is_some_and(CancellationToken::is_cancelled) {
                    return Ok(ProviderRoundOutcome::Finished(
                        finalize_inbox_cancelled_turn(
                            thread_id,
                            std::mem::replace(events, TurnEvents::new(None)),
                        ),
                    ));
                }
                retry?
            }
            Err(error) => return Err(error),
        };
        model_rounds = model_rounds.saturating_add(1);
        // The round carrying these observations reached the model, so the
        // matching state may now be advanced. A round that failed or was
        // cancelled above leaves them pending and redelivers them next time.
        self.commit_step_reminders(step_reminders, rollout_budget, runtime_state)
            .await?;
        if let Some(delivery) = completion_guard_delivery.take() {
            self.acknowledge_completion_delivery(&delivery).await?;
        }
        if let Some(budget) = budget.as_mut() {
            budget.record_tokens(ContextBudget::estimate_tokens(&response.text));
            if let Some(usage) = response.usage.as_ref() {
                budget.record_provider_usage(usage);
            }
        }
        record_rollout_usage(rollout_budget, response.usage.as_ref())?;

        let post_parse_control = self.drain_post_parse_control(user_message_id);
        if post_parse_control.cancelled {
            return Ok(ProviderRoundOutcome::Finished(
                finalize_inbox_cancelled_turn(
                    thread_id,
                    std::mem::replace(events, TurnEvents::new(None)),
                ),
            ));
        }
        if !post_parse_control.steers.is_empty() && response.tool_calls.is_empty() {
            self.append_steer_observations(
                &post_parse_control.steers,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                events,
            );
            return Ok(ProviderRoundOutcome::Continue {
                model_rounds,
                rollout_reviews,
            });
        }

        let response_committed_before_decision = !post_parse_control.steers.is_empty();
        if response_committed_before_decision {
            provider_response_items.extend(response.provider_items.iter().cloned());
            self.append_steer_observations(
                &post_parse_control.steers,
                provider_tool_calls,
                provider_tool_results,
                provider_response_items,
                events,
            );
        }

        match response.decision() {
            ModelDecision::Incomplete(reason) => {
                return Err(incomplete_model_response(reason, &response));
            }
            ModelDecision::Final(_) => {
                if let Some(intervention) = self
                    .apply_finalization_guard(
                        thread_id,
                        user_message_id,
                        store,
                        pending_tool_calls,
                        provider_tool_calls,
                        provider_tool_results,
                        provider_response_items,
                        events,
                    )
                    .await?
                {
                    *completion_guard_delivery = intervention.agent_delivery;
                    return Ok(ProviderRoundOutcome::Continue {
                        model_rounds,
                        rollout_reviews,
                    });
                }
                let outcome = finalization_outcome(
                    store,
                    self.turn_id(user_message_id),
                    self.goal.as_ref().map(|goal| goal.id),
                    provider_tool_results,
                )?;
                return Ok(ProviderRoundOutcome::Finished(finalize_provider_turn(
                    thread_id,
                    self.collaboration_mode,
                    response,
                    std::mem::take(provider_response_items),
                    std::mem::take(provider_tool_results),
                    budget.take(),
                    std::mem::replace(events, TurnEvents::new(None)),
                    compatibility_hash.to_string(),
                    outcome,
                )));
            }
            ModelDecision::Act(tool_calls) => {
                if let Some(message) =
                    repeated_invalid_tool_call_error(runtime_state, &tool_calls, tool_candidates)
                {
                    events.push(AgentEventPayload::ContextWarning {
                        stage: "invalid_tool_call_circuit_breaker".to_string(),
                        message: message.clone(),
                    });
                    anyhow::bail!(message);
                }
                runtime_state.record_tool_calls(&tool_calls);
                *pending_tool_calls = tool_calls
                    .into_iter()
                    .filter(|call| !completed_streaming_call_ids.contains(&call.id))
                    .collect();
            }
        }
        if !response_committed_before_decision {
            provider_response_items.extend(response.provider_items.iter().cloned());
        }
        provider_tool_calls.extend(response.tool_calls.iter().cloned());
        if let Some(budget) = budget.as_mut() {
            budget.record_tokens(0);
        }
        Ok(ProviderRoundOutcome::Continue {
            model_rounds,
            rollout_reviews,
        })
    }
}
