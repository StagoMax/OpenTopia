#[test]
fn provider_settings_validate_generation_limits_and_ids() {
    let mut provider = ProviderSettings::default();
    provider.id = "custom-glm".to_string();
    provider.name = "Custom GLM".to_string();
    provider.base_url = "https://example.test/v1".to_string();
    provider.temperature = Some(0.7);
    provider.max_output_tokens = Some(8_192);
    provider.context_window_tokens = Some(128_000);
    provider.reasoning_effort = Some("high".to_string());
    validate_provider_settings(&[provider.clone()]).expect("valid provider settings");

    provider.temperature = Some(3.0);
    let error = validate_provider_settings(&[provider]).expect_err("reject temperature");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut provider = ProviderSettings::default();
    provider.kind = ProviderKind::OpenAiResponses;
    provider.context_window_tokens = Some(8_192);
    provider.responses_compaction_threshold_tokens = Some(8_192);
    let error = validate_provider_settings(&[provider]).expect_err("reject compaction at window");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut provider = ProviderSettings::default();
    provider.rollout_budget = Some(opentopia_core::RolloutBudgetSettings {
        limit_tokens: 0,
        sampling_token_weight: 1.0,
        prefill_token_weight: 1.0,
    });
    let error = validate_provider_settings(&[provider]).expect_err("reject rollout budget");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut provider = ProviderSettings::default();
    provider.name = " ".to_string();
    let error = validate_provider_settings(&[provider]).expect_err("reject blank name");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);

    let mut provider = ProviderSettings::default();
    provider.model.clear();
    validate_provider_settings(&[provider.clone()])
        .expect("allow an empty model while discovery has not run");

    provider.synced_models = vec!["discovered-model".to_string()];
    let error = validate_provider_settings(&[provider])
        .expect_err("require a selected model after discovery");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}

#[test]
fn preview_artifact_resolution_is_scoped_to_the_route_thread() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let workspace = std::env::current_dir().expect("current directory");
    let owner = store
        .create_thread(Some("owner".to_string()), workspace.clone())
        .expect("create owner thread");
    let other = store
        .create_thread(Some("other".to_string()), workspace)
        .expect("create other thread");
    let artifact = store
        .insert_artifact(Artifact::inline(
            owner.id,
            "text",
            "text/plain; charset=utf-8",
            "thread private",
            json!({"name": "private.txt"}),
        ))
        .expect("insert artifact");
    let target = PreviewTarget::Artifact {
        artifact_id: artifact.id,
    };

    let owner_preview =
        resolve_preview_target(&store, &owner, &target).expect("owner resolves artifact");
    assert_eq!(owner_preview.descriptor.name, "private.txt");

    let error = resolve_preview_target(&store, &other, &target)
        .expect_err("other thread must not resolve artifact");
    assert_eq!(error.status, StatusCode::NOT_FOUND);
}

#[test]
fn preview_attachment_resolution_is_scoped_to_the_route_thread() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let workspace = std::env::current_dir().expect("current directory");
    let owner = store
        .create_thread(Some("owner".to_string()), workspace.clone())
        .expect("create owner thread");
    let other = store
        .create_thread(Some("other".to_string()), workspace)
        .expect("create other thread");
    let attachment_id = Uuid::new_v4();
    let attachment_path =
        std::env::temp_dir().join(format!("opentopia-preview-attachment-{attachment_id}.pdf"));
    std::fs::write(&attachment_path, b"%PDF-1.7").expect("write attachment");
    store
        .append_message(Message {
            id: Uuid::new_v4(),
            thread_id: owner.id,
            role: MessageRole::User,
            parts: vec![MessagePart::SourceRef {
                source: ContextSourceRef {
                    id: attachment_id,
                    path: attachment_path.clone(),
                    name: "brief.pdf".to_string(),
                    kind: opentopia_core::ContextSourceKind::Document,
                    content_type: "application/pdf".to_string(),
                    bytes: 8,
                    truncated: false,
                },
            }],
            created_at: Utc::now(),
        })
        .expect("store attachment reference");
    let target = PreviewTarget::Attachment { attachment_id };

    let owner_preview =
        resolve_preview_target(&store, &owner, &target).expect("owner resolves attachment");
    assert_eq!(owner_preview.descriptor.kind, PreviewKind::Pdf);

    let error = resolve_preview_target(&store, &other, &target)
        .expect_err("other thread must not resolve attachment");
    assert_eq!(error.status, StatusCode::NOT_FOUND);
    std::fs::remove_file(attachment_path).expect("remove attachment");
}

#[test]
fn preview_target_contract_uses_tagged_camel_case_artifact_id() {
    let artifact_id = Uuid::new_v4();
    let target: PreviewTarget = serde_json::from_value(json!({
        "source": "artifact",
        "artifactId": artifact_id,
    }))
    .expect("deserialize preview target");
    assert_eq!(target, PreviewTarget::Artifact { artifact_id });
}

#[test]
fn preview_target_contract_uses_tagged_camel_case_attachment_id() {
    let attachment_id = Uuid::new_v4();
    let target: PreviewTarget = serde_json::from_value(json!({
        "source": "attachment",
        "attachmentId": attachment_id,
    }))
    .expect("deserialize preview target");
    assert_eq!(target, PreviewTarget::Attachment { attachment_id });
}

#[test]
fn resource_resolve_contract_accepts_an_absolute_local_path_at_registration() {
    let path = PathBuf::from(r"C:\Users\Stargo\Downloads\说明.md");
    let request: ResourceResolveRequest = serde_json::from_value(json!({
        "source": "local",
        "path": path,
    }))
    .expect("deserialize local resource request");
    assert_eq!(
        request.into_locator(),
        ResourceLocator::Local {
            path: PathBuf::from(r"C:\Users\Stargo\Downloads\说明.md"),
        }
    );
}

#[test]
fn project_patch_distinguishes_missing_null_and_value_workspace() {
    let missing: UpdateProjectRequest =
        serde_json::from_value(json!({})).expect("deserialize missing workspace");
    assert!(matches!(missing.workspace_root, PatchValue::Missing));

    let null: UpdateProjectRequest = serde_json::from_value(json!({
        "workspaceRoot": null,
    }))
    .expect("deserialize null workspace");
    assert!(matches!(null.workspace_root, PatchValue::Null));

    let value: UpdateProjectRequest = serde_json::from_value(json!({
        "workspaceRoot": "J:\\Project\\OpenTopia",
        "sortOrder": 3,
    }))
    .expect("deserialize workspace value");
    assert!(matches!(
        value.workspace_root,
        PatchValue::Value(path) if path == PathBuf::from(r"J:\Project\OpenTopia")
    ));
    assert_eq!(value.sort_order, Some(3));
}

#[test]
fn thread_requests_use_camel_case_project_and_archive_fields() {
    let project_id = Uuid::new_v4();
    let create: CreateThreadRequest = serde_json::from_value(json!({
        "projectId": project_id,
    }))
    .expect("deserialize create thread");
    assert_eq!(create.project_id, Some(project_id));

    let missing_project: UpdateThreadRequest =
        serde_json::from_value(json!({})).expect("deserialize missing project patch");
    assert!(matches!(missing_project.project_id, PatchValue::Missing));

    let assign: UpdateThreadRequest = serde_json::from_value(json!({
        "projectId": project_id,
    }))
    .expect("deserialize project assignment");
    assert!(matches!(
        assign.project_id,
        PatchValue::Value(value) if value == project_id
    ));

    let detach: UpdateThreadRequest = serde_json::from_value(json!({
        "projectId": null,
    }))
    .expect("deserialize project detachment");
    assert!(matches!(detach.project_id, PatchValue::Null));

    let archive: UpdateThreadRequest = serde_json::from_value(json!({
        "archivedAt": Utc::now().to_rfc3339(),
    }))
    .expect("deserialize archive thread");
    assert!(matches!(archive.archived_at, PatchValue::Value(_)));

    let restore: UpdateThreadRequest = serde_json::from_value(json!({
        "archivedAt": null,
    }))
    .expect("deserialize restore thread");
    assert!(matches!(restore.archived_at, PatchValue::Null));
}

#[test]
fn store_errors_map_to_client_http_statuses() {
    let duplicate = ApiError::from(anyhow::Error::new(StoreError::DuplicateWorkspace(
        "j:/project/opentopia".to_string(),
    )));
    assert_eq!(duplicate.status, StatusCode::CONFLICT);

    let missing = ApiError::from(anyhow::Error::new(StoreError::ProjectNotFound(
        Uuid::new_v4(),
    )));
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let empty = ApiError::from(anyhow::Error::new(StoreError::EmptyProjectName));
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
}

#[test]
fn legacy_direct_tool_commands_are_not_agent_messages() {
    assert_eq!(legacy_direct_tool_command("/run cargo test"), Some("/run"));
    assert_eq!(
        legacy_direct_tool_command("  /READ src/lib.rs"),
        Some("/read")
    );
    assert_eq!(legacy_direct_tool_command("/run"), Some("/run"));
    assert_eq!(legacy_direct_tool_command("/runner status"), None);
    assert_eq!(legacy_direct_tool_command("Please /run the tests"), None);
}

#[test]
fn queued_turn_history_stops_before_the_current_message() {
    let thread_id = Uuid::new_v4();
    let first = Message::text(thread_id, MessageRole::User, "first");
    let current = Message::text(thread_id, MessageRole::User, "current");
    let future = Message::text(thread_id, MessageRole::User, "future queued input");

    let prior = prior_messages_for_turn(&[first.clone(), current.clone(), future], current.id)
        .expect("current message exists");

    assert_eq!(prior.len(), 1);
    assert_eq!(prior[0].id, first.id);
}

#[test]
fn persisted_tool_history_replays_as_structured_assistant_and_tool_messages() {
    let thread_id = Uuid::new_v4();
    let call = ToolCall::new("read_file", json!({"path": "README.md"}));
    let result = ToolResult::text(
        call.id,
        "file contents",
        json!({
            "providerToolCallId": "call_provider_1",
            "toolName": "read_file",
            "success": true,
        }),
    );
    let call_message = Message {
        id: Uuid::new_v4(),
        thread_id,
        role: MessageRole::Tool,
        parts: vec![MessagePart::ToolCall { call }],
        created_at: Utc::now(),
    };
    let result_message = Message {
        id: Uuid::new_v4(),
        thread_id,
        role: MessageRole::Tool,
        parts: vec![MessagePart::ToolResult { result }],
        created_at: Utc::now(),
    };

    let replay = project_model_conversation(&[call_message, result_message], &[]);
    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].role, ModelConversationRole::Assistant);
    assert_eq!(replay[0].tool_calls[0].id, "call_provider_1");
    assert_eq!(replay[1].role, ModelConversationRole::Tool);
    assert_eq!(replay[1].tool_results[0].call_id, "call_provider_1");
    assert_eq!(replay[1].tool_results[0].output, "file contents");
}

#[test]
fn provider_assistant_state_restores_parallel_tool_call_grouping() {
    let thread_id = Uuid::new_v4();
    let first = ToolCall::new("read_file", json!({"path": "a.txt"}));
    let second = ToolCall::new("read_file", json!({"path": "b.txt"}));
    let messages = vec![
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolCall {
                call: first.clone(),
            }],
            created_at: Utc::now(),
        },
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult {
                result: ToolResult::text(
                    first.id,
                    "A",
                    json!({
                        "providerToolCallId": "call_a",
                        "toolName": "read_file",
                        "success": true,
                    }),
                ),
            }],
            created_at: Utc::now(),
        },
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolCall {
                call: second.clone(),
            }],
            created_at: Utc::now(),
        },
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult {
                result: ToolResult::text(
                    second.id,
                    "B",
                    json!({
                        "providerToolCallId": "call_b",
                        "toolName": "read_file",
                        "success": true,
                    }),
                ),
            }],
            created_at: Utc::now(),
        },
    ];
    let provider_items = vec![json!({
        "type": "openai_chat_assistant_state",
        "content": "",
        "reasoning_content": "",
        "tool_call_ids": ["call_a", "call_b"],
    })];

    let replay = project_model_conversation(&messages, &provider_items);

    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].role, ModelConversationRole::Assistant);
    assert_eq!(replay[0].tool_calls.len(), 2);
    assert_eq!(replay[0].tool_calls[0].id, "call_a");
    assert_eq!(replay[0].tool_calls[1].id, "call_b");
    assert_eq!(replay[1].role, ModelConversationRole::Tool);
    assert_eq!(replay[1].tool_results.len(), 2);
    assert_eq!(replay[1].tool_results[0].call_id, "call_a");
    assert_eq!(replay[1].tool_results[1].call_id, "call_b");
}

#[test]
fn dangling_tool_calls_are_closed_before_history_replay() {
    let thread_id = Uuid::new_v4();
    let call = ToolCall::new("read_file", json!({"path": "lost.txt"}));
    let call_message = Message {
        id: Uuid::new_v4(),
        thread_id,
        role: MessageRole::Tool,
        parts: vec![MessagePart::ToolCall { call }],
        created_at: Utc::now(),
    };

    let replay = project_model_conversation(&[call_message], &[]);

    assert_eq!(replay.len(), 2);
    assert_eq!(replay[0].role, ModelConversationRole::Assistant);
    assert_eq!(replay[1].role, ModelConversationRole::Tool);
    assert!(replay[1].tool_results[0].is_error);
    assert_eq!(
        replay[1].tool_results[0].metadata["reason"],
        "missing_persisted_result"
    );
}

#[test]
fn canonical_tool_error_metadata_survives_history_projection() {
    let thread_id = Uuid::new_v4();
    let call = ToolCall::new("read_file", json!({"path": "missing.txt"}));
    let result = ToolResult::text(
        call.id,
        "not found",
        json!({
            "providerToolCallId": "call_missing",
            "toolName": "read_file",
            "isError": true,
        }),
    );
    let messages = vec![
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolCall { call }],
            created_at: Utc::now(),
        },
        Message {
            id: Uuid::new_v4(),
            thread_id,
            role: MessageRole::Tool,
            parts: vec![MessagePart::ToolResult { result }],
            created_at: Utc::now(),
        },
    ];

    let replay = project_model_conversation(&messages, &[]);

    assert!(replay[1].tool_results[0].is_error);
}

#[test]
fn chat_assistant_reasoning_is_included_in_history_budget() {
    let message = ModelConversationMessage {
        role: ModelConversationRole::Assistant,
        content: String::new(),
        content_parts: Vec::new(),
        tool_calls: vec![ProviderToolCall {
            id: "call_reasoning".to_string(),
            name: "read_file".to_string(),
            arguments: json!({}),
        }],
        tool_results: Vec::new(),
    };
    let provider_items = vec![json!({
        "type": "openai_chat_assistant_state",
        "content": "visible preamble",
        "reasoning_content": "reasoning ".repeat(100),
        "tool_call_ids": ["call_reasoning"],
    })];

    let without_state = model_conversation_message_token_estimate(&message, &[]);
    let with_state = model_conversation_message_token_estimate(&message, &provider_items);

    assert!(with_state > without_state + 100);
}

#[test]
fn browser_handoff_turns_are_paused_for_same_turn_resume() {
    assert!(!TurnStatus::WaitingUserAction.is_active());
    assert!(!TurnStatus::WaitingUserAction.is_terminal());
    assert_eq!(
        TurnStatus::WaitingUserAction.as_str(),
        "waiting_user_action"
    );
}

#[tokio::test]
async fn event_replay_deduplicates_events_seen_after_subscribe() {
    let bus = EventBus::default();
    let thread_id = Uuid::new_v4();
    let rx = bus.subscribe(thread_id);
    let first = AgentEvent::new(
        thread_id,
        Some(Uuid::new_v4()),
        1,
        AgentEventPayload::ModelDelta {
            text: "first".to_string(),
        },
    );
    bus.publish(first.clone());

    let mut events = Box::pin(replay_then_live_events(vec![first], rx, None));
    assert_eq!(events.next().await.expect("history event").seq, 1);

    let second = AgentEvent::new(
        thread_id,
        Some(Uuid::new_v4()),
        2,
        AgentEventPayload::ModelDelta {
            text: "second".to_string(),
        },
    );
    bus.publish(second);
    let next = timeout(Duration::from_secs(1), events.next())
        .await
        .expect("live event timeout")
        .expect("live event");
    assert_eq!(next.seq, 2, "queued history event must be skipped");
}

#[test]
fn conversation_stream_projection_removes_large_diagnostics() {
    let event = AgentEvent::new(
        Uuid::new_v4(),
        Some(Uuid::new_v4()),
        4,
        AgentEventPayload::ModelRequest {
            request_id: Uuid::new_v4(),
            round: 1,
            request: json!({"prompt": "large diagnostic payload"}),
        },
    );

    let projected = project_conversation_event(event).expect("project event");
    assert!(matches!(
        projected.payload,
        AgentEventPayload::ModelRequest { request, .. } if request.is_null()
    ));
}

#[test]
fn conversation_stream_projection_drops_hidden_reasoning() {
    let event = AgentEvent::new(
        Uuid::new_v4(),
        Some(Uuid::new_v4()),
        5,
        AgentEventPayload::ReasoningDelta {
            text: "hidden".to_string(),
        },
    );

    assert!(project_conversation_event(event).is_none());
}

#[test]
fn conversation_stream_projection_keeps_first_token_metrics() {
    let request_id = Uuid::new_v4();
    let event = AgentEvent::new(
        Uuid::new_v4(),
        Some(Uuid::new_v4()),
        6,
        AgentEventPayload::ProviderFirstTokenReceived { request_id },
    );

    let projected = project_conversation_event(event).expect("project first-token metric");
    assert!(matches!(
        projected.payload,
        AgentEventPayload::ProviderFirstTokenReceived {
            request_id: projected_request_id
        } if projected_request_id == request_id
    ));
}

#[test]
fn stream_payload_batches_merge_only_adjacent_compatible_deltas() {
    let payloads = compact_stream_payload_batch(vec![
        AgentEventPayload::ModelDelta {
            text: "hello ".to_string(),
        },
        AgentEventPayload::ModelDelta {
            text: "world".to_string(),
        },
        AgentEventPayload::Error {
            message: "boundary".to_string(),
        },
        AgentEventPayload::ModelDelta {
            text: "after boundary".to_string(),
        },
    ]);

    assert_eq!(payloads.len(), 3);
    assert!(matches!(
        &payloads[0],
        AgentEventPayload::ModelDelta { text } if text == "hello world"
    ));
    assert!(matches!(
        &payloads[2],
        AgentEventPayload::ModelDelta { text } if text == "after boundary"
    ));
}

#[test]
fn user_decision_questions_can_be_skipped_only_without_answers() {
    let request: UserInputRequest = serde_json::from_value(json!({
        "requestId": Uuid::new_v4(),
        "questions": [{
            "id": "runtime",
            "header": "Runtime",
            "question": "Which runtime should the task use?",
            "options": [
                { "id": "rust", "label": "Rust", "description": "Native runtime." },
                { "id": "node", "label": "Node", "description": "JavaScript runtime." }
            ]
        }]
    }))
    .expect("request");
    let skipped = validate_user_input_response(
        &request,
        UserInputResponse {
            answers: Vec::new(),
            skipped: true,
            cancelled: false,
        },
    )
    .expect("skip is valid");
    assert!(skipped.skipped);

    let cancelled = validate_user_input_response(
        &request,
        UserInputResponse {
            answers: Vec::new(),
            skipped: false,
            cancelled: true,
        },
    )
    .expect("cancel is valid");
    assert!(cancelled.cancelled);

    let invalid: UserInputResponse = serde_json::from_value(json!({
        "answers": [{ "questionId": "runtime", "optionId": "rust" }],
        "skipped": true
    }))
    .expect("response");
    assert!(validate_user_input_response(&request, invalid).is_err());

    let invalid_cancel: UserInputResponse = serde_json::from_value(json!({
        "answers": [{ "questionId": "runtime", "optionId": "rust" }],
        "cancelled": true
    }))
    .expect("cancel response");
    assert!(validate_user_input_response(&request, invalid_cancel).is_err());
}
