#[tokio::test]
async fn equivalent_tool_calls_are_not_blocked_by_the_runtime() {
    let workspace = test_workspace("equivalent-tool-loop");
    fs::write(workspace.join("sample.txt"), "stable content").unwrap();
    let responses = (0..4)
        .map(|index| ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: format!("call_read_{index}"),
                name: "filesystem".to_string(),
                arguments: json!({ "operation": "read", "path": "sample.txt" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        })
        .chain(std::iter::once(ModelResponse::text(
            "Stopped retrying the equivalent read.",
        )))
        .collect();
    let provider = Arc::new(ScriptedProvider::new(responses));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Inspect sample.txt without looping.".to_string(),
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
        .expect("equivalent calls remain model-controlled");

    assert_eq!(
        events
            .iter()
            .filter(
                |event| matches!(event, AgentEventPayload::ToolCallFinished { result }
                if result.metadata.get("providerToolCallId").and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("call_read_")))
            )
            .count(),
        4
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 5);
    let completed_reads = requests[4]
        .input
        .tool_results
        .iter()
        .filter(|result| result.name == "filesystem")
        .collect::<Vec<_>>();
    assert_eq!(completed_reads.len(), 4);
    assert!(completed_reads
        .iter()
        .all(|result| result.output.contains("stable content")));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn approve_mode_workspace_write_completes_without_suspension() {
    let workspace = test_workspace("approve-workspace-write");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_write".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": "approved.txt",
                    "content": "approved once"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Approved file written."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Create approved.txt with the requested content.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("workspace write completes");
    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert_eq!(
        fs::read_to_string(workspace.join("approved.txt")).unwrap(),
        "approved once"
    );
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn full_access_destructive_shell_command_still_suspends_for_user_approval() {
    let workspace = test_workspace("full-access-destructive-approval");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: String::new(),
        tool_calls: vec![ProviderToolCall {
            id: "call_destructive_shell".to_string(),
            name: "shell".to_string(),
            arguments: json!({ "command": "git reset --hard HEAD~1" }),
        }],
        usage: None,
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::Stop,
    }]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Run the destructive git command.".to_string(),
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
        .expect("destructive full-access command should suspend");

    assert!(matches!(
        &result.outcome,
        AgentTurnOutcome::Suspended { .. }
    ));
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    assert!(!result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { .. } | AgentEventPayload::ToolCallFinished { .. }
    )));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn unrestricted_destructive_shell_command_runs_without_approval() {
    let workspace = test_workspace("unrestricted-destructive-no-approval");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_unrestricted_destructive_shell".to_string(),
                name: "shell".to_string(),
                // The temporary workspace is not a repository, so this
                // exercises authorization without changing user data.
                arguments: json!({ "command": "git reset --hard HEAD~1" }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Command attempted without an approval pause."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Run the destructive git command without approval.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Unrestricted,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("unrestricted command should not suspend");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(!result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ApprovalRequested { .. })));
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, AgentEventPayload::ToolCallStarted { .. })));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn approved_protected_metadata_write_uses_one_shot_path_grant() {
    let workspace = test_workspace("approved-path-grant");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_write_metadata".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/config.toml",
                    "content": "approved metadata"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Approved metadata written."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Update the protected metadata configuration.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("protected metadata write suspends");
    assert!(!workspace.join(".codex/config.toml").exists());
    assert!(!result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { .. } | AgentEventPayload::ToolCallFinished { .. }
    )));
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        AgentTurnOutcome::Completed => panic!("protected write should wait for approval"),
        AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
            panic!("protected write should not reach terminal finalization")
        }
        AgentTurnOutcome::Stopped { .. } | AgentTurnOutcome::Cancelled { .. } => {
            panic!("turn should not be rollout-stopped")
        }
        AgentTurnOutcome::WaitingUserAction { .. } => {
            panic!("protected write should wait for approval, not browser input")
        }
        AgentTurnOutcome::AwaitingInput { .. } => {
            panic!("turn should not wait for user input")
        }
    };

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
        .expect("approved path grant resumes");

    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert_eq!(
        fs::read_to_string(workspace.join(".codex/config.toml")).unwrap(),
        "approved metadata"
    );
    let started = resumed
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ToolCallStarted { call }
                if call.name == "filesystem"
                    && call.input["path"] == json!(".codex/config.toml") =>
            {
                Some(call.id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let finished = resumed
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ToolCallFinished { result }
                if result.metadata["providerToolCallId"] == json!("call_write_metadata") =>
            {
                Some(result.call_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(started.len(), 1);
    assert_eq!(finished, started);
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn approved_external_write_grant_does_not_authorize_a_sibling_call() {
    let workspace = test_workspace("approved-external-path-grant");
    let outside = test_workspace("approved-external-path-target");
    let approved_path = outside.join("approved.txt");
    let sibling_path = outside.join("not-approved.txt");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_approved_path".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": approved_path,
                    "content": "approved once"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_sibling_path".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": sibling_path,
                    "content": "must require its own approval"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::enforce());

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Write only the explicitly approved external file.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("external write waits for approval");
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        AgentTurnOutcome::Completed => panic!("external write should wait for approval"),
        AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
            panic!("external write should not reach terminal finalization")
        }
        AgentTurnOutcome::Stopped { .. } | AgentTurnOutcome::Cancelled { .. } => {
            panic!("turn should not be rollout-stopped")
        }
        AgentTurnOutcome::WaitingUserAction { .. } => {
            panic!("external write should wait for approval, not browser input")
        }
        AgentTurnOutcome::AwaitingInput { .. } => {
            panic!("turn should not wait for user input")
        }
    };

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
        .expect("approved external path is written");

    assert!(matches!(
        resumed.outcome,
        AgentTurnOutcome::Suspended { .. }
    ));
    assert_eq!(fs::read_to_string(&approved_path).unwrap(), "approved once");
    assert!(!sibling_path.exists());
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(outside);
}

#[cfg(windows)]
#[tokio::test]
async fn approved_shell_command_uses_a_one_shot_sandbox_escape() {
    if crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
        return;
    }
    let workspace = test_workspace("approved-shell-remains-sandboxed");
    let outside = std::env::current_dir()
        .expect("current directory")
        .parent()
        .expect("workspace parent")
        .join(format!("opentopia-approved-outside-{}.txt", Uuid::new_v4()));
    let escaped_outside = outside.to_string_lossy().replace('\'', "''");
    let command = format!(
        "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{escaped_outside}' -Value approved-shell"
    );
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_shell".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("Approved shell command completed."),
    ]));
    let mut sandbox = LocalSandboxConfig::best_effort();
    sandbox.network = crate::sandbox::NetworkPolicy::Allow;
    sandbox.windows_backend = crate::sandbox::WindowsSandboxBackend::Unelevated;
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(sandbox);

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Run the requested external write command.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("sandbox denial suspends the turn");
    assert!(!outside.exists());
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        AgentTurnOutcome::Completed => panic!("sandbox denial should wait for approval"),
        AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
            panic!("sandbox denial should not reach terminal finalization")
        }
        AgentTurnOutcome::Stopped { .. } | AgentTurnOutcome::Cancelled { .. } => {
            panic!("turn should not be rollout-stopped")
        }
        AgentTurnOutcome::WaitingUserAction { .. } => {
            panic!("sandbox denial should wait for approval, not browser input")
        }
        AgentTurnOutcome::AwaitingInput { .. } => {
            panic!("turn should not wait for user input")
        }
    };

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
        .expect("approved call executes once outside the sandbox");

    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert!(outside.exists());
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1].input.tool_results[0]
            .metadata
            .get("approvalSource")
            .and_then(Value::as_str),
        Some("user")
    );
    let _ = fs::remove_file(outside);
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn denied_protected_metadata_write_completes_without_execution() {
    let workspace = test_workspace("denied-protected-continuation");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_denied_write".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/denied.txt",
                    "content": "never written"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("The file was not written because approval was denied."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins());
    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Create protected metadata with the requested content.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("turn suspends");
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        AgentTurnOutcome::Completed => panic!("turn should wait for approval"),
        AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
            panic!("approval denial should not reach terminal finalization")
        }
        AgentTurnOutcome::Stopped { .. } | AgentTurnOutcome::Cancelled { .. } => {
            panic!("turn should not be rollout-stopped")
        }
        AgentTurnOutcome::WaitingUserAction { .. } => {
            panic!("approval denial should wait for approval, not browser input")
        }
        AgentTurnOutcome::AwaitingInput { .. } => {
            panic!("turn should not wait for user input")
        }
    };

    let resumed = agent
        .resume_from_signal_streaming(
            continuation,
            crate::agent_runtime::AgentResumeSignal::Approval {
                approval_id: None,
                approved: false,
            },
            None,
            None,
            None,
        )
        .await
        .expect("denied turn resolves");
    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert!(!workspace.join(".codex/denied.txt").exists());
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn denied_protected_tool_call_is_returned_to_model_as_error() {
    let workspace = test_workspace("denied-provider-continuation");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_write".to_string(),
                name: "filesystem".to_string(),
                arguments: json!({
                    "operation": "write",
                    "path": ".codex/denied-provider.txt",
                    "content": "must not exist"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text("I did not write the file because approval was denied."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());
    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: Uuid::new_v4(),
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Create protected provider metadata".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::Approve,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
        )
        .await
        .expect("provider turn suspends");
    let continuation = match result.outcome {
        AgentTurnOutcome::Suspended { continuation, .. } => continuation,
        AgentTurnOutcome::Completed => panic!("protected write should require approval"),
        AgentTurnOutcome::Partial { .. } | AgentTurnOutcome::Blocked { .. } => {
            panic!("protected write should not reach terminal finalization")
        }
        AgentTurnOutcome::Stopped { .. } | AgentTurnOutcome::Cancelled { .. } => {
            panic!("turn should not be rollout-stopped")
        }
        AgentTurnOutcome::WaitingUserAction { .. } => {
            panic!("protected write should wait for approval, not browser input")
        }
        AgentTurnOutcome::AwaitingInput { .. } => {
            panic!("turn should not wait for user input")
        }
    };

    let resumed = agent
        .resume_from_signal_streaming(
            continuation,
            crate::agent_runtime::AgentResumeSignal::Approval {
                approval_id: None,
                approved: false,
            },
            None,
            None,
            None,
        )
        .await
        .expect("provider receives denial result");
    assert!(matches!(resumed.outcome, AgentTurnOutcome::Completed));
    assert!(assistant_text(&resumed.events).contains("approval was denied"));
    assert!(!workspace.join(".codex/denied-provider.txt").exists());
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].input.tool_results[0].is_error);
    assert_eq!(
        requests[1].input.tool_results[0]
            .metadata
            .get("approvalDenied")
            .and_then(Value::as_bool),
        Some(true)
    );
    let _ = fs::remove_dir_all(workspace);
}
