#[tokio::test]
async fn approved_batch_executes_disjoint_resources_concurrently_in_order() {
    let workspace = test_workspace("approved-parallel-batch");
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "parallel_process_test".to_string(),
        Arc::new(ParallelProcessTestTool {
            active,
            max_active: Arc::clone(&max_active),
        }),
    );
    let agent = AgentCore::new(Arc::new(MockProvider), registry)
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let calls = vec![
        ProviderToolCall {
            id: "approved-a".to_string(),
            name: "parallel_process_test".to_string(),
            arguments: json!({ "resource": "a" }),
        },
        ProviderToolCall {
            id: "approved-b".to_string(),
            name: "parallel_process_test".to_string(),
            arguments: json!({ "resource": "b" }),
        },
    ];
    assert_eq!(
        agent.approved_parallel_tool_call_indices(&calls),
        vec![0, 1]
    );

    let mut events = TurnEvents::new(None);
    let results = agent
        .execute_scoped_approved_batch(
            calls,
            &workspace,
            PermissionMode::FullAccess,
            None,
            None,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "test_batch",
            &mut events,
        )
        .await
        .expect("approved batch executes");

    assert_eq!(max_active.load(Ordering::SeqCst), 2);
    assert_eq!(
        results
            .iter()
            .map(|result| result.call_id.as_str())
            .collect::<Vec<_>>(),
        vec!["approved-a", "approved-b"]
    );
    assert!(results.iter().all(|result| {
        result
            .metadata
            .get("approvalSource")
            .and_then(Value::as_str)
            == Some("test_batch")
    }));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn independent_read_only_provider_calls_execute_concurrently_in_order() {
    let workspace = test_workspace("parallel-provider-calls");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![
                ProviderToolCall {
                    id: "read-a".to_string(),
                    name: "parallel_observation_test".to_string(),
                    arguments: json!({ "resource": "a" }),
                },
                ProviderToolCall {
                    id: "read-b".to_string(),
                    name: "parallel_observation_test".to_string(),
                    arguments: json!({ "resource": "b" }),
                },
            ],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("done"),
    ]));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "parallel_observation_test".to_string(),
        Arc::new(ParallelObservationTestTool {
            active,
            max_active: Arc::clone(&max_active),
        }),
    );
    let agent = AgentCore::new(provider.clone(), registry);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "inspect both resources".to_string(),
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
        .expect("parallel read-only turn succeeds");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(max_active.load(Ordering::SeqCst), 2);
    let requests = provider.requests();
    let result_ids = requests[1]
        .input
        .tool_results
        .iter()
        .map(|result| result.call_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(result_ids, vec!["read-a", "read-b"]);

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn non_read_only_calls_use_non_contiguous_parallel_selection_and_keep_result_order() {
    let workspace = test_workspace("parallel-process-provider-calls");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![
                ProviderToolCall {
                    id: "process-a".to_string(),
                    name: "parallel_process_test".to_string(),
                    arguments: json!({ "resource": "shared" }),
                },
                ProviderToolCall {
                    id: "process-b".to_string(),
                    name: "parallel_process_test".to_string(),
                    arguments: json!({ "resource": "shared" }),
                },
                ProviderToolCall {
                    id: "process-c".to_string(),
                    name: "parallel_process_test".to_string(),
                    arguments: json!({ "resource": "independent" }),
                },
            ],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("done"),
    ]));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "parallel_process_test".to_string(),
        Arc::new(ParallelProcessTestTool {
            active,
            max_active: Arc::clone(&max_active),
        }),
    );
    let agent = AgentCore::new(provider.clone(), registry);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "run all independent processes".to_string(),
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
        .expect("parallel process turn succeeds");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(max_active.load(Ordering::SeqCst), 2);
    let requests = provider.requests();
    let result_ids = requests[1]
        .input
        .tool_results
        .iter()
        .map(|result| result.call_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(result_ids, vec!["process-a", "process-b", "process-c"]);

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn model_driven_direction_choice_resumes_and_executes_the_answer() {
    let thread_id = Uuid::new_v4();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "ask_storage".to_string(),
                name: "request_user_input".to_string(),
                arguments: json!({
                    "questions": [{
                        "id": "storage",
                        "header": "Storage",
                        "question": "Which persistence strategy should the plan use?",
                        "options": [
                            {
                                "id": "sqlite",
                                "label": "SQLite",
                                "description": "Persist across restarts.",
                                "recommended": true
                            },
                            {
                                "id": "memory",
                                "label": "In memory",
                                "description": "Keep state only for the process lifetime."
                            }
                        ]
                    }]
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text(
            "<proposed_plan>\nThe plan uses SQLite as selected.\n</proposed_plan>",
        ),
    ]));
    let workspace = test_workspace("plan-user-input");
    let mut agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
    agent
        .apply_collaboration_mode(CollaborationMode::Plan, None)
        .expect("Plan mode");
    let catalog = agent.provider_tool_catalog();
    assert!(catalog.iter().any(|tool| tool.name == "request_user_input"));
    assert!(catalog.iter().any(|tool| tool.name == "set_plan"));

    let initial = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id,
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Plan the persistence architecture.".to_string(),
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
        .expect("initial plan turn");
    let (request, continuation) = match initial.outcome {
        AgentTurnOutcome::AwaitingInput {
            request,
            continuation,
        } => (request, continuation),
        other => panic!("expected user input suspension, got {other:?}"),
    };
    assert_eq!(request.questions[0].id, "storage");

    let resumed = agent
        .resume_from_signal_streaming(
            continuation,
            crate::agent_runtime::AgentResumeSignal::UserInput {
                request_id: request.request_id,
                response: UserInputResponse {
                    answers: vec![crate::model::UserInputAnswer {
                        question_id: "storage".to_string(),
                        option_id: Some("sqlite".to_string()),
                        custom_text: None,
                    }],
                    skipped: false,
                    cancelled: false,
                },
            },
            None,
            None,
            None,
        )
        .await
        .expect("resume plan turn");
    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert!(!resumed
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::WorkFormUpdated { .. })));
    let assistant = resumed
        .events
        .iter()
        .find_map(|event| match event {
            AgentEventPayload::AssistantMessage { message } => Some(message),
            _ => None,
        })
        .expect("Plan-mode assistant message");
    assert!(matches!(
        assistant.parts.as_slice(),
        [MessagePart::ProposedPlan { text }]
            if text == "\nThe plan uses SQLite as selected.\n"
    ));
    assert!(resumed.events.iter().all(|event| {
        !matches!(
            event,
            AgentEventPayload::ModelDelta { text }
                if text.contains("<proposed_plan>") || text.contains("</proposed_plan>")
        )
    }));
    let requests = provider.requests();
    let answered = requests[1]
        .input
        .tool_results
        .iter()
        .find(|result| result.name == "request_user_input")
        .expect("answered request result");
    assert!(answered.output.contains("sqlite"));
    assert!(!answered.output.contains('\n'));
    assert!(answered.metadata.get("userInputRequest").is_none());
    assert!(answered.metadata.get("userInputResponse").is_none());

    let _ = fs::remove_dir_all(workspace);
}

struct ReasoningProvider;

#[async_trait::async_trait]
impl ModelProvider for ReasoningProvider {
    async fn complete(&self, _request: ModelRequest) -> anyhow::Result<ModelResponse> {
        Ok(ModelResponse::text("已完成检查"))
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut crate::provider::ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self.complete(request).await?;
        on_delta(ModelStreamDelta::Reasoning {
            text: "正在检查项目结构".to_string(),
        })?;
        on_delta(ModelStreamDelta::Text {
            text: response.text.clone(),
        })?;
        Ok(response)
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

#[test]
fn base_agent_prompt_is_versioned_and_contains_the_runtime_contract() {
    let prompt = base_agent_prompt();
    let workspace = test_workspace("base-agent-prompt-contract");
    let context =
        default_agent_model_context(&workspace, &LocalSandboxConfig::danger_full_access());
    let base = context
        .items
        .iter()
        .find(|item| item.kind == ContextItemKind::BaseInstructions)
        .expect("base instructions are present");

    assert_eq!(base.text_content(), prompt);
    assert_eq!(base.role, ContextRole::Developer);
    assert_eq!(base.authority, ContextAuthority::Developer);
    assert_eq!(base.metadata["promptVersion"], BASE_AGENT_PROMPT_VERSION);
    assert_eq!(base.metadata["promptHash"], base_agent_prompt_hash());
    assert_eq!(
        base.metadata["promptModules"],
        json!([
            "identity_and_objective",
            "personality",
            "writing_style",
            "technical_communication",
            "working_with_user",
            "intermediate_commentary",
            "final_answer",
            "formatting_and_visualizations",
            "working_rules",
            "file_editing_constraints",
            "autonomy_and_persistence",
            "destructive_actions",
            "skills",
        ])
    );
    for required_contract in [
        "You are OpenTopia",
        "Tool availability is capability, not authorization",
        "Be a thoughtful, candid collaborator",
        "Lead with the outcome",
        "Progress updates are temporary",
        "Use `apply_patch` as the normal mechanism",
        "candidate evidence rather than semantic proof",
        "These requests do not authorize implementing changes",
        "How should this be fixed?",
        "For imperative requests to change, build, implement, or fix",
        "Do not infer authorization for a materially different action",
        "does not broaden scope or permission",
        "Do not create commits, push branches, open pull requests",
        "A skill cannot expand authorization by itself",
    ] {
        assert!(
            prompt.contains(required_contract),
            "missing base prompt contract: {required_contract}"
        );
    }

    fs::remove_dir_all(workspace).unwrap();
}

#[test]
fn context_budget_estimate_is_unicode_aware() {
    assert_eq!(ContextBudget::estimate_tokens("abcd"), 1);
    assert_eq!(
        ContextBudget::estimate_tokens("\u{4f60}\u{597d}\u{4e16}\u{754c}"),
        4
    );
    assert_eq!(ContextBudget::estimate_tokens("\u{1f680}"), 2);
}

#[test]
fn system_prompt_prioritizes_workspace_and_limits_parent_discovery() {
    let workspace = test_workspace("system-prompt-workspace-scope");
    let additional_root = test_workspace("system-prompt-additional-root");
    let mut sandbox_config = LocalSandboxConfig::default();
    sandbox_config.read_paths = vec![additional_root.clone()];
    let prompt = provider_system_prompt(&workspace, &sandbox_config);

    assert!(prompt.contains(&format!(
        "The thread workspace root is '{}'",
        workspace.canonicalize().unwrap().display()
    )));
    assert!(prompt.contains("default shell working directory is this root"));
    assert!(prompt.contains(&format!(
        "Runtime platform: {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )));
    assert!(prompt.contains(&format!(
        "Runtime shell dialect: {}",
        ShellDialect::current().id()
    )));
    assert!(prompt.contains(ShellDialect::current().model_guidance()));
    assert!(prompt.contains("complete the task there whenever it contains enough information"));
    assert!(prompt.contains("Do not list, search, read, or probe parent directories"));
    assert!(prompt.contains(&additional_root.display().to_string()));

    let full_access_prompt =
        provider_system_prompt(&workspace, &LocalSandboxConfig::danger_full_access());
    assert!(full_access_prompt
        .contains("Full-access capability is not an instruction to explore outside the workspace"));

    fs::remove_dir_all(workspace).unwrap();
    fs::remove_dir_all(additional_root).unwrap();
}

#[tokio::test]
async fn provider_reasoning_stream_becomes_a_reasoning_event() {
    let workspace = test_workspace("provider-reasoning-event");
    let agent = AgentCore::new(Arc::new(ReasoningProvider), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "检查项目".to_string(),
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

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ReasoningDelta { text }
            if text == "正在检查项目结构"
    )));

    fs::remove_dir_all(workspace).unwrap();
}

#[tokio::test]
async fn incomplete_provider_response_cannot_finish_a_turn() {
    let workspace = test_workspace("incomplete-provider-response");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: "partial answer".to_string(),
        tool_calls: Vec::new(),
        usage: None,
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Length,
    }]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());

    let error = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Return a status summary.".to_string(),
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
        .expect_err("truncated response must not finish the turn");

    assert!(error.to_string().contains("output token limit reached"));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn empty_response_after_tools_is_not_replaced_with_a_local_summary() {
    let workspace = test_workspace("empty-final-response");
    fs::write(workspace.join("status.txt"), "done").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_read_status".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read", "path": "status.txt" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("  "),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());

    let error = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Read the status and report it.".to_string(),
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
        .expect_err("empty model output must not become a local final response");

    assert!(error.to_string().contains("empty assistant response"));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn provider_tool_loop_executes_tool_and_requests_summary() {
    let workspace = test_workspace("provider-tool-loop");
    fs::write(workspace.join("sample.txt"), "hello from provider loop").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_read".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read", "path": "sample.txt" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("I read sample.txt and found hello from provider loop."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "What is in sample.txt?".to_string(),
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

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { call } if call.name == "filesystem"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallFinished { result }
            if result.metadata.get("providerToolCallId").and_then(Value::as_str) == Some("call_read")
    )));
    assert!(assistant_text(&events).contains("I read sample.txt"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0]
        .tool_candidates
        .iter()
        .any(|candidate| candidate.name == "filesystem"));
    assert_eq!(requests[1].input.tool_calls[0].id, "call_read");
    assert_eq!(requests[1].input.tool_results[0].call_id, "call_read");
    assert!(requests[1].input.tool_results[0]
        .output
        .contains("hello from provider loop"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn provider_tool_loop_normalizes_schema_equivalent_argument_keys() {
    let workspace = test_workspace("provider-tool-key-normalization");
    fs::write(workspace.join("sample.txt"), "sample").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_find".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "find",
                    "name_contains": "sample",
                    "case_sensitive": false,
                    "max_depth": 1
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("Found sample.txt."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Find sample files.".to_string(),
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
        .expect("schema-equivalent key spelling is normalized");

    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ContextWarning { stage, .. }
            if stage == "tool_argument_key_normalization"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { call }
            if call.input.get("nameContains").and_then(Value::as_str) == Some("sample")
                && call.input.get("caseSensitive").and_then(Value::as_bool) == Some(false)
                && call.input.get("maxDepth").and_then(Value::as_u64) == Some(1)
    )));
    assert!(assistant_text(&events).contains("Found sample.txt"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn schema_invalid_provider_tool_call_returns_actionable_error() {
    let workspace = test_workspace("invalid-provider-tool-call");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_read_without_path".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("The provider call was invalid, so I stopped."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Read a file.".to_string(),
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
        .expect("the model can recover from one invalid call");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let result = &requests[1].input.tool_results[0];
    assert!(result.is_error);
    assert_eq!(result.metadata["invalidToolArguments"], true);
    assert_eq!(result.metadata["errorRecord"]["recorded"], true);
    assert_eq!(
        result.metadata["errorRecord"]["code"],
        "invalid_tool_arguments"
    );
    assert_eq!(result.metadata["errorRecord"]["phase"], "validation");
    assert_eq!(result.metadata["errorRecord"]["executed"], false);
    assert!(result.output.contains("arguments.path is required"));
    assert!(result.output.contains("Do not retry this call unchanged"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn malformed_provider_arguments_are_returned_to_the_model_as_an_unexecuted_tool_error() {
    let workspace = test_workspace("malformed-provider-tool-json");
    let malformed = json!({
        "$opentopiaInvalidToolArguments": {
            "field": "function.arguments",
            "toolName": "spawn_agent",
            "reason": "expected value at line 1 column 47",
            "argumentBytes": 96,
            "fingerprint": "fnv1a64:test",
            "errorLine": 1,
            "errorColumn": 47,
            "errorOffset": 46,
            "redactedExcerpt": "…\"**********\":none,\"*******\":\"********\"…"
        }
    });
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_malformed_spawn".to_string(),
                name: "spawn_agent".to_string(),
                arguments: malformed,
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("I corrected the malformed tool call."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let result = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Delegate the review.".to_string(),
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
        .expect("the model can recover from malformed tool JSON");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].input.tool_calls[0].id, "call_malformed_spawn");
    let tool_result = &requests[1].input.tool_results[0];
    assert_eq!(tool_result.call_id, "call_malformed_spawn");
    assert!(tool_result.is_error);
    assert_eq!(tool_result.metadata["invalidToolArgumentsJson"], true);
    assert_eq!(tool_result.metadata["retryable"], true);
    assert_eq!(tool_result.metadata["errorRecord"]["executed"], false);
    assert_eq!(tool_result.metadata["errorRecord"]["retryable"], true);
    assert!(tool_result.output.contains("was not executed"));
    assert!(tool_result.output.contains("line 1, column 47"));
    assert!(tool_result.output.contains(r#""fork_turns":"none""#));
    assert!(result.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallFinished { result }
            if result.metadata["invalidToolArgumentsJson"] == true
    )));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn repeated_malformed_provider_argument_rounds_trip_the_circuit_breaker() {
    let workspace = test_workspace("malformed-provider-tool-json-loop");
    let responses = (1..=INVALID_TOOL_ARGUMENT_JSON_ROUND_LIMIT)
        .map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_malformed_{index}"),
                name: "spawn_agent".to_string(),
                arguments: json!({
                    "$opentopiaInvalidToolArguments": {
                        "reason": "expected value at line 1 column 47",
                        "errorLine": 1,
                        "errorColumn": 47,
                        "redactedExcerpt": "\"**********\":none"
                    }
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        })
        .collect::<Vec<_>>();
    let provider = Arc::new(ScriptedProvider::new(responses));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let error = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Delegate the review.".to_string(),
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
        .expect_err("the third malformed-JSON round must stop the turn");

    assert!(error
        .to_string()
        .contains("syntactically invalid tool-arguments JSON in 3 consecutive model rounds"));
    assert_eq!(
        provider.requests().len(),
        INVALID_TOOL_ARGUMENT_JSON_ROUND_LIMIT
    );
    assert_eq!(provider.requests()[2].input.tool_results.len(), 2);

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn valid_tool_round_resets_the_malformed_argument_circuit_breaker() {
    let malformed = ProviderToolCall {
        id: "call_malformed".to_string(),
        name: "spawn_agent".to_string(),
        arguments: json!({
            "$opentopiaInvalidToolArguments": {
                "reason": "expected value",
                "errorLine": 1,
                "errorColumn": 47,
            }
        }),
    };
    let valid = ProviderToolCall {
        id: "call_valid".to_string(),
        name: "filesystem".to_string(),
        arguments: json!({ "operation": "list", "path": "." }),
    };
    let mut runtime = TurnRuntimeState::default();

    runtime.record_tool_calls(std::slice::from_ref(&malformed));
    runtime.record_tool_calls(std::slice::from_ref(&malformed));
    assert_eq!(runtime.invalid_tool_argument_json_rounds, 2);
    runtime.record_tool_calls(std::slice::from_ref(&valid));
    assert_eq!(runtime.invalid_tool_argument_json_rounds, 0);
    assert!(
        repeated_invalid_tool_call_error(&runtime, std::slice::from_ref(&malformed), &[]).is_none()
    );
}

#[tokio::test]
async fn repeated_schema_invalid_provider_calls_trip_circuit_breaker() {
    let workspace = test_workspace("invalid-provider-tool-call-loop");
    let responses = (1..=INVALID_TOOL_CALL_REPEAT_LIMIT)
        .map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_invalid_{index}"),
                name: "shell".to_string(),
                arguments: json!({}),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        })
        .collect::<Vec<_>>();
    let provider = Arc::new(ScriptedProvider::new(responses));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let error = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Run the command.".to_string(),
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
        .expect_err("the third identical invalid call must stop the turn");

    assert!(error
        .to_string()
        .contains("provider returned the same schema-invalid `shell` call 3 times"));
    assert_eq!(provider.requests().len(), INVALID_TOOL_CALL_REPEAT_LIMIT);

    let _ = fs::remove_dir_all(workspace);
}
