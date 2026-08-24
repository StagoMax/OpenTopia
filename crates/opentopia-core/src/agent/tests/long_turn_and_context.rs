#[tokio::test]
async fn provider_tool_loop_supports_multiple_rounds() {
    let workspace = test_workspace("provider-multi-tool-loop");
    fs::write(workspace.join("first.txt"), "first result").unwrap();
    fs::write(workspace.join("second.txt"), "second result").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_first".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read", "path": "first.txt" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_second".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read", "path": "second.txt" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Both files were inspected."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Inspect both files.".to_string(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: None,
            cancellation: None,
        })
        .await
        .expect("turn succeeds");

    assert!(assistant_text(&events).contains("Both files were inspected."));
    let request_ids = events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ProviderRequestSent { request_id, .. } => Some(*request_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let first_token_request_ids = events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ProviderFirstTokenReceived { request_id } => Some(*request_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(first_token_request_ids, request_ids);
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].input.tool_calls.len(), 2);
    assert_eq!(requests[2].input.tool_results.len(), 2);
    assert!(requests[2]
        .tool_candidates
        .iter()
        .any(|tool| tool.name == "filesystem"));
    assert!(requests[2].input.tool_results[0]
        .output
        .contains("first result"));
    assert!(requests[2].input.tool_results[1]
        .output
        .contains("second result"));
    assert_eq!(
        serde_json::to_value(&requests[1].input.tool_results[0]).unwrap(),
        serde_json::to_value(&requests[2].input.tool_results[0]).unwrap(),
        "a previously exposed tool result must remain byte-stable in later rounds"
    );
    assert_eq!(
        requests[1].input.tool_results[0].metadata["toolResultEnvelope"]["stage"],
        "pre_model_ingress"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn eight_tool_rounds_continue_without_approval() {
    let workspace = test_workspace("eight-tool-rounds");
    for index in 0..8 {
        fs::write(workspace.join(format!("sample-{index}.txt")), "content").unwrap();
    }
    let tool_responses = (0..8)
        .map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_{index}"),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "read",
                    "path": format!("sample-{index}.txt")
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        })
        .collect::<Vec<_>>();
    let provider = Arc::new(ScriptedProvider::new(
        tool_responses
            .into_iter()
            .chain(std::iter::once(ModelResponse::text(
                "Completed all eight distinct observations without a checkpoint.",
            )))
            .collect(),
    ));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect sample.txt until the work is complete.".to_string(),
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
        .expect("turn continues without a checkpoint");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(assistant_text(&result.events).contains("without a checkpoint"));
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    assert_eq!(provider.requests().len(), 9);
    assert!(provider.requests()[8]
        .instructions
        .instructions()
        .contains("hard resource ceiling of 270 main-model rounds"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn more_than_twenty_four_distinct_tool_rounds_can_complete() {
    let workspace = test_workspace("unbounded-tool-rounds");
    for index in 0..30 {
        fs::write(workspace.join(format!("sample-{index}.txt")), "content").unwrap();
    }
    let responses = (0..30)
        .map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_{index}"),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "read",
                    "path": format!("sample-{index}.txt")
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        })
        .chain(std::iter::once(ModelResponse::text(
            "Completed after thirty distinct tool rounds.",
        )))
        .collect();
    let provider = Arc::new(ScriptedProvider::new(responses));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect all thirty distinct inputs.".to_string(),
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
        .expect("long turn completes without continuation");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(assistant_text(&result.events).contains("thirty distinct tool rounds"));
    let requests = provider.requests();
    assert_eq!(requests.len(), 31);
    let final_request = requests.last().expect("final provider request");
    assert!(!final_request.tool_candidates.is_empty());
    assert!(final_request
        .instructions
        .instructions()
        .contains("hard resource ceiling of 270 main-model rounds"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn long_turn_replaces_covered_tool_history_with_a_durable_checkpoint() {
    let workspace = test_workspace("durable-round-context-compaction");
    for index in 0..10 {
        fs::write(
            workspace.join(format!("large-{index}.txt")),
            format!("record-{index}-{}", "x".repeat(2_000)),
        )
        .unwrap();
    }
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: (0..10)
                .map(|index| ProviderToolCall {
                    id: format!("call_{index}"),
                    name: "filesystem".to_string(),
                    arguments: json!({
                        "operation": "read",
                        "path": format!("large-{index}.txt")
                    }),
                })
                .collect(),
            usage: Some(ModelUsage {
                input_tokens: 3_400,
                output_tokens: 100,
                total_tokens: 3_500,
                cached_input_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            }),
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Completed after automatic context maintenance."),
    ]));
    #[derive(Debug)]
    struct DurableTestCompactor;

    #[async_trait]
    impl crate::round_compaction::RoundContextCompactor for DurableTestCompactor {
        async fn compact(
            &self,
            request: crate::round_compaction::RoundContextCompactionRequest,
        ) -> anyhow::Result<crate::round_compaction::RoundContextCompactionResult> {
            let checkpoint = ContextCheckpoint::manual(
                request.thread_id,
                ContextCheckpointCoverage::default(),
                "Inspect all large records; completed filesystem reads are covered by this durable checkpoint.",
            );
            let rendered = serde_json::to_string_pretty(&checkpoint)?;
            let mut summary = ContextSummary::new(request.thread_id, 0, 0, rendered);
            summary.checkpoint = Some(checkpoint);
            Ok(crate::round_compaction::RoundContextCompactionResult {
                summary,
                details: None,
                covered_tool_call_ids: (0..10).map(|index| format!("call_{index}")).collect(),
            })
        }
    }

    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_round_context_compactor(Arc::new(DurableTestCompactor));

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect all large records.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: Some(ContextBudget::new(4_096)),
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("round pressure creates a durable checkpoint");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.tool_calls.is_empty());
    assert!(requests[1].input.tool_results.is_empty());
    assert!(requests[1]
        .instructions
        .items
        .iter()
        .any(|item| item.kind == ContextItemKind::Checkpoint));
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ContextCompacted { .. })));
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn provider_request_includes_durable_context_summary() {
    let workspace = test_workspace("provider-durable-context");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "Continued from durable context.",
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Continue the implementation.".to_string(),
            user_content: Vec::new(),
            context_summary: Some("Decision: keep the Rust sidecar API stable.".to_string()),
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: None,
            cancellation: None,
        })
        .await
        .expect("turn succeeds");

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].input.current_user.message,
        "Continue the implementation."
    );
    let checkpoint_items = requests[0]
        .instructions
        .items
        .iter()
        .filter(|item| {
            item.text_content()
                .contains("keep the Rust sidecar API stable")
        })
        .collect::<Vec<_>>();
    assert_eq!(checkpoint_items.len(), 1);
    assert_eq!(checkpoint_items[0].kind, ContextItemKind::Checkpoint);
    assert_eq!(checkpoint_items[0].role, ContextRole::Developer);
    assert_eq!(checkpoint_items[0].cache_scope, ContextCacheScope::Thread);

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn provider_cursor_is_used_only_for_a_compatible_request_prefix() {
    let workspace = test_workspace("provider-state-cursor");
    let sandbox = LocalSandboxConfig::danger_full_access();
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: "Continued from the stored response.".to_string(),
        tool_calls: Vec::new(),
        usage: None,
        response_id: Some("resp_next".to_string()),
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Stop,
    }]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(sandbox.clone());
    let base_model_context = agent_model_context_with_runtime(
        &workspace,
        &sandbox,
        &agent.agent_runtime_settings,
        agent.prompt_runtime_capabilities(RuntimeSurface::Core),
    );
    let tool_candidates = agent.provider_tool_candidates();
    let model_context = DefaultContextAssembler
        .prepare_context(ContextPreparationInput {
            model_context: &base_model_context,
            context_summary: None,
            tool_candidates: &tool_candidates,
            lineage_instructions: None,
        })
        .expect("prepare context");
    let compatibility_hash =
        provider_compatibility_hash(&model_context, None, &tool_candidates, None);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: Some(ProviderConversationCursor {
                    response_id: "resp_previous".to_string(),
                    compatibility_hash: compatibility_hash.clone(),
                    response_items: Vec::new(),
                    state_kind: ProviderContextStateKind::StoredResponse,
                    compaction_item_count: 0,
                }),
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("turn succeeds");

    assert_eq!(
        provider.requests()[0].previous_response_id.as_deref(),
        Some("resp_previous")
    );
    assert_eq!(
        result.provider_cursor,
        Some(ProviderConversationCursor {
            response_id: "resp_next".to_string(),
            compatibility_hash,
            response_items: Vec::new(),
            state_kind: ProviderContextStateKind::StoredResponse,
            compaction_item_count: 0,
        })
    );

    let incompatible_provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "Used local replay.",
    )]));
    let incompatible_agent =
        AgentCore::new(incompatible_provider.clone(), ToolRegistry::with_builtins())
            .with_sandbox_config(sandbox);
    incompatible_agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Continue with changed context.".to_string(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: Some(ProviderConversationCursor {
                response_id: "resp_stale".to_string(),
                compatibility_hash: "stale".to_string(),
                response_items: Vec::new(),
                state_kind: ProviderContextStateKind::StoredResponse,
                compaction_item_count: 0,
            }),
            store: None,
            cancellation: None,
        })
        .await
        .expect("incompatible cursor falls back to replay");
    assert!(incompatible_provider.requests()[0]
        .previous_response_id
        .is_none());

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn stateless_provider_cursor_replays_only_opaque_context_items() {
    let workspace = test_workspace("provider-state-items");
    let sandbox = LocalSandboxConfig::danger_full_access();
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: "Continued from opaque state.".to_string(),
        tool_calls: Vec::new(),
        usage: None,
        response_id: None,
        provider_items: vec![
            json!({ "type": "compaction", "id": "cmp_next", "encrypted_content": "opaque" }),
            json!({ "type": "reasoning", "id": "rs_next", "encrypted_content": "opaque" }),
            json!({ "type": "message", "id": "msg_next", "content": [] }),
        ],
        finish_reason: ModelFinishReason::Stop,
    }]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(sandbox.clone());
    let base_model_context = agent_model_context_with_runtime(
        &workspace,
        &sandbox,
        &agent.agent_runtime_settings,
        agent.prompt_runtime_capabilities(RuntimeSurface::Core),
    );
    let tool_candidates = agent.provider_tool_candidates();
    let model_context = DefaultContextAssembler
        .prepare_context(ContextPreparationInput {
            model_context: &base_model_context,
            context_summary: None,
            tool_candidates: &tool_candidates,
            lineage_instructions: None,
        })
        .expect("prepare context");
    let compatibility_hash =
        provider_compatibility_hash(&model_context, None, &tool_candidates, None);
    let previous_item = json!({
        "type": "compaction",
        "id": "cmp_previous",
        "encrypted_content": "opaque"
    });

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Continue.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: Some(ProviderConversationCursor {
                    response_id: String::new(),
                    compatibility_hash,
                    response_items: vec![previous_item.clone()],
                    state_kind: ProviderContextStateKind::CompactionItems,
                    compaction_item_count: 1,
                }),
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("turn succeeds");

    let request = &provider.requests()[0];
    assert!(request.previous_response_id.is_none());
    assert_eq!(request.previous_response_items, vec![previous_item]);
    let cursor = result.provider_cursor.expect("opaque cursor is retained");
    assert_eq!(cursor.state_kind, ProviderContextStateKind::CompactionItems);
    assert_eq!(cursor.compaction_item_count, 1);
    assert_eq!(cursor.response_items.len(), 2);
    assert!(cursor
        .response_items
        .iter()
        .all(|item| item.get("type").and_then(Value::as_str) != Some("message")));
    assert!(cursor
        .response_items
        .iter()
        .any(|item| { item.get("id").and_then(Value::as_str) == Some("cmp_next") }));
    assert!(!cursor
        .response_items
        .iter()
        .any(|item| { item.get("id").and_then(Value::as_str) == Some("cmp_previous") }));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn opaque_provider_state_survives_until_a_new_compaction_supersedes_it() {
    let retained = replayable_provider_state_items(&[
        json!({ "type": "compaction", "id": "cmp_old" }),
        json!({ "type": "reasoning", "id": "reasoning_new", "encrypted_content": "opaque" }),
    ]);
    assert_eq!(retained.len(), 2);

    let superseded = replayable_provider_state_items(&[
        json!({ "type": "compaction", "id": "cmp_old" }),
        json!({ "type": "reasoning", "id": "reasoning_old", "encrypted_content": "opaque" }),
        json!({ "type": "compaction", "id": "cmp_new" }),
        json!({ "type": "reasoning", "id": "reasoning_new", "encrypted_content": "opaque" }),
    ]);
    assert_eq!(superseded.len(), 2);
    assert_eq!(
        superseded[0].get("id").and_then(Value::as_str),
        Some("cmp_new")
    );
}

#[test]
fn chat_assistant_state_is_replayable_across_turns() {
    let state = json!({
        "type": "openai_chat_assistant_state",
        "content": "",
        "reasoning_content": "",
        "tool_call_ids": ["call_a", "call_b"],
    });

    assert_eq!(
        replayable_provider_state_items(std::slice::from_ref(&state)),
        vec![state]
    );
}

#[test]
fn completed_wire_transcript_supersedes_redundant_chat_assistant_state() {
    let old_transcript = ProviderWireTranscript {
        format: "openai_chat_native_messages_v1".to_string(),
        items: vec![json!({ "role": "user", "content": "old" })],
    };
    let completed_transcript = ProviderWireTranscript {
        format: old_transcript.format.clone(),
        items: vec![
            json!({ "role": "user", "content": "old" }),
            json!({ "role": "assistant", "content": "done" }),
        ],
    };
    let retained = replayable_provider_state_items(&[
        provider_transcript_state_item(&old_transcript),
        json!({
            "type": "openai_chat_assistant_state",
            "content": "",
            "tool_call_ids": ["call_old"],
        }),
        provider_transcript_candidate_item(&completed_transcript),
    ]);

    assert_eq!(retained.len(), 1);
    assert_eq!(
        provider_wire_transcript(&retained[0]),
        Some(completed_transcript)
    );
    assert_eq!(
        retained[0].get("type").and_then(Value::as_str),
        Some(PROVIDER_TRANSCRIPT_STATE_TYPE)
    );
}

#[tokio::test]
async fn provider_request_does_not_prefetch_workspace_listing() {
    let workspace = test_workspace("no-workspace-preflight");
    fs::write(workspace.join("private.txt"), "workspace marker").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "No workspace inspection was required.",
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Explain the available tools.".to_string(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: None,
            cancellation: None,
        })
        .await
        .expect("turn succeeds");

    assert!(!events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { call }
            if call.name == "filesystem" && call.input["operation"] == "list"
    )));
    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].input.current_user.message,
        "Explain the available tools."
    );
    assert!(!requests[0]
        .input
        .current_user
        .message
        .contains("Workspace root listing"));
    assert!(!requests[0]
        .input
        .current_user
        .message
        .contains("workspace marker"));
    assert!(requests[0]
        .tool_candidates
        .iter()
        .any(|candidate| candidate.name == "filesystem"));
    let (request_id, round, snapshot) = events
        .iter()
        .find_map(|event| match event {
            AgentEventPayload::ModelRequest {
                request_id,
                round,
                request,
            } => Some((request_id, round, request)),
            _ => None,
        })
        .expect("model request snapshot");
    assert_eq!(*round, 1);
    assert_eq!(
        snapshot["input"]["currentUser"]["message"],
        requests[0].input.current_user.message
    );
    assert_eq!(
        snapshot["toolCandidates"],
        serde_json::to_value(&requests[0].tool_candidates).unwrap()
    );
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ModelContextBuilt {
            request_id: context_request_id,
            items,
            ..
        } if context_request_id == request_id && !items.is_empty()
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ProviderRequestSent {
            request_id: provider_request_id,
            ..
        } if provider_request_id == request_id
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ProviderResponseReceived {
            request_id: response_request_id,
            ..
        } if response_request_id == request_id
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn flow_tool_call_budget_denies_calls_before_execution() {
    let workspace = test_workspace("flow-tool-budget");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace.clone(),
        PermissionMode::FullAccess,
    ));
    let mut agent = AgentCore::new(Arc::new(MockProvider), ToolRegistry::with_builtins());
    agent.set_tool_call_budget(1);
    let mut events = TurnEvents::new(None);

    agent
        .execute_tool_call(
            ToolCall::new("filesystem", json!({"operation": "list", "path": "."})),
            ToolInvocationContext::local(workspace.clone(), policy.clone()),
            &mut events,
            None,
        )
        .await
        .expect("first tool call is inside budget");
    let error = agent
        .execute_tool_call(
            ToolCall::new("filesystem", json!({"operation": "list", "path": "."})),
            ToolInvocationContext::local(workspace.clone(), policy),
            &mut events,
            None,
        )
        .await
        .expect_err("second tool call must be denied");
    assert!(error.to_string().contains("tool-call budget exhausted"));
    assert_eq!(agent.tool_calls_used(), 1);

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn flow_transcript_keeps_tool_activity_without_hidden_reasoning() {
    let call = ToolCall::new("filesystem", json!({"operation": "list", "path": "."}));
    let events = vec![
        AgentEventPayload::ReasoningDelta {
            text: "private reasoning must not be persisted".to_string(),
        },
        AgentEventPayload::ToolCallStarted { call: call.clone() },
        AgentEventPayload::ToolCallFinished {
            result: ToolResult::text(call.id, "[]", json!({"isError": false})),
        },
    ];

    let transcript = flow_transcript_from_events(&events);
    assert_eq!(transcript.len(), 2);
    assert_eq!(transcript[0].kind, FlowTranscriptEntryKindV1::ToolCall);
    assert_eq!(transcript[1].kind, FlowTranscriptEntryKindV1::ToolResult);
    assert!(!serde_json::to_string(&transcript)
        .expect("serialize transcript")
        .contains("private reasoning"));
}

#[test]
fn post_parse_safe_point_consumes_steer_and_defers_non_control_observations() {
    let inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
    let turn_id = Uuid::new_v4();
    let mut agent = AgentCore::default().with_turn_inbox(inbox.clone());
    agent.set_turn_execution_identity(turn_id, 1);
    inbox.push(
        turn_id,
        TurnInboxItem::Reminder {
            source_id: "background".into(),
            message: "done".into(),
        },
    );
    let message_id = Uuid::new_v4();
    inbox.push(
        turn_id,
        TurnInboxItem::Steer {
            message_id,
            content: "Use the other implementation.".into(),
        },
    );

    let control = agent.drain_post_parse_control(Uuid::new_v4());
    assert_eq!(
        control.steers,
        vec![(message_id, "Use the other implementation.".into())]
    );
    assert!(!control.cancelled);
    assert!(matches!(
        inbox.drain(turn_id).as_slice(),
        [TurnInboxItem::Reminder { .. }]
    ));
}

#[tokio::test]
async fn post_parse_steer_discards_unstarted_tool_calls_without_orphans_or_side_effects() {
    let workspace = test_workspace("post-parse-steer");
    let turn_id = Uuid::new_v4();
    let inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
    let provider = Arc::new(SteerAfterParseProvider::new(inbox.clone(), turn_id));
    let mut agent =
        AgentCore::new(provider.clone(), ToolRegistry::with_builtins()).with_turn_inbox(inbox);
    agent.set_turn_execution_identity(turn_id, 1);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Create the requested output.".into(),
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
        .expect("steered turn completes");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(!workspace.join("must-not-exist.txt").exists());
    assert!(!result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { call }
            if call.name == "filesystem" && call.input["operation"] == "write"
    )));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.tool_results.iter().any(|result| {
        result.output.contains("Do not write the file") && result.metadata["stage"] == "user_steer"
    }));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn completion_guard_has_no_plan_requirement_or_evidence_business_scan() {
    let source = include_str!("../completion_guard.rs");
    for forbidden in [
        "TaskEvidenceKind",
        "requirements_uncovered",
        "plan_evidence_invalid",
        "plan_missing",
    ] {
        assert!(
            !source.contains(forbidden),
            "completion guard must not inspect {forbidden}"
        );
    }
    assert!(source.contains("completion_registry.signals"));
}

fn test_workspace(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("opentopia-{name}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn assistant_text(events: &[AgentEventPayload]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::AssistantMessage { message } => Some(
                message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        MessagePart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
