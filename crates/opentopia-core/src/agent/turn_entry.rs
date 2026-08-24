use super::{
    agent_model_context_with_runtime, finalization_outcome, finalize_inbox_cancelled_turn,
    finalize_provider_turn, incomplete_model_response, json, provider_compatibility_hash,
    provider_context_window_exceeded, record_provider_tool_result_event, record_rollout_usage,
    repeated_invalid_tool_call_error, truncate_for_summary, user_denied_tool_result,
    AgentContinuation, AgentContinuationState, AgentCore, AgentEventPayload, AgentEventSender,
    AgentTurnInput, AgentTurnResult, Arc, CancellationToken, CompiledModelContext, Context,
    ContextBudget, ContextPreparationInput, ModelContentPart, ModelDecision, ProviderToolResult,
    RolloutBudget, RuntimeSurface, SessionStore, ToolCall, TurnEvents, TurnRuntimeState, Value,
    BACKGROUND_COMMAND_REMINDER_STAGE,
};

impl AgentCore {
    #[cfg(test)]
    pub(crate) async fn run_turn(
        &self,
        input: AgentTurnInput,
    ) -> anyhow::Result<Vec<AgentEventPayload>> {
        Ok(self.run_turn_detailed_streaming(input, None).await?.events)
    }

    #[cfg(test)]
    pub(crate) async fn run_turn_detailed_streaming(
        &self,
        input: AgentTurnInput,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.run_turn_detailed_streaming_with_context(input, None, sender)
            .await
    }

    pub(crate) async fn run_turn_detailed_streaming_with_context(
        &self,
        mut input: AgentTurnInput,
        model_context: Option<CompiledModelContext>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.validate_turn_admission(&input)?;
        if !self
            .capability_projection
            .allows_workspace_root(&input.workspace_root)
        {
            anyhow::bail!(
                "workspace root is outside the active ExecutionContext projection: {}",
                input.workspace_root.display()
            );
        }
        let mut events = TurnEvents::new(sender);
        let mut budget = input.context_budget;
        let mut rollout_budget = self.rollout_budget_settings.clone().map(RolloutBudget::new);

        events.push(AgentEventPayload::TurnStarted {
            user_message_id: input.user_message_id,
        });

        if let Some(ref mut budget) = budget {
            let input_tokens = ContextBudget::estimate_tokens(&input.content);
            budget.record_tokens(input_tokens);
        }

        let model_user_message = input.content.clone();
        let base_model_context = model_context.unwrap_or_else(|| {
            agent_model_context_with_runtime(
                &input.workspace_root,
                &self.tool_host.sandbox_config,
                &self.agent_runtime_settings,
                self.prompt_runtime_capabilities(RuntimeSurface::Core),
            )
        });
        let lineage_instructions = self.lineage_instructions();
        let tool_candidates = self.provider_tool_candidates();
        let model_context =
            self.kernel
                .context_assembler
                .prepare_context(ContextPreparationInput {
                    model_context: &base_model_context,
                    context_summary: input.context_summary.as_deref(),
                    tool_candidates: &tool_candidates,
                    lineage_instructions: lineage_instructions.as_deref(),
                })?;
        // Kept in continuations for backward-compatible serialization. New
        // turns materialize branch/profile/flow policy in the lineage header.
        let branch_developer_instructions = None;
        let mut provider_compatibility_hash = provider_compatibility_hash(
            &model_context,
            input.context_summary.as_deref(),
            &tool_candidates,
            branch_developer_instructions.as_deref(),
        );
        let compatible_cursor = input
            .provider_cursor
            .as_ref()
            .filter(|cursor| cursor.compatibility_hash == provider_compatibility_hash);
        if input.provider_cursor.is_some() && compatible_cursor.is_none() {
            events.push(AgentEventPayload::ProviderContextStateInvalidated {
                provider_id: None,
                model: None,
                reason: "provider context compatibility hash changed; rebuilt from the local checkpoint and recent history".to_string(),
            });
        }
        let previous_response_id = compatible_cursor
            .filter(|cursor| !cursor.response_id.is_empty())
            .map(|cursor| cursor.response_id.clone());
        let previous_response_items = compatible_cursor
            .filter(|cursor| cursor.response_id.is_empty())
            .map(|cursor| cursor.response_items.clone())
            .unwrap_or_default();
        // Work left running by an earlier turn has to be visible on the very first
        // round of this one: a user who starts a build and then asks whether it is done
        // should not have to wait for a second round to hear the answer.
        let mut runtime_state = TurnRuntimeState::default();
        let opening_reminders = self.collect_step_reminders(
            input.thread_id,
            input.user_message_id,
            0,
            rollout_budget.as_ref(),
            &runtime_state,
        );
        if opening_reminders.cancelled {
            return Ok(finalize_inbox_cancelled_turn(input.thread_id, events));
        }
        let mut opening_provider_tool_calls = Vec::new();
        let mut opening_provider_tool_results = Vec::new();
        let mut opening_provider_response_items = previous_response_items.clone();
        for reminder in &opening_reminders.reminders {
            events.push(AgentEventPayload::ContextWarning {
                stage: format!("step_reminder.{}", reminder.stage),
                message: truncate_for_summary(&reminder.content, 400),
            });
            if reminder.stage != BACKGROUND_COMMAND_REMINDER_STAGE {
                self.append_step_reminder_observation(
                    &reminder.stage,
                    &reminder.content,
                    reminder.observation_id.as_deref(),
                    &mut opening_provider_tool_calls,
                    &mut opening_provider_tool_results,
                    &mut opening_provider_response_items,
                    &mut events,
                );
            }
        }
        for async_result in &opening_reminders.async_tool_results {
            self.append_background_completion_observation(
                async_result,
                &mut opening_provider_tool_calls,
                &mut opening_provider_tool_results,
                &mut opening_provider_response_items,
                &mut events,
            );
        }
        let mut compacted_tool_history = String::new();
        let opening_request = self
            .admitted_round_request(
                input.thread_id,
                input.user_message_id,
                0,
                &model_context,
                &model_context,
                &mut input.context_summary,
                &mut input.conversation,
                &mut budget,
                &mut runtime_state,
                &model_user_message,
                &input.user_content,
                &tool_candidates,
                &mut opening_provider_tool_calls,
                &mut opening_provider_tool_results,
                &mut compacted_tool_history,
                &mut opening_provider_response_items,
                previous_response_id,
                branch_developer_instructions.as_deref(),
                &mut provider_compatibility_hash,
                &mut events,
            )
            .await?;
        let rejected_opening_request = opening_request.logical().clone();
        let response = match self
            .complete_model(
                opening_request,
                1,
                &provider_compatibility_hash,
                &mut events,
                input.cancellation.as_ref(),
            )
            .await
        {
            Ok(response) => response,
            Err(_)
                if input
                    .cancellation
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled) =>
            {
                return Ok(finalize_inbox_cancelled_turn(input.thread_id, events));
            }
            Err(error) if provider_context_window_exceeded(&error) => {
                let Some(retry_request) = self
                    .request_after_context_overflow(
                        input.thread_id,
                        input.user_message_id,
                        0,
                        &model_context,
                        &model_context,
                        &mut input.context_summary,
                        &mut input.conversation,
                        &mut budget,
                        &mut runtime_state,
                        &model_user_message,
                        &input.user_content,
                        &tool_candidates,
                        &mut opening_provider_tool_calls,
                        &mut opening_provider_tool_results,
                        &mut compacted_tool_history,
                        &mut opening_provider_response_items,
                        &rejected_opening_request,
                        branch_developer_instructions.as_deref(),
                        &mut provider_compatibility_hash,
                        &mut events,
                    )
                    .await?
                else {
                    return Err(error);
                };
                let retry = self
                    .complete_model(
                        retry_request,
                        1,
                        &provider_compatibility_hash,
                        &mut events,
                        input.cancellation.as_ref(),
                    )
                    .await;
                if input
                    .cancellation
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
                {
                    return Ok(finalize_inbox_cancelled_turn(input.thread_id, events));
                }
                retry?
            }
            Err(error) => return Err(error),
        };
        self.commit_step_reminders(opening_reminders, &mut rollout_budget, &mut runtime_state)
            .await?;
        let model_rounds = 1;
        let rollout_reviews = 0;
        if let Some(ref mut budget) = budget {
            budget.record_tokens(ContextBudget::estimate_tokens(&response.text));
            if let Some(usage) = response.usage.as_ref() {
                budget.record_provider_usage(usage);
            }
        }
        record_rollout_usage(&mut rollout_budget, response.usage.as_ref())?;
        let post_parse_control = self.drain_post_parse_control(input.user_message_id);
        if post_parse_control.cancelled {
            return Ok(finalize_inbox_cancelled_turn(input.thread_id, events));
        }
        if !post_parse_control.steers.is_empty() {
            self.append_steer_observations(
                &post_parse_control.steers,
                &mut opening_provider_tool_calls,
                &mut opening_provider_tool_results,
                &mut opening_provider_response_items,
                &mut events,
            );
            return self
                .continue_provider_turn(
                    input.thread_id,
                    input.user_message_id,
                    input.workspace_root,
                    input.context_summary,
                    input.conversation,
                    input.permission_mode,
                    budget,
                    rollout_budget,
                    model_rounds,
                    rollout_reviews,
                    runtime_state,
                    model_context,
                    input.store,
                    input.cancellation,
                    model_user_message,
                    input.user_content,
                    tool_candidates,
                    opening_provider_tool_calls,
                    opening_provider_tool_results,
                    Vec::new(),
                    compacted_tool_history,
                    opening_provider_response_items,
                    branch_developer_instructions,
                    provider_compatibility_hash,
                    None,
                    &mut events,
                )
                .await;
        }
        let mut provider_response_items = opening_provider_response_items.clone();
        provider_response_items.extend(response.provider_items.iter().cloned());
        match response.decision() {
            ModelDecision::Incomplete(reason) => {
                return Err(incomplete_model_response(reason, &response));
            }
            ModelDecision::Final(_) => {
                let mut provider_tool_calls = opening_provider_tool_calls;
                let mut provider_tool_results = opening_provider_tool_results;
                if let Some(intervention) = self
                    .apply_finalization_guard(
                        input.thread_id,
                        input.user_message_id,
                        input.store.as_ref(),
                        &[],
                        &mut provider_tool_calls,
                        &mut provider_tool_results,
                        &mut provider_response_items,
                        &mut events,
                    )
                    .await?
                {
                    return self
                        .continue_provider_turn(
                            input.thread_id,
                            input.user_message_id,
                            input.workspace_root,
                            input.context_summary,
                            input.conversation,
                            input.permission_mode,
                            budget,
                            rollout_budget,
                            model_rounds,
                            rollout_reviews,
                            runtime_state.clone(),
                            model_context,
                            input.store,
                            input.cancellation,
                            model_user_message,
                            input.user_content,
                            tool_candidates,
                            provider_tool_calls,
                            provider_tool_results,
                            Vec::new(),
                            String::new(),
                            provider_response_items,
                            branch_developer_instructions,
                            provider_compatibility_hash,
                            intervention.agent_delivery,
                            &mut events,
                        )
                        .await;
                }
                let outcome = finalization_outcome(
                    input.store.as_ref(),
                    self.turn_id(input.user_message_id),
                    self.goal.as_ref().map(|goal| goal.id),
                    &provider_tool_results,
                )?;
                return Ok(finalize_provider_turn(
                    input.thread_id,
                    self.collaboration_mode,
                    response,
                    opening_provider_response_items,
                    provider_tool_results,
                    budget,
                    events,
                    provider_compatibility_hash,
                    outcome,
                ));
            }
            ModelDecision::Act(tool_calls) => {
                if let Some(message) =
                    repeated_invalid_tool_call_error(&runtime_state, &tool_calls, &tool_candidates)
                {
                    events.push(AgentEventPayload::ContextWarning {
                        stage: "invalid_tool_call_circuit_breaker".to_string(),
                        message: message.clone(),
                    });
                    anyhow::bail!(message);
                }
                runtime_state.record_tool_calls(&tool_calls);
            }
        }

        opening_provider_tool_calls.extend(response.tool_calls.clone());
        self.continue_provider_turn(
            input.thread_id,
            input.user_message_id,
            input.workspace_root,
            input.context_summary,
            input.conversation,
            input.permission_mode,
            budget,
            rollout_budget,
            model_rounds,
            rollout_reviews,
            runtime_state,
            model_context,
            input.store,
            input.cancellation,
            model_user_message,
            input.user_content,
            tool_candidates,
            opening_provider_tool_calls,
            opening_provider_tool_results,
            response.tool_calls,
            String::new(),
            provider_response_items,
            branch_developer_instructions,
            provider_compatibility_hash,
            None,
            &mut events,
        )
        .await
    }

    pub(crate) async fn resume_from_signal_streaming(
        &self,
        continuation: AgentContinuation,
        signal: crate::agent_runtime::AgentResumeSignal,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        let mut events = TurnEvents::new(sender);
        events.push(AgentEventPayload::TurnStarted {
            user_message_id: continuation.user_message_id,
        });

        match continuation.state {
            AgentContinuationState::Provider {
                model_user_message,
                model_user_content,
                mut tool_candidates,
                provider_tool_calls,
                mut provider_tool_results,
                mut pending_tool_calls,
                compacted_tool_history,
                provider_response_items,
                model_rounds,
                rollout_reviews,
                mut runtime_state,
                branch_developer_instructions,
                mut provider_compatibility_hash,
            } => {
                let refreshed_tool_candidates =
                    self.refresh_resumed_tool_candidates(&tool_candidates);
                if refreshed_tool_candidates != tool_candidates {
                    tool_candidates = refreshed_tool_candidates;
                    provider_compatibility_hash = super::provider_compatibility_hash(
                        &continuation.model_context,
                        continuation.context_summary.as_deref(),
                        &tool_candidates,
                        branch_developer_instructions.as_deref(),
                    );
                    events.push(AgentEventPayload::ProviderContextStateInvalidated {
                        provider_id: None,
                        model: None,
                        reason: "tool catalog changed while the turn was suspended; refreshed current tool contracts"
                            .to_string(),
                    });
                }
                let first_new_result = provider_tool_results.len();
                match signal {
                    crate::agent_runtime::AgentResumeSignal::Approval { approved, .. } => {
                        let batch_approval = runtime_state.pending_approval_call_ids.len() > 1;
                        let mut approved_call_ids =
                            std::mem::take(&mut runtime_state.pending_approval_call_ids);
                        if approved_call_ids.is_empty() {
                            approved_call_ids.push(
                                pending_tool_calls
                                    .first()
                                    .context("provider continuation has no pending call")?
                                    .id
                                    .clone(),
                            );
                        }
                        let approved_call_count = approved_call_ids.len();
                        let approved_calls = approved_call_ids
                            .iter()
                            .enumerate()
                            .map(|(index, expected_call_id)| {
                                let pending =
                                    pending_tool_calls.get(index).cloned().ok_or_else(|| {
                                        anyhow::anyhow!(
                                            "batch approval references missing provider call `{expected_call_id}`"
                                        )
                                    })?;
                                anyhow::ensure!(
                                    pending.id == *expected_call_id,
                                    "batch approval order mismatch: expected `{expected_call_id}`, found `{}`",
                                    pending.id
                                );
                                Ok(pending)
                            })
                            .collect::<anyhow::Result<Vec<_>>>()?;
                        let resumed_results = if approved {
                            self.grant_turn_path_leases(
                                &mut runtime_state,
                                &approved_calls,
                                &continuation.workspace_root,
                            )?;
                            self.execute_scoped_approved_batch(
                                approved_calls,
                                &continuation.workspace_root,
                                continuation.permission_mode,
                                store.clone(),
                                cancellation.clone(),
                                continuation.thread_id,
                                continuation.user_message_id,
                                if batch_approval { "user_batch" } else { "user" },
                                &mut events,
                            )
                            .await?
                        } else {
                            let results = approved_calls
                                .iter()
                                .map(user_denied_tool_result)
                                .collect::<Vec<_>>();
                            for (call, result) in approved_calls.iter().zip(&results) {
                                record_provider_tool_result_event(
                                    &mut events,
                                    ToolCall::new(&call.name, call.arguments.clone()),
                                    result,
                                );
                            }
                            results
                        };
                        pending_tool_calls.drain(..approved_call_count);
                        provider_tool_results.extend(resumed_results);
                    }
                    crate::agent_runtime::AgentResumeSignal::UserInput {
                        request_id,
                        response,
                    } => {
                        let request_id_text = request_id.to_string();
                        let result = provider_tool_results
                            .iter_mut()
                            .rev()
                            .find(|result| {
                                result
                                    .metadata
                                    .get("userInputRequest")
                                    .and_then(|value| value.get("requestId"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|value| value == request_id_text)
                            })
                            .context(
                                "user input continuation does not contain the matching request",
                            )?;
                        let response_value = serde_json::to_value(&response)?;
                        result.output = serde_json::to_string(&response_value)?;
                        result.content = vec![ModelContentPart::json(response_value.clone())];
                        result.is_error = false;
                        if let Some(metadata) = result.metadata.as_object_mut() {
                            metadata.remove("userInputRequest");
                            metadata.insert("waitingForUserInput".to_string(), json!(false));
                        }
                    }
                    crate::agent_runtime::AgentResumeSignal::ExternalAction { observation } => {
                        let call = pending_tool_calls
                            .first()
                            .cloned()
                            .context("external-action continuation has no pending tool call")?;
                        pending_tool_calls.remove(0);
                        let payload = json!({
                            "completed": true,
                            "observation": observation,
                            "next": "Re-observe the external surface before taking another action.",
                        });
                        provider_tool_results.push(ProviderToolResult {
                            call_id: call.id,
                            name: call.name,
                            output: serde_json::to_string_pretty(&payload)?,
                            content: vec![ModelContentPart::json(payload)],
                            is_error: false,
                            metadata: json!({
                                "externalActionCompleted": true,
                                "executedBy": "user",
                            }),
                        });
                    }
                }

                let mut context_budget = continuation.context_budget;
                let rollout_budget = continuation.rollout_budget;
                if let Some(ref mut budget) = context_budget {
                    for result in &provider_tool_results[first_new_result..] {
                        budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                    }
                }

                self.continue_provider_turn(
                    continuation.thread_id,
                    continuation.user_message_id,
                    continuation.workspace_root,
                    continuation.context_summary,
                    continuation.conversation,
                    continuation.permission_mode,
                    context_budget,
                    rollout_budget,
                    model_rounds,
                    rollout_reviews,
                    runtime_state,
                    continuation.model_context,
                    store,
                    cancellation,
                    model_user_message,
                    model_user_content,
                    tool_candidates,
                    provider_tool_calls,
                    provider_tool_results,
                    pending_tool_calls,
                    compacted_tool_history,
                    provider_response_items,
                    branch_developer_instructions,
                    provider_compatibility_hash,
                    None,
                    &mut events,
                )
                .await
            }
        }
    }
}
