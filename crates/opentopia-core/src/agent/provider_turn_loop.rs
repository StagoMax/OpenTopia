use super::{
    approval_required, browser_handoff_required, current_work_form_for_tool,
    finalize_automatic_review_failure_turn, policy_denied_tool_result, provider_compatibility_hash,
    provider_tool_approval_action, record_provider_tool_result_event, unreviewable_action_result,
    AgentCompletionGuardDelivery, AgentContinuation, AgentContinuationState, AgentCore,
    AgentEventPayload, AgentTurnOutcome, AgentTurnResult, ApprovalsReviewer, Arc,
    CancellationToken, CompiledModelContext, Context, ContextBudget, ExecutionAuthority,
    GuardianApprovalAction, GuardianApprovalRequest, GuardianReviewStatus, HashMap,
    ModelContentPart, ModelConversationMessage, ModelConversationRole, PathBuf, PermissionMode,
    ProviderRoundOutcome, ProviderToolCall, ProviderToolCandidate, ProviderToolResult,
    RolloutBudget, SessionStore, ToolCall, ToolReviewInput, ToolStateStore, TurnEvents,
    TurnRuntimeState, UserInputRequest, Uuid, Value,
};

impl AgentCore {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn continue_provider_turn(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
        workspace_root: PathBuf,
        context_summary: Option<String>,
        mut conversation: Vec<ModelConversationMessage>,
        permission_mode: PermissionMode,
        mut budget: Option<ContextBudget>,
        mut rollout_budget: Option<RolloutBudget>,
        mut model_rounds: usize,
        mut rollout_reviews: usize,
        mut runtime_state: TurnRuntimeState,
        model_context: CompiledModelContext,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        model_user_message: String,
        model_user_content: Vec<ModelContentPart>,
        mut tool_candidates: Vec<ProviderToolCandidate>,
        mut provider_tool_calls: Vec<ProviderToolCall>,
        mut provider_tool_results: Vec<ProviderToolResult>,
        mut pending_tool_calls: Vec<ProviderToolCall>,
        mut compacted_tool_history: String,
        mut provider_response_items: Vec<Value>,
        branch_developer_instructions: Option<String>,
        mut compatibility_hash: String,
        mut completion_guard_delivery: Option<AgentCompletionGuardDelivery>,
        events: &mut TurnEvents,
    ) -> anyhow::Result<AgentTurnResult> {
        let mut parallel_outcomes: HashMap<
            String,
            (anyhow::Result<ProviderToolResult>, TurnEvents),
        > = HashMap::new();
        loop {
            while !pending_tool_calls.is_empty() {
                let front_call_id = pending_tool_calls
                    .first()
                    .expect("non-empty pending tool-call queue")
                    .id
                    .clone();
                if let Some((result, local_events)) = parallel_outcomes.remove(&front_call_id) {
                    // Calls may start out of order when they have independent
                    // resources, but provider results and durable events remain
                    // in the exact order emitted by the model.
                    let provider_call = pending_tool_calls
                        .first()
                        .cloned()
                        .expect("non-empty pending tool-call queue");
                    match result {
                        Ok(result) => {
                            for event in local_events.into_vec() {
                                events.push(event);
                            }
                            let user_input_request = result
                                .metadata
                                .get("userInputRequest")
                                .cloned()
                                .map(serde_json::from_value::<UserInputRequest>)
                                .transpose()?;
                            anyhow::ensure!(
                                user_input_request.is_none(),
                                "parallel tool `{}` unexpectedly requested user input",
                                provider_call.name
                            );
                            if let Some(ref mut budget) = budget {
                                budget
                                    .record_tokens(ContextBudget::estimate_tokens(&result.output));
                            }
                            if self.reveal_tools_from_search_result(&result, &mut tool_candidates) {
                                compatibility_hash = provider_compatibility_hash(
                                    &model_context,
                                    context_summary.as_deref(),
                                    &tool_candidates,
                                    branch_developer_instructions.as_deref(),
                                );
                            }
                            provider_tool_results.push(result);
                            pending_tool_calls.remove(0);
                            continue;
                        }
                        Err(error)
                            if approval_required(&error).is_some()
                                || browser_handoff_required(&error).is_some() =>
                        {
                            // The preflight is deliberately conservative, but a
                            // tool may discover an interactive boundary only at
                            // execution time. Re-enter the ordinary sequential
                            // path so approval/handoff state is persisted instead
                            // of aborting the turn with `?`.
                        }
                        Err(error) => {
                            for event in local_events.into_vec() {
                                events.push(event);
                            }
                            return Err(error).with_context(|| {
                                format!(
                                    "parallel tool `{}` failed before returning a tool result",
                                    provider_call.name
                                )
                            });
                        }
                    }
                }

                let turn_sandbox_config =
                    runtime_state.sandbox_config_with_path_leases(&self.tool_host.sandbox_config);
                if parallel_outcomes.is_empty() {
                    let batch = self.approval_candidates(
                        &pending_tool_calls,
                        &workspace_root,
                        permission_mode,
                        &turn_sandbox_config,
                    );
                    if !batch.is_empty() {
                        if permission_mode.approvals_reviewer() == ApprovalsReviewer::User {
                            let approval_id = Uuid::new_v4();
                            let approval_reason = if batch.len() == 1 {
                                format!("approval required: {}", batch[0].reason)
                            } else {
                                format!(
                                    "approval required for {} actions: {}",
                                    batch.len(),
                                    batch
                                        .iter()
                                        .map(|item| item.reason.as_str())
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                )
                            };
                            let approval_action = batch
                                .iter()
                                .map(|item| provider_tool_approval_action(&item.call))
                                .collect::<Vec<_>>()
                                .join("\n");
                            runtime_state.pending_approval_call_ids =
                                batch.iter().map(|item| item.call.id.clone()).collect();
                            events.push(AgentEventPayload::ApprovalRequested {
                                approval_id,
                                reason: approval_reason.clone(),
                                action: approval_action,
                            });
                            events.push(AgentEventPayload::TurnSuspended {
                                approval_id,
                                reason: approval_reason,
                            });
                            return Ok(AgentTurnResult {
                                events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                                outcome: AgentTurnOutcome::Suspended {
                                    approval_id,
                                    continuation: AgentContinuation {
                                        thread_id,
                                        turn_id: self.turn_id(user_message_id),
                                        invocation_id: self.invocation_id,
                                        user_message_id,
                                        workspace_root: workspace_root.clone(),
                                        context_summary,
                                        conversation,
                                        permission_mode,
                                        execution_authority: Some(ExecutionAuthority::new(
                                            workspace_root.clone(),
                                            permission_mode,
                                            turn_sandbox_config.clone(),
                                            self.capability_projection.clone(),
                                        )?),
                                        context_budget: budget,
                                        rollout_budget,
                                        model_context,
                                        collaboration_mode: self.collaboration_mode,
                                        goal: self.goal.clone(),
                                        state: AgentContinuationState::Provider {
                                            model_user_message,
                                            model_user_content,
                                            tool_candidates,
                                            provider_tool_calls,
                                            provider_tool_results,
                                            pending_tool_calls,
                                            compacted_tool_history,
                                            provider_response_items,
                                            model_rounds,
                                            rollout_reviews,
                                            runtime_state: runtime_state.clone(),
                                            branch_developer_instructions,
                                            provider_compatibility_hash: compatibility_hash,
                                        },
                                    },
                                },
                                provider_cursor: None,
                            });
                        }

                        let target_item_id = batch[0].call.id.clone();
                        let boundary_reason = batch
                            .iter()
                            .map(|item| format!("{}: {}", item.call.name, item.reason))
                            .collect::<Vec<_>>()
                            .join("\n");
                        let review_reason = if batch.len() == 1 {
                            format!(
                                "Review the exact approval-bound action:\n{}",
                                boundary_reason
                            )
                        } else {
                            format!(
                                "Review {} exact approval-bound actions as one provider batch:\n{}",
                                batch.len(),
                                boundary_reason
                            )
                        };
                        let review_action = if batch.len() == 1 {
                            batch[0].action.clone()
                        } else {
                            GuardianApprovalAction::Batch {
                                actions: batch.iter().map(|item| item.action.clone()).collect(),
                            }
                        };
                        let request = GuardianApprovalRequest::new(
                            thread_id,
                            user_message_id,
                            review_reason,
                            review_action,
                        );
                        let action_summary = request.action.event_summary();
                        events.push(AgentEventPayload::AutomaticApprovalReviewStarted {
                            review_id: request.review_id,
                            target_item_id: target_item_id.clone(),
                            action: action_summary.clone(),
                        });
                        let review = self
                            .kernel
                            .tool_runtime
                            .review(
                                ToolReviewInput {
                                    guardian: &self.guardian,
                                    request: &request,
                                    conversation: &conversation,
                                    current_user_message: &model_user_message,
                                    tool_calls: &provider_tool_calls,
                                    tool_results: &provider_tool_results,
                                    workspace_root: &workspace_root,
                                    sandbox_config: &turn_sandbox_config,
                                },
                                cancellation.as_ref(),
                            )
                            .await;
                        events.push(AgentEventPayload::AutomaticApprovalReviewCompleted {
                            review_id: request.review_id,
                            target_item_id,
                            status: review.status,
                            risk_level: review.assessment.as_ref().map(|value| value.risk_level),
                            user_authorization: review
                                .assessment
                                .as_ref()
                                .map(|value| value.user_authorization),
                            rationale: review.rationale.clone(),
                            action: action_summary,
                            usage: review.usage.clone(),
                            attempts: review.attempts,
                            tool_rounds: review.tool_rounds,
                            decision_source: review.decision_source,
                            failure_kind: review.failure_kind,
                        });
                        if review.status == GuardianReviewStatus::Aborted {
                            anyhow::bail!("cancelled");
                        }
                        if review.technical_failure() {
                            return Ok(finalize_automatic_review_failure_turn(
                                thread_id,
                                review.status,
                                review.rationale,
                                std::mem::replace(events, TurnEvents::new(None)),
                            ));
                        }
                        if let Some(message) = review.interrupt_turn {
                            events.push(AgentEventPayload::AutoReviewInterruptionWarning {
                                message: message.clone(),
                            });
                            anyhow::bail!(message);
                        }

                        if review.needs_user_approval() {
                            let approval_id = Uuid::new_v4();
                            let approval_reason = if batch.len() == 1 {
                                format!(
                                    "automatic reviewer requires user approval: {}",
                                    review.rationale
                                )
                            } else {
                                format!(
                                    "automatic reviewer requires user approval for {} actions: {}",
                                    batch.len(),
                                    review.rationale
                                )
                            };
                            let approval_action = batch
                                .iter()
                                .map(|item| provider_tool_approval_action(&item.call))
                                .collect::<Vec<_>>()
                                .join("\n");
                            runtime_state.pending_approval_call_ids =
                                batch.iter().map(|item| item.call.id.clone()).collect();
                            events.push(AgentEventPayload::ApprovalRequested {
                                approval_id,
                                reason: approval_reason.clone(),
                                action: approval_action,
                            });
                            events.push(AgentEventPayload::TurnSuspended {
                                approval_id,
                                reason: approval_reason,
                            });
                            return Ok(AgentTurnResult {
                                events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                                outcome: AgentTurnOutcome::Suspended {
                                    approval_id,
                                    continuation: AgentContinuation {
                                        thread_id,
                                        turn_id: self.turn_id(user_message_id),
                                        invocation_id: self.invocation_id,
                                        user_message_id,
                                        workspace_root: workspace_root.clone(),
                                        context_summary,
                                        conversation,
                                        permission_mode,
                                        execution_authority: Some(ExecutionAuthority::new(
                                            workspace_root.clone(),
                                            permission_mode,
                                            turn_sandbox_config.clone(),
                                            self.capability_projection.clone(),
                                        )?),
                                        context_budget: budget,
                                        rollout_budget,
                                        model_context,
                                        collaboration_mode: self.collaboration_mode,
                                        goal: self.goal.clone(),
                                        state: AgentContinuationState::Provider {
                                            model_user_message,
                                            model_user_content,
                                            tool_candidates,
                                            provider_tool_calls,
                                            provider_tool_results,
                                            pending_tool_calls,
                                            compacted_tool_history,
                                            provider_response_items,
                                            model_rounds,
                                            rollout_reviews,
                                            runtime_state: runtime_state.clone(),
                                            branch_developer_instructions,
                                            provider_compatibility_hash: compatibility_hash,
                                        },
                                    },
                                },
                                provider_cursor: None,
                            });
                        }

                        let approved = review.approved();
                        let denied_by_policy = review.denied_by_policy();
                        let rationale = review.rationale;
                        let batch_call_count = batch.len();
                        for (index, item) in batch.iter().enumerate() {
                            let pending = pending_tool_calls.get(index).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "automatic approval batch lost provider call `{}`",
                                    item.call.id
                                )
                            })?;
                            anyhow::ensure!(
                                pending.id == item.call.id,
                                "automatic approval batch order mismatch: expected `{}`, found `{}`",
                                item.call.id,
                                pending.id
                            );
                        }
                        let batch_calls = batch
                            .iter()
                            .map(|item| item.call.clone())
                            .collect::<Vec<_>>();
                        let batch_results = if approved {
                            self.grant_turn_path_leases(
                                &mut runtime_state,
                                &batch_calls,
                                &workspace_root,
                            )?;
                            self.execute_scoped_approved_batch(
                                batch_calls,
                                &workspace_root,
                                permission_mode,
                                store.clone(),
                                cancellation.clone(),
                                thread_id,
                                self.turn_id(user_message_id),
                                "auto_review_batch",
                                events,
                            )
                            .await?
                        } else {
                            debug_assert!(denied_by_policy);
                            let results = batch_calls
                                .iter()
                                .map(|call| policy_denied_tool_result(call, &rationale))
                                .collect::<Vec<_>>();
                            for (call, result) in batch_calls.iter().zip(&results) {
                                record_provider_tool_result_event(
                                    events,
                                    ToolCall::new(&call.name, call.arguments.clone()),
                                    result,
                                );
                            }
                            results
                        };
                        for result in batch_results {
                            if let Some(ref mut budget) = budget {
                                budget
                                    .record_tokens(ContextBudget::estimate_tokens(&result.output));
                            }
                            provider_tool_results.push(result);
                        }
                        pending_tool_calls.drain(..batch_call_count);
                        continue;
                    }
                }

                let parallel_indices = if parallel_outcomes.is_empty() {
                    self.parallel_tool_call_indices_with_sandbox(
                        &pending_tool_calls,
                        &workspace_root,
                        permission_mode,
                        &turn_sandbox_config,
                    )
                } else {
                    Vec::new()
                };
                let starts_past_interactive_call =
                    parallel_indices.first().is_some_and(|index| *index > 0);
                if parallel_indices.len() >= 2 || starts_past_interactive_call {
                    let authority = ExecutionAuthority::new(
                        workspace_root.clone(),
                        permission_mode,
                        turn_sandbox_config.clone(),
                        self.capability_projection.clone(),
                    )?;
                    let mut base_ctx = authority.local_tool_context();
                    base_ctx.state = store.clone().map(ToolStateStore::new);
                    base_ctx.thread_id = Some(thread_id);
                    base_ctx.cancel = cancellation.clone();
                    base_ctx.browser = Some(self.tool_host.browser.clone());
                    base_ctx.computer = Some(self.tool_host.computer.clone());
                    self.apply_agent_context(&mut base_ctx, user_message_id);
                    base_ctx.fork_conversation = conversation.clone();
                    base_ctx.fork_conversation.push(ModelConversationMessage {
                        role: ModelConversationRole::User,
                        content: model_user_message.clone(),
                        content_parts: model_user_content.clone(),
                        tool_calls: Vec::new(),
                        tool_results: Vec::new(),
                    });
                    base_ctx.fork_model_context = Some(model_context.clone());
                    base_ctx.current_work_form = current_work_form_for_tool(&base_ctx, events)?;

                    let calls = parallel_indices
                        .into_iter()
                        .map(|index| pending_tool_calls[index].clone())
                        .collect::<Vec<_>>();
                    let runtime_catalog = self.tool_runtime_catalog();
                    let inputs = calls
                        .into_iter()
                        .map(
                            |provider_call| crate::tool_runtime::ProviderToolExecutionInput {
                                catalog: runtime_catalog.clone(),
                                provider_call,
                                user_message_id: self.turn_id(user_message_id),
                                agent_path: self.agent_path.clone(),
                                context: base_ctx.clone(),
                                background: self.tool_host.background.clone(),
                                turn_inbox: Arc::clone(&self.kernel.turn_inbox),
                            },
                        )
                        .collect();
                    let outcomes = self
                        .kernel
                        .tool_runtime
                        .execute_provider_batch(inputs)
                        .await;

                    for report in outcomes {
                        let provider_call = report.provider_call;
                        let result = report.outcome.into_result();
                        let local_events = TurnEvents::from_recorded(report.events);
                        anyhow::ensure!(
                            parallel_outcomes
                                .insert(provider_call.id.clone(), (result, local_events))
                                .is_none(),
                            "provider returned duplicate tool-call id `{}`",
                            provider_call.id
                        );
                    }
                    continue;
                }

                let provider_call = pending_tool_calls
                    .first()
                    .cloned()
                    .expect("non-empty pending tool-call queue");
                let authority = ExecutionAuthority::new(
                    workspace_root.clone(),
                    permission_mode,
                    turn_sandbox_config.clone(),
                    self.capability_projection.clone(),
                )?;
                let mut ctx = authority.local_tool_context();
                ctx.state = store.clone().map(ToolStateStore::new);
                ctx.thread_id = Some(thread_id);
                ctx.cancel = cancellation.clone();
                ctx.browser = Some(self.tool_host.browser.clone());
                ctx.computer = Some(self.tool_host.computer.clone());
                self.apply_agent_context(&mut ctx, user_message_id);
                ctx.fork_conversation = conversation.clone();
                ctx.fork_conversation.push(ModelConversationMessage {
                    role: ModelConversationRole::User,
                    content: model_user_message.clone(),
                    content_parts: model_user_content.clone(),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                });
                ctx.fork_model_context = Some(model_context.clone());
                match self
                    .execute_provider_tool_call(&provider_call, user_message_id, ctx, events)
                    .await
                {
                    Ok(result) => {
                        let user_input_request = result
                            .metadata
                            .get("userInputRequest")
                            .cloned()
                            .map(serde_json::from_value::<UserInputRequest>)
                            .transpose()?;
                        if let Some(ref mut budget) = budget {
                            budget.record_tokens(ContextBudget::estimate_tokens(&result.output));
                        }
                        if self.reveal_tools_from_search_result(&result, &mut tool_candidates) {
                            compatibility_hash = provider_compatibility_hash(
                                &model_context,
                                context_summary.as_deref(),
                                &tool_candidates,
                                branch_developer_instructions.as_deref(),
                            );
                        }
                        provider_tool_results.push(result);
                        pending_tool_calls.remove(0);
                        if let Some(request) = user_input_request {
                            events.push(AgentEventPayload::UserInputRequested {
                                request: request.clone(),
                            });
                            events.push(AgentEventPayload::TurnAwaitingInput {
                                request_id: request.request_id,
                            });
                            return Ok(AgentTurnResult {
                                events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                                outcome: AgentTurnOutcome::AwaitingInput {
                                    request,
                                    continuation: AgentContinuation {
                                        thread_id,
                                        turn_id: self.turn_id(user_message_id),
                                        invocation_id: self.invocation_id,
                                        user_message_id,
                                        workspace_root: workspace_root.clone(),
                                        context_summary,
                                        conversation,
                                        permission_mode,
                                        execution_authority: Some(ExecutionAuthority::new(
                                            workspace_root.clone(),
                                            permission_mode,
                                            turn_sandbox_config.clone(),
                                            self.capability_projection.clone(),
                                        )?),
                                        context_budget: budget,
                                        rollout_budget,
                                        model_context,
                                        collaboration_mode: self.collaboration_mode,
                                        goal: self.goal.clone(),
                                        state: AgentContinuationState::Provider {
                                            model_user_message,
                                            model_user_content,
                                            tool_candidates,
                                            provider_tool_calls,
                                            provider_tool_results,
                                            pending_tool_calls,
                                            compacted_tool_history,
                                            provider_response_items,
                                            model_rounds,
                                            rollout_reviews,
                                            runtime_state: runtime_state.clone(),
                                            branch_developer_instructions,
                                            provider_compatibility_hash: compatibility_hash,
                                        },
                                    },
                                },
                                provider_cursor: None,
                            });
                        }
                    }
                    Err(err) if browser_handoff_required(&err).is_some() => {
                        let handoff =
                            browser_handoff_required(&err).expect("browser handoff error guard");
                        events.push(AgentEventPayload::BrowserHandoffRequired {
                            action: handoff.action.clone(),
                            reason: handoff.reason.clone(),
                            url: handoff.url.clone(),
                        });
                        return Ok(AgentTurnResult {
                            events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                            outcome: AgentTurnOutcome::WaitingUserAction {
                                action: handoff.action.clone(),
                                reason: handoff.reason.clone(),
                                url: handoff.url.clone(),
                                continuation: AgentContinuation {
                                    thread_id,
                                    turn_id: self.turn_id(user_message_id),
                                    invocation_id: self.invocation_id,
                                    user_message_id,
                                    workspace_root: workspace_root.clone(),
                                    context_summary,
                                    conversation,
                                    permission_mode,
                                    execution_authority: Some(ExecutionAuthority::new(
                                        workspace_root.clone(),
                                        permission_mode,
                                        turn_sandbox_config.clone(),
                                        self.capability_projection.clone(),
                                    )?),
                                    context_budget: budget,
                                    rollout_budget,
                                    model_context,
                                    collaboration_mode: self.collaboration_mode,
                                    goal: self.goal.clone(),
                                    state: AgentContinuationState::Provider {
                                        model_user_message,
                                        model_user_content,
                                        tool_candidates,
                                        provider_tool_calls,
                                        provider_tool_results,
                                        pending_tool_calls,
                                        compacted_tool_history,
                                        provider_response_items,
                                        model_rounds,
                                        rollout_reviews,
                                        runtime_state: runtime_state.clone(),
                                        branch_developer_instructions,
                                        provider_compatibility_hash: compatibility_hash,
                                    },
                                },
                            },
                            provider_cursor: None,
                        });
                    }
                    Err(err) if approval_required(&err).is_some() => {
                        let reason = approval_required(&err)
                            .expect("approval error guard")
                            .reason()
                            .to_string();
                        if permission_mode.approvals_reviewer() == ApprovalsReviewer::AutoReview {
                            let action = GuardianApprovalAction::from_provider_call(
                                &provider_call,
                                &workspace_root,
                            );
                            if let Some(reviewability_error) = action.reviewability_error() {
                                let result = unreviewable_action_result(
                                    &provider_call,
                                    &reviewability_error,
                                );
                                if let Some(ref mut budget) = budget {
                                    budget.record_tokens(ContextBudget::estimate_tokens(
                                        &result.output,
                                    ));
                                }
                                provider_tool_results.push(result);
                                pending_tool_calls.remove(0);
                                continue;
                            }
                            let request = GuardianApprovalRequest::new(
                                thread_id,
                                user_message_id,
                                reason.clone(),
                                action,
                            );
                            let action_summary = request.action.event_summary();
                            events.push(AgentEventPayload::AutomaticApprovalReviewStarted {
                                review_id: request.review_id,
                                target_item_id: provider_call.id.clone(),
                                action: action_summary.clone(),
                            });
                            let review = self
                                .kernel
                                .tool_runtime
                                .review(
                                    ToolReviewInput {
                                        guardian: &self.guardian,
                                        request: &request,
                                        conversation: &conversation,
                                        current_user_message: &model_user_message,
                                        tool_calls: &provider_tool_calls,
                                        tool_results: &provider_tool_results,
                                        workspace_root: &workspace_root,
                                        sandbox_config: &turn_sandbox_config,
                                    },
                                    cancellation.as_ref(),
                                )
                                .await;
                            let risk_level =
                                review.assessment.as_ref().map(|value| value.risk_level);
                            let user_authorization = review
                                .assessment
                                .as_ref()
                                .map(|value| value.user_authorization);
                            events.push(AgentEventPayload::AutomaticApprovalReviewCompleted {
                                review_id: request.review_id,
                                target_item_id: provider_call.id.clone(),
                                status: review.status,
                                risk_level,
                                user_authorization,
                                rationale: review.rationale.clone(),
                                action: action_summary,
                                usage: review.usage.clone(),
                                attempts: review.attempts,
                                tool_rounds: review.tool_rounds,
                                decision_source: review.decision_source,
                                failure_kind: review.failure_kind,
                            });
                            if review.status == GuardianReviewStatus::Aborted {
                                anyhow::bail!("cancelled");
                            }
                            if review.technical_failure() {
                                return Ok(finalize_automatic_review_failure_turn(
                                    thread_id,
                                    review.status,
                                    review.rationale,
                                    std::mem::replace(events, TurnEvents::new(None)),
                                ));
                            }
                            if let Some(message) = review.interrupt_turn {
                                events.push(AgentEventPayload::AutoReviewInterruptionWarning {
                                    message: message.clone(),
                                });
                                anyhow::bail!(message);
                            }

                            if review.needs_user_approval() {
                                let approval_id = Uuid::new_v4();
                                let approval_reason = format!(
                                    "automatic reviewer requires user approval: {}",
                                    review.rationale
                                );
                                events.push(AgentEventPayload::ApprovalRequested {
                                    approval_id,
                                    reason: approval_reason.clone(),
                                    action: provider_tool_approval_action(&provider_call),
                                });
                                events.push(AgentEventPayload::TurnSuspended {
                                    approval_id,
                                    reason: approval_reason,
                                });
                                return Ok(AgentTurnResult {
                                    events: std::mem::replace(events, TurnEvents::new(None))
                                        .into_vec(),
                                    outcome: AgentTurnOutcome::Suspended {
                                        approval_id,
                                        continuation: AgentContinuation {
                                            thread_id,
                                            turn_id: self.turn_id(user_message_id),
                                            invocation_id: self.invocation_id,
                                            user_message_id,
                                            workspace_root: workspace_root.clone(),
                                            context_summary,
                                            conversation,
                                            permission_mode,
                                            execution_authority: Some(ExecutionAuthority::new(
                                                workspace_root.clone(),
                                                permission_mode,
                                                turn_sandbox_config.clone(),
                                                self.capability_projection.clone(),
                                            )?),
                                            context_budget: budget,
                                            rollout_budget,
                                            model_context,
                                            collaboration_mode: self.collaboration_mode,
                                            goal: self.goal.clone(),
                                            state: AgentContinuationState::Provider {
                                                model_user_message,
                                                model_user_content,
                                                tool_candidates,
                                                provider_tool_calls,
                                                provider_tool_results,
                                                pending_tool_calls,
                                                compacted_tool_history,
                                                provider_response_items,
                                                model_rounds,
                                                rollout_reviews,
                                                runtime_state: runtime_state.clone(),
                                                branch_developer_instructions,
                                                provider_compatibility_hash: compatibility_hash,
                                            },
                                        },
                                    },
                                    provider_cursor: None,
                                });
                            }

                            let result = if review.approved() {
                                self.grant_turn_path_leases(
                                    &mut runtime_state,
                                    std::slice::from_ref(&provider_call),
                                    &workspace_root,
                                )?;
                                self.execute_scoped_approved_call(
                                    &provider_call,
                                    &workspace_root,
                                    permission_mode,
                                    store.clone(),
                                    cancellation.clone(),
                                    thread_id,
                                    user_message_id,
                                    "auto_review",
                                    events,
                                )
                                .await?
                            } else {
                                debug_assert!(review.denied_by_policy());
                                let result =
                                    policy_denied_tool_result(&provider_call, &review.rationale);
                                record_provider_tool_result_event(
                                    events,
                                    ToolCall::new(
                                        &provider_call.name,
                                        provider_call.arguments.clone(),
                                    ),
                                    &result,
                                );
                                result
                            };
                            if let Some(ref mut budget) = budget {
                                budget
                                    .record_tokens(ContextBudget::estimate_tokens(&result.output));
                            }
                            provider_tool_results.push(result);
                            pending_tool_calls.remove(0);
                            continue;
                        }

                        let approval_id = Uuid::new_v4();
                        events.push(AgentEventPayload::ApprovalRequested {
                            approval_id,
                            reason: format!("approval required: {reason}"),
                            action: provider_tool_approval_action(&provider_call),
                        });
                        events.push(AgentEventPayload::TurnSuspended {
                            approval_id,
                            reason: format!("approval required: {reason}"),
                        });
                        return Ok(AgentTurnResult {
                            events: std::mem::replace(events, TurnEvents::new(None)).into_vec(),
                            outcome: AgentTurnOutcome::Suspended {
                                approval_id,
                                continuation: AgentContinuation {
                                    thread_id,
                                    turn_id: self.turn_id(user_message_id),
                                    invocation_id: self.invocation_id,
                                    user_message_id,
                                    workspace_root: workspace_root.clone(),
                                    context_summary,
                                    conversation,
                                    permission_mode,
                                    execution_authority: Some(ExecutionAuthority::new(
                                        workspace_root.clone(),
                                        permission_mode,
                                        turn_sandbox_config.clone(),
                                        self.capability_projection.clone(),
                                    )?),
                                    context_budget: budget,
                                    rollout_budget,
                                    model_context,
                                    collaboration_mode: self.collaboration_mode,
                                    goal: self.goal.clone(),
                                    state: AgentContinuationState::Provider {
                                        model_user_message,
                                        model_user_content,
                                        tool_candidates,
                                        provider_tool_calls,
                                        provider_tool_results,
                                        pending_tool_calls,
                                        compacted_tool_history,
                                        provider_response_items,
                                        model_rounds,
                                        rollout_reviews,
                                        runtime_state: runtime_state.clone(),
                                        branch_developer_instructions,
                                        provider_compatibility_hash: compatibility_hash,
                                    },
                                },
                            },
                            provider_cursor: None,
                        });
                    }
                    Err(err) => return Err(err),
                }
            }

            match self
                .complete_provider_round(
                    thread_id,
                    user_message_id,
                    context_summary.as_deref(),
                    &mut conversation,
                    &mut budget,
                    &mut rollout_budget,
                    model_rounds,
                    rollout_reviews,
                    &mut runtime_state,
                    &model_context,
                    store.as_ref(),
                    cancellation.as_ref(),
                    &model_user_message,
                    &model_user_content,
                    &tool_candidates,
                    &mut provider_tool_calls,
                    &mut provider_tool_results,
                    &mut pending_tool_calls,
                    &mut compacted_tool_history,
                    &mut provider_response_items,
                    branch_developer_instructions.as_deref(),
                    &compatibility_hash,
                    &mut completion_guard_delivery,
                    events,
                )
                .await?
            {
                ProviderRoundOutcome::Continue {
                    model_rounds: completed_rounds,
                    rollout_reviews: completed_reviews,
                } => {
                    model_rounds = completed_rounds;
                    rollout_reviews = completed_reviews;
                }
                ProviderRoundOutcome::Finished(result) => return Ok(result),
            }
        }
    }
}
