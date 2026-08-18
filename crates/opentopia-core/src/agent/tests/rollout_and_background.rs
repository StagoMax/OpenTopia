#[tokio::test]
async fn model_can_summarize_the_conversation_into_a_skill_tool_call() {
    let workspace = test_workspace("create-skill-tool-loop");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_create_skill".to_string(),
                name: "create_skill".to_string(),
                arguments: json!({
                    "name": "summarize-workflow",
                    "description": "Summarize a completed workflow into reusable instructions. Use when the user asks to preserve the current conversation as a Skill.",
                    "instructions": "# Summarize a workflow\n\nExtract the reusable decisions and steps from the conversation. Remove task-specific details. Preserve validation criteria and report the resulting artifact.",
                    "scope": "workspace"
                }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        },
        ModelResponse::text(
            "Created the `summarize-workflow` project Skill with reusable workflow instructions.",
        ),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

    let events = agent
        .run_turn(AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.clone(),
            content: "Summarize what we just did and create it as a project Skill.".to_string(),
            user_content: Vec::new(),
            context_summary: Some(
                "The conversation established a repeatable implementation and validation workflow."
                    .to_string(),
            ),
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: None,
            cancellation: None,
        })
        .await
        .expect("turn succeeds");

    let skill_file = workspace.join(".agents/skills/summarize-workflow/SKILL.md");
    assert!(skill_file.is_file());
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ToolCallStarted { call } if call.name == "create_skill"
    )));
    assert!(assistant_text(&events).contains("Created the `summarize-workflow`"));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let candidate = requests[0]
        .tool_candidates
        .iter()
        .find(|candidate| candidate.name == "create_skill")
        .expect("create_skill is exposed to the model");
    assert!(candidate.description.contains("current conversation"));
    assert!(requests[1].input.tool_results[0]
        .output
        .contains("Created Skill `summarize-workflow`"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn rollout_budget_stops_before_another_provider_round() {
    let workspace = test_workspace("rollout-budget-exhausted");
    let provider = Arc::new(ScriptedProvider::new(vec![ModelResponse {
        text: String::new(),
        tool_calls: vec![ProviderToolCall {
            id: "call_list".to_string(),
            name: "filesystem".to_string(),
            arguments: json!({ "operation": "list", "path": "." }),
        }],
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
        .expect_err("exhausted budget stops the rollout");

    assert!(error
        .to_string()
        .contains("shared rollout token budget exhausted"));
    assert_eq!(provider.requests().len(), 1);

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn rollout_self_review_checkpoints_are_due_before_the_hard_limit() {
    assert!(!rollout_checkpoint_due(89, 0));
    assert!(rollout_checkpoint_due(90, 0));
    assert!(!rollout_checkpoint_due(179, 1));
    assert!(rollout_checkpoint_due(180, 1));
    assert!(!rollout_checkpoint_due(269, 2));
    assert!(!rollout_checkpoint_due(270, 2));
    assert!(!rollout_checkpoint_due(271, 3));
}

fn spent_rollout_budget(limit_tokens: u64, spent: u64) -> RolloutBudget {
    let mut budget = RolloutBudget::new(RolloutBudgetSettings {
        limit_tokens,
        sampling_token_weight: 1.0,
        prefill_token_weight: 1.0,
    });
    budget.record_usage(&ModelUsage {
        input_tokens: 0,
        output_tokens: spent,
        total_tokens: spent,
        cached_input_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
    });
    budget
}

#[test]
fn budget_reminder_is_only_consumed_once_delivery_is_confirmed() {
    let mut budget = spent_rollout_budget(100, 80);
    let reminder = budget
        .pending_reminder()
        .expect("crossing a threshold produces a reminder");

    // A round that failed or was cancelled before reaching the model must not
    // swallow the reminder.
    assert!(budget.pending_reminder().is_some());

    budget.mark_reminder_delivered(&reminder);
    assert!(budget.pending_reminder().is_none());
}

#[test]
fn repeated_tool_call_counts_are_objective_windowed_telemetry() {
    fn listing(path: &str) -> Vec<ProviderToolCall> {
        vec![ProviderToolCall {
            id: format!("call-{path}"),
            name: "filesystem".to_string(),
            arguments: json!({ "operation": "list", "path": path }),
        }]
    }

    let mut repeating = TurnRuntimeState::default();
    repeating.record_tool_calls(&listing("."));
    repeating.record_tool_calls(&listing("."));
    assert!(repeating.repeated_tool_call_counts().is_empty());

    // The call id deliberately stays out of the signature: only the action counts.
    repeating.record_tool_calls(&listing("."));
    let repeated = repeating.repeated_tool_call_counts();
    assert_eq!(repeated.len(), 1);
    let (signature, count) = repeated[0];
    assert!(signature.contains("filesystem"));
    assert_eq!(count, REPEATED_TOOL_CALL_REPORT_THRESHOLD);

    // Counts describe canonical calls only; they do not label distinct calls
    // as progress or repetition as lack of progress.
    let mut distinct = TurnRuntimeState::default();
    for index in 0..REPEATED_TOOL_CALL_WINDOW {
        distinct.record_tool_calls(&listing(&format!("dir{index}")));
    }
    assert!(distinct.repeated_tool_call_counts().is_empty());

    assert!(repeating.repeated_tool_call_report_due(1));
    let reminded = TurnRuntimeState {
        last_repeated_tool_call_report_round: Some(5),
        ..repeating.clone()
    };
    assert!(!reminded.repeated_tool_call_report_due(6));
    assert!(reminded.repeated_tool_call_report_due(5 + REPEATED_TOOL_CALL_REPORT_COOLDOWN_ROUNDS));
}

#[test]
fn repetition_telemetry_state_accepts_the_legacy_stall_field() {
    let state: TurnRuntimeState = serde_json::from_value(json!({
        "lastStallReminderRound": 7
    }))
    .unwrap();
    assert_eq!(state.last_repeated_tool_call_report_round, Some(7));

    let serialized = serde_json::to_value(state).unwrap();
    assert_eq!(serialized["lastRepeatedToolCallReportRound"], 7);
    assert!(serialized.get("lastStallReminderRound").is_none());
}

#[tokio::test]
async fn repeated_tool_calls_reach_the_model_as_an_observation() {
    let workspace = test_workspace("repeated-tool-call-telemetry");
    let mut responses = (1..=REPEATED_TOOL_CALL_REPORT_THRESHOLD)
        .map(rollout_tool_response)
        .collect::<Vec<_>>();
    responses.push(ModelResponse::text(
        "I interpreted the repetition using the results and finished.",
    ));
    let provider = Arc::new(ScriptedProvider::new(responses));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins());

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
        .expect("repetition telemetry does not end the turn");

    // The runtime reports counts without assigning them progress meaning.
    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    let requests = provider.requests();
    assert_eq!(requests.len(), REPEATED_TOOL_CALL_REPORT_THRESHOLD + 1);
    let telemetry = requests
        .iter()
        .flat_map(|request| &request.input.tool_results)
        .find(|result| {
            result.name == STEP_REMINDER_TOOL_NAME
                && result.output.contains("[Repeated tool-call telemetry]")
        })
        .expect("repeated canonical calls should produce objective telemetry");
    let telemetry = &telemetry.output;
    assert!(telemetry.contains(r#""occurrences":3"#));
    assert!(telemetry
        .contains(r#""groupedBy":"tool name and JSON arguments; provider call id excluded"#));
    assert!(!telemetry.contains("decide"));
    assert!(!telemetry.contains("progress"));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn a_running_background_job_is_advisory_not_a_completion_blocker() {
    let workspace = test_workspace("background-completion-advisory");
    let thread_id = Uuid::new_v4();
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 5; Write-Output finished"
    } else {
        "sleep 5; echo finished"
    };
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_slow_bg".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command, "background": true }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("The detached job is running; this turn can finish."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let registry = agent.background_processes();

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id,
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Start the slow job without waiting for it.".to_string(),
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
        .expect("running background work is advisory");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    assert!(result.events.iter().any(|event| matches!(
        event,
        AgentEventPayload::ContextWarning { stage, message }
            if stage == "completion_advisory"
                && message.contains("does not block this turn")
    )));

    let scope = BackgroundScope {
        thread_id,
        agent_path: "/root".to_string(),
    };
    for job in registry.list(&scope) {
        registry.stop(&scope, job.job_id).ok();
    }
    for _ in 0..100 {
        if registry
            .list(&scope)
            .iter()
            .all(|job| job.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn an_accepted_background_job_appends_its_terminal_result_durably() {
    let workspace = test_workspace("durable-background-completion");
    let store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
    let thread = store
        .create_thread(Some("durable background".to_string()), workspace.clone())
        .expect("create thread");
    let user_message_id = Uuid::new_v4();
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, user_message_id))
        .expect("insert turn");
    let command = if cfg!(windows) {
        "Start-Sleep -Milliseconds 750; Write-Output durable-finished"
    } else {
        "sleep 0.75; echo durable-finished"
    };
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_durable_bg".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command, "background": true }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("The detached job may finish after this turn."),
    ]));
    let agent = AgentCore::new(provider, ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let registry = agent.background_processes();

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id: thread.id,
                user_message_id,
                workspace_root: workspace.clone(),
                content: "Start the background job.".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: Some(Arc::clone(&store)),
                cancellation: None,
            },
            None,
        )
        .await
        .expect("turn completes after accepting background work");
    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));

    let scope = BackgroundScope {
        thread_id: thread.id,
        agent_path: "/root".to_string(),
    };
    for _ in 0..200 {
        if registry
            .list(&scope)
            .iter()
            .all(|job| job.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(registry.pending_completions(&scope).is_empty());

    let messages = store.list_messages(thread.id).expect("list messages");
    let durable_result = messages
        .iter()
        .flat_map(|message| &message.parts)
        .find_map(|part| match part {
            MessagePart::ToolResult { result }
                if result.metadata["durablyAppended"] == json!(true) =>
            {
                Some(result)
            }
            _ => None,
        })
        .expect("terminal result is appended to durable history");
    assert!(durable_result.output.contains("durable-finished"));
    assert_eq!(durable_result.metadata["sourceToolName"], "shell");
    assert!(store
        .list_events(thread.id, None)
        .expect("list events")
        .iter()
        .any(|event| event.turn_id == Some(turn.turn_id)
            && matches!(event.payload, AgentEventPayload::ToolCallFinished { .. })));

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn a_background_command_reports_itself_without_being_polled() {
    let workspace = test_workspace("background-command-delivery");
    let thread_id = Uuid::new_v4();
    let command = if cfg!(windows) {
        "Write-Output background-finished"
    } else {
        "echo background-finished"
    };

    // Round one starts the command and returns; round two must already carry the
    // result, without the model calling background_output at all.
    let provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_bg".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command, "background": true }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        rollout_tool_response(2),
        ModelResponse::text("The background command finished, so the work is done."),
    ]));
    let agent = AgentCore::new(provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let registry = agent.background_processes();

    let result = agent
        .run_turn_detailed_streaming(
            AgentTurnInput {
                thread_id,
                user_message_id: Uuid::new_v4(),
                workspace_root: workspace.clone(),
                content: "Start the long command and carry on.".to_string(),
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
        .expect("a background command does not block the turn");

    assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);

    // The spawn returned a job id straight away rather than the command output.
    let spawn_result = requests[1]
        .input
        .tool_results
        .iter()
        .find(|result| result.name == "shell")
        .expect("the shell call is answered");
    assert!(spawn_result.output.contains("jobId"));
    assert!(spawn_result.output.contains("running"));

    // Delivery is best-effort within one turn: the command may still be running when
    // the last round is built. Either way the model was never made to poll for it.
    assert!(!requests.iter().any(|request| request
        .input
        .tool_calls
        .iter()
        .any(|call| call.name == "background_output")));

    let scope = BackgroundScope {
        thread_id,
        agent_path: "/root".to_string(),
    };
    for _ in 0..100 {
        if registry
            .list(&scope)
            .iter()
            .all(|job| job.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let jobs = registry.list(&scope);
    assert_eq!(jobs.len(), 1, "the command is tracked for this agent");
    assert!(jobs[0].status.is_terminal());

    // Whatever was not delivered mid-turn is still pending, never lost.
    let delivered_in_turn = requests.iter().any(|request| {
        request
            .input
            .tool_results
            .iter()
            .any(|result| result.name == BACKGROUND_COMPLETION_TOOL_NAME)
    });
    assert!(!requests.iter().any(|request| request
        .instructions
        .items
        .iter()
        .any(|item| item.source
            == format!("opentopia:step_reminder:{BACKGROUND_COMMAND_REMINDER_STAGE}"))));
    let still_pending = !registry.pending_completions(&scope).is_empty();
    assert!(
        delivered_in_turn || still_pending,
        "a finished command must either have been reported or still be waiting to be"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn a_command_left_running_is_reported_on_the_next_turn() {
    let workspace = test_workspace("background-across-turns");
    let thread_id = Uuid::new_v4();
    let command = if cfg!(windows) {
        "Write-Output install-complete"
    } else {
        "echo install-complete"
    };

    // Turn one starts the command and stops without ever looking at it.
    let first_provider = Arc::new(ScriptedProvider::new(vec![
        ModelResponse {
            text: String::new(),
            tool_calls: vec![ProviderToolCall {
                id: "call_bg".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": command, "background": true }),
            }],
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        },
        ModelResponse::text("Started it; I will report back once it finishes."),
    ]));
    let first_agent = AgentCore::new(first_provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    let registry = first_agent.background_processes();

    let turn_input = |content: &str| AgentTurnInput {
        thread_id,
        user_message_id: Uuid::new_v4(),
        workspace_root: workspace.clone(),
        content: content.to_string(),
        user_content: Vec::new(),
        context_summary: None,
        conversation: Vec::new(),
        permission_mode: PermissionMode::FullAccess,
        context_budget: None,
        provider_cursor: None,
        store: None,
        cancellation: None,
    };

    first_agent
        .run_turn_detailed_streaming(turn_input("Kick off the install."), None)
        .await
        .expect("the first turn ends without waiting for the command");

    let scope = BackgroundScope {
        thread_id,
        agent_path: "/root".to_string(),
    };
    for _ in 0..100 {
        if registry
            .list(&scope)
            .iter()
            .all(|job| job.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // A second turn on the same thread, sharing the registry the way the server does.
    let second_provider = Arc::new(ScriptedProvider::new(vec![ModelResponse::text(
        "The install finished successfully.",
    )]));
    let mut second_agent = AgentCore::new(second_provider.clone(), ToolRegistry::with_builtins())
        .with_sandbox_config(LocalSandboxConfig::danger_full_access());
    second_agent.set_background_processes(registry.clone());

    second_agent
        .run_turn_detailed_streaming(turn_input("Did the install finish?"), None)
        .await
        .expect("the second turn completes");

    // The answer was already in the very first request of the new turn, so the model
    // never had to ask for it.
    let requests = second_provider.requests();
    assert_eq!(requests.len(), 1);
    let report = requests[0]
        .input
        .tool_results
        .iter()
        .find(|result| result.name == BACKGROUND_COMPLETION_TOOL_NAME)
        .expect("a command that finished between turns is reported on arrival");
    assert!(report.output.contains("install-complete"));
    assert!(requests[0]
        .input
        .tool_calls
        .iter()
        .any(|call| call.name == BACKGROUND_COMPLETION_TOOL_NAME));
    assert!(
        !requests[0].instructions.items.iter().any(|item| item.source
            == format!("opentopia:step_reminder:{BACKGROUND_COMMAND_REMINDER_STAGE}"))
    );
    assert!(registry.pending_completions(&scope).is_empty());

    let _ = fs::remove_dir_all(workspace);
}
