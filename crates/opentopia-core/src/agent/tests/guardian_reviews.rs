#[tokio::test]
async fn auto_review_approves_and_executes_the_exact_scoped_call() {
    let workspace = test_workspace("auto-review-approved");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_auto_write".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/auto-approved.txt",
                    "content": "reviewed once"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("The reviewed write completed."),
    ]));
    let mut reviewer_response = ModelResponse::text(
        r#"{"risk_level":"low","user_authorization":"high","outcome":"allow","rationale":"The user explicitly requested this narrow local write."}"#,
    );
    reviewer_response.usage = Some(ModelUsage {
        input_tokens: 20,
        output_tokens: 5,
        total_tokens: 25,
        cached_input_tokens: Some(8),
        cache_write_tokens: Some(2),
        reasoning_tokens: Some(1),
    });
    let reviewer = Arc::new(ScriptedProvider::new(vec![reviewer_response]));
    let agent =
        AgentCore::new(provider, ToolRegistry::with_builtins()).with_guardian_provider(reviewer);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Write the exact protected test file.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("auto-reviewed turn completes");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(
        fs::read_to_string(workspace.join(".codex/auto-approved.txt")).unwrap(),
        "reviewed once"
    );
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::AutomaticApprovalReviewCompleted {
            status: GuardianReviewStatus::Approved,
            usage,
            attempts: 1,
            tool_rounds: 0,
            failure_kind: None,
            ..
        } if usage.total_tokens == 25 && usage.cached_input_tokens == Some(8)
    )));
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    let review_completed = result
        .events
        .iter()
        .position(|event| {
            matches!(
                event,
                AgentEventPayload::AutomaticApprovalReviewCompleted {
                    status: GuardianReviewStatus::Approved,
                    ..
                }
            )
        })
        .expect("automatic review completed event");
    let started = result
        .events
        .iter()
        .enumerate()
        .filter_map(|(index, event)| match event {
            AgentEventPayload::ToolCallStarted { call }
                if call.name == "filesystem"
                    && call.input["path"] == json!(".codex/auto-approved.txt") =>
            {
                Some((index, call.id))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 1, "approved call must execute exactly once");
    assert!(review_completed < started[0].0);
    let finished = result
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata["providerToolCallId"] == json!("call_auto_write") =>
            {
                Some(result)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].call_id, started[0].1);
    assert!(!finished[0].output.starts_with("approval required:"));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn auto_review_batches_contiguous_preflight_asks_into_one_guardian_request() {
    let workspace = test_workspace("auto-review-batch");
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace)
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(workspace.join("first.tmp"), "first\n").unwrap();
    fs::write(workspace.join("second.tmp"), "second\n").unwrap();

    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![
                ProviderToolCall {
                    id: "call_batch_first".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git clean -fd -- first.tmp" }),
                },
                ProviderToolCall {
                    id: "call_batch_second".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git clean -fd -- second.tmp" }),
                },
            ],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Both reviewed cleanup actions completed."),
    ]));
    let reviewer = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        r#"{"risk_level":"medium","user_authorization":"high","outcome":"allow","rationale":"Both actions are exact workspace-local cleanup targets requested by the user."}"#,
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer.clone());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Remove the two exact temporary files.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("batched automatic review completes");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(!workspace.join("first.tmp").exists());
    assert!(!workspace.join("second.tmp").exists());
    assert_eq!(reviewer.requests().len(), 1);
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::AutomaticApprovalReviewCompleted {
            status: GuardianReviewStatus::Approved,
            action,
            ..
        } if action.get("type").and_then(Value::as_str) == Some("batch")
            && action.get("count").and_then(Value::as_u64) == Some(2)
    )));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].input.tool_results.len(), 2);
    assert!(requests[1].input.tool_results.iter().all(|result| {
        result
            .metadata
            .get("approvalSource")
            .and_then(Value::as_str)
            == Some("auto_review_batch")
    }));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn auto_review_policy_denial_is_returned_to_the_main_model_without_execution() {
    let workspace = test_workspace("auto-review-denied");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_auto_denied".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/auto-denied.txt",
                    "content": "must not exist"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("I stopped after the reviewer denied the action."),
    ]));
    let reviewer = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        r#"{"risk_level":"critical","user_authorization":"unknown","outcome":"deny_by_policy","rationale":"The protected metadata write is forbidden by tenant policy."}"#,
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect the repository.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("review denial is returned to the model");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(!workspace.join(".codex/auto-denied.txt").exists());
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::AutomaticApprovalReviewCompleted {
            status: GuardianReviewStatus::DeniedByPolicy,
            rationale,
            ..
        } if rationale.contains("forbidden")
    )));
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].input.tool_results[0]
            .metadata
            .get("approvalReview")
            .and_then(Value::as_str),
        Some("denied_by_policy")
    );
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn auto_review_needs_user_approval_suspends_for_the_user() {
    let workspace = test_workspace("auto-review-needs-user");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_auto_needs_user".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/auto-needs-user.txt",
                    "content": "must wait"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("The user-approved write completed."),
    ]));
    let reviewer = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        r#"{"risk_level":"high","user_authorization":"unknown","outcome":"needs_user_approval","rationale":"The concrete protected write needs the user's decision."}"#,
    )]));
    let agent =
        AgentCore::new(provider, ToolRegistry::with_builtins()).with_guardian_provider(reviewer);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect the repository.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("user-reviewable action suspends");

    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        other => panic!("expected suspended outcome, got {other:?}"),
    };
    assert!(!workspace.join(".codex/auto-needs-user.txt").exists());
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::AutomaticApprovalReviewCompleted {
            status: GuardianReviewStatus::NeedsUserApproval,
            ..
        }
    )));
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    let resumed = agent
        .resume_from_signal_streaming(
            continuation,
            crate::agent_runtime::AgentResumeSignal::Approval {
                approval_id: None,
                approved: true,
            },
            None,
            None,
            None,
        )
        .await
        .expect("explicit user approval resumes the concrete call");
    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert_eq!(
        fs::read_to_string(workspace.join(".codex/auto-needs-user.txt")).unwrap(),
        "must wait"
    );
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn one_user_decision_resumes_every_call_in_a_guardian_batch() {
    let workspace = test_workspace("auto-review-batch-user");
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace)
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(workspace.join("first.tmp"), "first\n").unwrap();
    fs::write(workspace.join("second.tmp"), "second\n").unwrap();
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![
                ProviderToolCall {
                    id: "call_user_batch_first".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git clean -fd -- first.tmp" }),
                },
                ProviderToolCall {
                    id: "call_user_batch_second".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git clean -fd -- second.tmp" }),
                },
            ],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("The user-approved cleanup batch completed."),
    ]));
    let reviewer = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        r#"{"risk_level":"high","user_authorization":"unknown","outcome":"needs_user_approval","rationale":"The two destructive actions need one explicit user decision."}"#,
    )]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer.clone());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Remove the two exact temporary files.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("batch waits for user approval");
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        other => panic!("expected suspended batch, got {other:?}"),
    };
    assert!(workspace.join("first.tmp").exists());
    assert!(workspace.join("second.tmp").exists());
    assert_eq!(reviewer.requests().len(), 1);

    let resumed = agent
        .resume_from_signal_streaming(
            continuation,
            crate::agent_runtime::AgentResumeSignal::Approval {
                approval_id: None,
                approved: true,
            },
            None,
            None,
            None,
        )
        .await
        .expect("one user approval resumes the exact batch");
    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert!(!workspace.join("first.tmp").exists());
    assert!(!workspace.join("second.tmp").exists());
    assert_eq!(reviewer.requests().len(), 1);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].input.tool_results.len(), 2);
    assert!(requests[1].input.tool_results.iter().all(|result| {
        result
            .metadata
            .get("approvalSource")
            .and_then(Value::as_str)
            == Some("user_batch")
    }));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn invalid_auto_reviewer_response_stops_without_requesting_user_approval() {
    let workspace = test_workspace("auto-review-invalid-response");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: String::new(),
        tool_calls: vec![ProviderToolCall {
            id: "call_auto_invalid_reviewer".to_string(),
            name: "filesystem".to_string(),
            arguments: json!({
                "operation": "write",
                "path": ".codex/auto-invalid-reviewer.txt",
                "content": "must not execute"
            }),
        }],
        usage: None,
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Stop,
    }]));
    let reviewer = Arc::new(ScriptedProvider::new(vec![
        ModelResponse::text("not json"),
        ModelResponse::text("still not json"),
        ModelResponse::text("invalid again"),
    ]));
    let agent =
        AgentCore::new(provider, ToolRegistry::with_builtins()).with_guardian_provider(reviewer);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Inspect the repository.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("reviewer failure becomes a stopped result");

    assert!(matches!(
        &result.outcome,
        AgentTurnOutcome::Stopped { reason } if reason.contains("invalid_reviewer_response")
    ));
    assert!(!workspace.join(".codex/auto-invalid-reviewer.txt").exists());
    assert!(
        result.events.iter().any(|event| matches!(
            event,
            AgentEventPayload::AutomaticApprovalReviewCompleted {
                status: GuardianReviewStatus::InvalidReviewerResponse,
                attempts: 3,
                failure_kind: Some(
                    crate::guardian::GuardianReviewFailureKind::InvalidReviewerResponse,
                ),
                ..
            }
        ))
    );
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn dangerous_dynamic_shell_action_is_returned_as_unreviewable() {
    let workspace = test_workspace("auto-review-unreviewable-shell");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_dynamic_delete".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": "rm -rf $target" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("I will resolve the target before retrying."),
    ]));
    let reviewer = Arc::new(ScriptedProvider::new(Vec::new()));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_guardian_provider(reviewer.clone());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Clean the generated target.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Auto,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("unreviewable action is returned to the model");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(reviewer.requests().is_empty());
    let requests = provider.requests();
    assert_eq!(
        requests[1].input.tool_results[0]
            .metadata
            .get("reviewability")
            .and_then(Value::as_str),
        Some("unreviewable_action")
    );
    assert_eq!(
        requests[1].input.tool_results[0].metadata["errorRecord"]["executed"],
        false
    );
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn turn_cancellation_reaches_shell_execution_context() {
    let workspace = test_workspace("turn-shell-cancellation");
    let cancellation = CancellationToken::new();
    let command = if cfg!(windows) {
        "powershell -NoProfile -Command \"Start-Sleep -Seconds 30\""
    } else {
        "sh -c 'sleep 30'"
    };
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: String::new(),
        tool_calls: vec![ProviderToolCall {
            id: "call_sleep".to_string(),
            name: "shell".to_string(),
            arguments: json!({ "command": command }),
        }],
        usage: None,
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Stop,
    }]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let workspace_for_turn = workspace.clone();
    let cancellation_for_turn = cancellation.clone();
    let task = tokio::spawn(async move {
        agent
            .run_turn(AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace_for_turn,
                content: "Run a long-running command.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: Some(cancellation_for_turn),
            })
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    cancellation.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("cancelled shell returns promptly")
        .expect("turn task joins");
    let error = result.expect_err("cancelled shell should fail the command turn");
    assert!(
        error.to_string().contains("cancelled"),
        "unexpected cancellation error: {error:#}"
    );
    let _ = fs::remove_dir_all(workspace);
}
