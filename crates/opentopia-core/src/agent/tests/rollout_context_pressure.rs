#[tokio::test]
async fn rollout_checkpoint_is_delivered_to_the_main_model_without_a_reviewer_call() {
    let workspace = test_workspace("rollout-self-review-checkpoint");
    let mut responses = (1..=ROLLOUT_REVIEW_INTERVAL)
        .map(rollout_tool_response)
        .collect::<Vec<_>>();
    responses.push(ModelResponse::text(
        "I reviewed the original request and current evidence myself, then completed the task.",
    ));
    let provider = Arc::new(ScriptedProvider::new(responses));
    let reviewer = Arc::new(ScriptedProvider::new(Vec::new()));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer.clone());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect and finish when the evidence is sufficient.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("main-model self-review completes");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(reviewer.requests().is_empty());
    let requests = provider.requests();
    assert_eq!(requests.len(), ROLLOUT_REVIEW_INTERVAL + 1);
    let checkpoint = requests[ROLLOUT_REVIEW_INTERVAL]
        .input
        .tool_results
        .iter()
        .find(|result| result.name == ROLLOUT_CHECKPOINT_TOOL_NAME)
        .expect("the objective checkpoint reaches the main model");
    assert!(checkpoint.output.contains("self_review_required"));
    assert!(checkpoint.output.contains("\"decision\": null"));
    assert!(checkpoint
        .output
        .contains("runtime has not made a progress judgement"));
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ContextWarning { stage, message }
            if stage == "rollout_self_review_checkpoint"
                && message.contains("without making a progress decision")
    )));
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ModelRequest { round, .. }
            if *round == ROLLOUT_REVIEW_INTERVAL + 1
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn rollout_never_starts_a_main_model_round_after_two_hundred_seventy() {
    let workspace = test_workspace("rollout-hard-limit");
    let provider = Arc::new(ScriptedProvider::new(
        (1..=MAX_ROLLOUT_MODEL_ROUNDS)
            .map(rollout_tool_response)
            .collect(),
    ));
    let reviewer = Arc::new(ScriptedProvider::new(Vec::new()));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer.clone());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue until the work is complete or a resource limit is reached."
                    .to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("hard-limit stop is a structured turn result");

    assert!(matches!(
        &result.outcome,
        AgentTurnOutcome::Stopped { reason } if reason.contains("hard limit")
    ));
    assert_eq!(provider.requests().len(), MAX_ROLLOUT_MODEL_ROUNDS);
    assert!(reviewer.requests().is_empty());
    assert!(assistant_text(&result.events).contains("hard limit of 270"));
    assert!(!result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ModelRequest { round, .. }
            if *round > MAX_ROLLOUT_MODEL_ROUNDS
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn opening_round_uses_the_same_context_pressure_admission_boundary() {
    #[derive(Debug)]
    struct RecordingCompactor {
        rounds: Arc<std::sync::Mutex<Vec<usize>>>,
    }

    #[async_trait]
    impl crate::round_compaction::RoundContextCompactor for RecordingCompactor {
        async fn compact(
            &self,
            request: crate::round_compaction::RoundContextCompactionRequest,
        ) -> anyhow::Result<crate::round_compaction::RoundContextCompactionResult> {
            self.rounds.lock().unwrap().push(request.round);
            let checkpoint = ContextCheckpoint::manual(
                request.thread_id,
                ContextCheckpointCoverage::default(),
                "Earlier conversation is covered by the durable opening-round checkpoint.",
            );
            let rendered = serde_json::to_string_pretty(&checkpoint)?;
            let mut summary = ContextSummary::new(request.thread_id, 0, 0, rendered);
            summary.checkpoint = Some(checkpoint);
            Ok(crate::round_compaction::RoundContextCompactionResult {
                summary,
                details: None,
                covered_tool_call_ids: Default::default(),
            })
        }
    }

    let message = |role, content: String| ModelConversationMessage {
        role,
        content,
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    };
    let conversation = vec![
        message(ModelConversationRole::User, "old user ".repeat(1_500)),
        message(
            ModelConversationRole::Assistant,
            "old assistant ".repeat(1_500),
        ),
        message(ModelConversationRole::User, "newer user ".repeat(1_500)),
        message(
            ModelConversationRole::Assistant,
            "newer assistant ".repeat(1_500),
        ),
    ];
    let original_message_count = conversation.len();
    let rounds = Arc::new(std::sync::Mutex::new(Vec::new()));
    let workspace = test_workspace("opening-round-context-admission");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "Opening round completed after context admission.",
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_round_context_compactor(Arc::new(RecordingCompactor {
            rounds: Arc::clone(&rounds),
        }));

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue from the durable history.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation,
                permission_mode: PermissionMode::FullAccess,
                context_budget: Some({
                    let mut budget = ContextBudget::new(4_096);
                    budget.set_last_provider_total_tokens(3_500);
                    budget
                }),
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("opening round is admitted after compaction");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(*rounds.lock().unwrap(), vec![0]);
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].input.conversation.len() < original_message_count);
    assert!(requests[0]
        .instructions
        .items
        .iter()
        .any(|item| item.kind == ContextItemKind::Checkpoint));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn opening_round_ignores_local_estimates_below_the_last_provider_total_threshold() {
    let workspace = test_workspace("opening-round-authoritative-usage");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "The provider round ran without premature compaction.",
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
    let mut budget = ContextBudget::new(4_096);
    budget.set_last_provider_total_tokens(2_300);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue from the large history.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: vec![ModelConversationMessage {
                    role: ModelConversationRole::User,
                    content: "large locally estimated history ".repeat(2_000),
                    content_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                }],
                permission_mode: PermissionMode::FullAccess,
                context_budget: Some(budget),
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("a 56% provider total must not trigger local-estimate compaction");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(provider.requests().len(), 1);
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ContextCompacted { .. })));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn failed_context_compaction_blocks_the_provider_round() {
    #[derive(Debug)]
    struct FailingCompactor {
        attempts: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl crate::round_compaction::RoundContextCompactor for FailingCompactor {
        async fn compact(
            &self,
            request: crate::round_compaction::RoundContextCompactionRequest,
        ) -> anyhow::Result<crate::round_compaction::RoundContextCompactionResult> {
            assert!(
                request.event_sender.is_some(),
                "a live turn must give the compactor its ordered event channel"
            );
            self.attempts.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("checkpoint response is not JSON")
        }
    }

    let workspace = test_workspace("failed-context-compaction-barrier");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "This response must never be requested.",
    )]));
    let attempts = Arc::new(AtomicUsize::new(0));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_round_context_compactor(Arc::new(FailingCompactor {
            attempts: Arc::clone(&attempts),
        }));
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();

    let error = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue only after durable context admission.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: vec![ModelConversationMessage {
                    role: ModelConversationRole::User,
                    content: "uncompressed durable history ".repeat(2_000),
                    content_parts: Vec::new(),
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                }],
                permission_mode: PermissionMode::FullAccess,
                context_budget: Some({
                    let mut budget = ContextBudget::new(4_096);
                    budget.set_last_provider_total_tokens(3_500);
                    budget
                }),
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            Some(sender),
        )
        .await
        .expect_err("a failed compaction must stop before the provider round");

    assert!(error
        .to_string()
        .contains("Durable round context compaction failed"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(
        provider.requests().is_empty(),
        "the uncompressed request must never reach the normal provider"
    );
    let mut streamed = Vec::new();
    while let Ok(payload) = receiver.try_recv() {
        streamed.push(payload);
    }
    assert!(streamed.iter().any(|payload| matches!(
        payload,
        AgentEventPayload::ContextWarning { stage, message }
            if stage == "round_context_compaction"
                && message.contains("checkpoint response is not JSON")
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn opening_round_provider_overflow_uses_the_same_durable_recovery_path() {
    struct OverflowOnceProvider {
        requests: std::sync::Mutex<Vec<ModelRequest>>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ModelProvider for OverflowOnceProvider {
        async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
            self.requests.lock().unwrap().push(request);
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                anyhow::bail!(
                    "context_length_exceeded: provider estimate exceeded its context window"
                );
            }
            Ok(ModelResponse::text("Recovered opening round."))
        }

        async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
            Ok(ProviderHealthCheck {
                reachable: true,
                latency_ms: None,
                model_available: true,
                error: None,
                openai_compatibility: None,
            })
        }
    }

    #[derive(Debug)]
    struct RecoveryCompactor {
        rounds: Arc<std::sync::Mutex<Vec<usize>>>,
    }

    #[async_trait]
    impl crate::round_compaction::RoundContextCompactor for RecoveryCompactor {
        async fn compact(
            &self,
            request: crate::round_compaction::RoundContextCompactionRequest,
        ) -> anyhow::Result<crate::round_compaction::RoundContextCompactionResult> {
            self.rounds.lock().unwrap().push(request.round);
            let checkpoint = ContextCheckpoint::manual(
                request.thread_id,
                ContextCheckpointCoverage::default(),
                "Durable history is covered for overflow recovery.",
            );
            let rendered = serde_json::to_string_pretty(&checkpoint)?;
            let mut summary = ContextSummary::new(request.thread_id, 0, 0, rendered);
            summary.checkpoint = Some(checkpoint);
            Ok(crate::round_compaction::RoundContextCompactionResult {
                summary,
                details: None,
                covered_tool_call_ids: Default::default(),
            })
        }
    }

    let workspace = test_workspace("opening-round-overflow-recovery");
    let rounds = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = Arc::new(OverflowOnceProvider {
        requests: std::sync::Mutex::new(Vec::new()),
        calls: AtomicUsize::new(0),
    });
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_round_context_compactor(Arc::new(RecoveryCompactor {
            rounds: Arc::clone(&rounds),
        }));
    let conversation = vec![ModelConversationMessage {
        role: ModelConversationRole::User,
        content: "durable prior history ".repeat(1_000),
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    }];

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation,
                permission_mode: PermissionMode::FullAccess,
                context_budget: Some(ContextBudget::new(100_000)),
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("opening round recovers from provider overflow");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(*rounds.lock().unwrap(), vec![0]);
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].input.conversation.is_empty());
    assert!(requests[1].input.conversation.is_empty());
    assert!(requests[1].previous_response_id.is_none());
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ContextWarning { stage, .. }
            if stage == "provider_context_overflow_recovery"
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn context_pressure_counts_typed_tool_content_and_preserves_sub_threshold_history() {
    let result = ProviderToolResult {
        call_id: "call_json".to_string(),
        name: "spreadsheet".to_string(),
        output: "bounded".to_string(),
        content: vec![ModelContentPart::json(json!({
            "rows": (0..500).map(|row| format!("row-{row}-{}", "x".repeat(40))).collect::<Vec<_>>()
        }))],
        is_error: false,
        metadata: json!({ "success": true }),
    };
    let request = ModelRequest {
        instructions: CompiledModelContext {
            items: Vec::new(),
            prompt_cache_key: Some("stable-lineage".to_string()),
        },
        input: ModelInputLedger {
            current_user: ModelUserInput {
                message: "continue".to_string(),
                content: Vec::new(),
            },
            tool_results: vec![result.clone()],
            ..Default::default()
        },
        tool_candidates: Vec::new(),
        previous_response_items: Vec::new(),
        provider_transcript: None,
        previous_response_id: None,
        prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::AppendOnlyUsers,
        final_output_json_schema: None,
    };
    let mut budget = Some(ContextBudget::new(100_000));
    synchronize_context_budget(&mut budget, &request);
    assert!(budget.as_ref().unwrap().used_tokens > 1_000);

    let mut below_threshold = ContextBudget::new(10_000);
    below_threshold.used_tokens = 99_999;
    assert!(!below_threshold.requires_compaction(80));
    below_threshold.set_last_provider_total_tokens(7_999);
    assert!(!below_threshold.requires_compaction(80));
    below_threshold.set_round_pressure(9_999);
    assert!(!below_threshold.requires_compaction(80));
    below_threshold.set_last_provider_total_tokens(8_000);
    assert!(below_threshold.requires_compaction(80));
}

#[test]
fn provider_context_overflow_detection_is_specific() {
    assert!(provider_context_window_exceeded(&anyhow::anyhow!(
        "context_length_exceeded: maximum context length is 128000"
    )));
    assert!(provider_context_window_exceeded(&anyhow::anyhow!(
        "prompt is too long"
    )));
    assert!(!provider_context_window_exceeded(&anyhow::anyhow!(
        "provider returned 429 rate limit"
    )));
}

#[tokio::test]
async fn rollout_budget_applies_to_a_final_provider_response() {
    let workspace = test_workspace("rollout-budget-final-response");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: "This response crosses the configured budget.".to_string(),
        tool_calls: Vec::new(),
        usage: Some(ModelUsage {
            input_tokens: 20,
            output_tokens: 80,
            total_tokens: 100,
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        }),
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Stop,
    }]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_rollout_budget_settings(RolloutBudgetSettings {
            limit_tokens: 100,
            sampling_token_weight: 1.0,
            prefill_token_weight: 1.0,
        });

    let error = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Answer directly.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect_err("final responses count toward the rollout budget");

    assert!(error
        .to_string()
        .contains("shared rollout token budget exhausted"));
    assert_eq!(provider.requests().len(), 1);

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn rollout_budget_reminder_is_injected_before_final_provider_round() {
    let workspace = test_workspace("rollout-budget-reminder");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_list".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "list", "path": "." }),
            }],
            usage: Some(ModelUsage {
                input_tokens: 0,
                output_tokens: 80,
                total_tokens: 80,
                cached_input_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            }),
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Workspace inspection is complete."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_rollout_budget_settings(RolloutBudgetSettings {
            limit_tokens: 100,
            sampling_token_weight: 1.0,
            prefill_token_weight: 1.0,
        });

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect the workspace.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("budget reminder leaves enough room for final output");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1]
        .input
        .tool_results
        .iter()
        .any(|result| result.output.contains("[Rollout budget]")
            && result.output.contains("20 weighted tokens")));

    let _ = fs::remove_dir_all(workspace);
}
