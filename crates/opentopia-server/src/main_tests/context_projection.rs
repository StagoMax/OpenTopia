#[test]
fn maps_inline_message_images_to_model_content() {
    let part = MessagePart::Image {
        id: None,
        content_type: "image/png".to_string(),
        data: vec![0x89, b'P', b'N', b'G'],
        name: Some("pasted.png".to_string()),
    };
    assert_eq!(
        message_model_content_parts(&part),
        vec![ModelContentPart::image(
            "image/png",
            vec![0x89, b'P', b'N', b'G']
        )]
    );
}

#[test]
fn model_catalog_reads_ids_from_every_shape_relays_return() {
    let openai = json!({"data": [{"id": "gpt-4.1-mini"}, {"id": "o3-mini"}]});
    assert_eq!(
        model_catalog_summary(&openai),
        vec![
            ("gpt-4.1-mini".to_string(), None, None),
            ("o3-mini".to_string(), None, None)
        ]
    );

    let bare = json!(["kimi-k2.5", "glm-4.6"]);
    assert_eq!(
        model_catalog_summary(&bare),
        vec![
            ("kimi-k2.5".to_string(), None, None),
            ("glm-4.6".to_string(), None, None)
        ]
    );

    let named = json!({"models": [{"name": "deepseek-reasoner"}]});
    assert_eq!(
        model_catalog_summary(&named),
        vec![("deepseek-reasoner".to_string(), None, None)]
    );
}

#[test]
fn model_catalog_default_preserves_provider_priority() {
    let payload = json!({
        "data": [
            {"id": "gpt-5.6-sol"},
            {"id": "codex-auto-review"},
            {"id": "gpt-5.6-terra"}
        ]
    });
    let catalog = extract_model_catalog(&payload);

    assert_eq!(
        provider_model_catalog_default(&catalog).as_deref(),
        Some("gpt-5.6-sol")
    );
}

#[test]
fn model_catalog_uses_anthropics_versioned_models_endpoint() {
    let provider = ProviderSettings {
        kind: ProviderKind::Anthropic,
        base_url: "https://api.anthropic.com/".to_string(),
        ..ProviderSettings::default()
    };
    assert_eq!(
        provider_model_catalog_url(&provider),
        "https://api.anthropic.com/v1/models"
    );
}

#[test]
fn model_catalog_accepts_service_roots_and_versioned_api_roots() {
    let root = ProviderSettings {
        base_url: "https://models.example".to_string(),
        ..ProviderSettings::default()
    };
    assert_eq!(
        provider_model_catalog_url(&root),
        "https://models.example/v1/models"
    );

    let versioned = ProviderSettings {
        base_url: "https://models.example/custom/v1/".to_string(),
        ..ProviderSettings::default()
    };
    assert_eq!(
        provider_model_catalog_url(&versioned),
        "https://models.example/custom/v1/models"
    );
}

#[test]
fn model_catalog_picks_up_whichever_context_field_the_endpoint_uses() {
    let payload = json!({"data": [
        {"id": "a", "context_length": 200_000},
        {"id": "b", "max_model_len": 32_768},
        {"id": "c", "max_input_tokens": 128_000},
        {"id": "d", "top_provider": {"context_length": 1_000_000}},
        {"id": "e", "max_context_tokens": "256000"},
    ]});
    assert_eq!(
        model_catalog_summary(&payload),
        vec![
            ("a".to_string(), Some(200_000), None),
            ("b".to_string(), Some(32_768), None),
            ("c".to_string(), Some(128_000), None),
            ("d".to_string(), Some(1_000_000), None),
            ("e".to_string(), Some(256_000), None),
        ]
    );
}

#[test]
fn model_catalog_reads_image_capabilities_without_guessing_missing_values() {
    let payload = json!({"data": [
        {"id": "vision", "architecture": {"input_modalities": ["text", "image"]}},
        {"id": "text-only", "input_modalities": ["text"]},
        {"id": "flag", "capabilities": {"vision": true}},
        {"id": "unknown"}
    ]});
    assert_eq!(
        model_catalog_summary(&payload),
        vec![
            ("vision".to_string(), None, Some(true)),
            ("text-only".to_string(), None, Some(false)),
            ("flag".to_string(), None, Some(true)),
            ("unknown".to_string(), None, None),
        ]
    );
}

#[test]
fn implausible_reported_context_windows_are_ignored() {
    // Too small to be a real window, and a byte count masquerading as tokens.
    let payload = json!({"data": [
        {"id": "tiny", "context_length": 8},
        {"id": "huge", "context_length": 999_000_000_u64},
        {"id": "text", "context_length": "not-a-number"},
    ]});
    assert_eq!(
        model_catalog_summary(&payload),
        vec![
            ("tiny".to_string(), None, None),
            ("huge".to_string(), None, None),
            ("text".to_string(), None, None),
        ]
    );
}

#[test]
fn local_thread_titles_are_unicode_bounded() {
    assert_eq!(
        local_thread_title("  修复\n\n侧栏标题滚动  "),
        Some("修复 侧栏标题滚动".to_string())
    );
    let title = local_thread_title(&"一".repeat(101)).expect("local title");
    assert_eq!(title.chars().count(), MAX_THREAD_TITLE_CHARS);
    assert!(title.ends_with('…'));
}

#[test]
fn title_generation_uses_the_thread_pinned_connection_and_model() {
    let mut settings = AppSettings::from_env(PermissionMode::Auto);
    let mut active = settings.active_provider().clone();
    active.id = "active".to_string();
    active.model = "active-model".to_string();
    active.base_url = "https://active.example/v1".to_string();

    let mut pinned = active.clone();
    pinned.id = "pinned".to_string();
    pinned.model = "connection-default".to_string();
    pinned.base_url = "https://pinned.example/v1".to_string();
    pinned.preferred_adapter = Some(ProviderAdapterKind::OpenAiChat);

    settings.active_provider_id = active.id.clone();
    settings.providers = vec![active, pinned];

    let selection: ThreadModelSelection = serde_json::from_value(serde_json::json!({
        "connectionId": "pinned",
        "modelId": "thread-model",
        "adapter": "open_ai_responses",
        "reasoningEffort": "high"
    }))
    .expect("legacy thread model selection");

    let provider = provider_settings_for_thread(&settings, Some(&selection));
    assert_eq!(provider.id, "pinned");
    assert_eq!(provider.base_url, "https://pinned.example/v1");
    assert_eq!(provider.model, "thread-model");
    assert_eq!(
        provider.resolved_adapter_for_model(&provider.model),
        ProviderAdapterKind::OpenAiChat
    );
    assert_eq!(
        provider.reasoning_effort_for_model(),
        Some("high".to_string())
    );
}

#[test]
fn local_thread_title_rejects_blank_prompts() {
    assert_eq!(local_thread_title(" \n\t "), None);
}

#[test]
fn plugin_mcp_identity_is_stable_and_source_specific() {
    let workspace = short_plugin_identity("workspace:C:/repo/.codex-plugin/plugin.json");
    assert_eq!(workspace.len(), 8);
    assert_eq!(
        workspace,
        short_plugin_identity("workspace:C:/repo/.codex-plugin/plugin.json")
    );
    assert_ne!(
        workspace,
        short_plugin_identity("codex:C:/repo/.codex-plugin/plugin.json")
    );
}

#[test]
fn token_estimate_is_conservative_for_non_ascii_text() {
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("上下文管理"), 10);
    assert!(estimate_tokens("上下文管理") > "上下文管理".len().div_ceil(4));
}

#[test]
fn legacy_automatic_summaries_do_not_skip_unverified_messages() {
    let thread_id = Uuid::new_v4();
    let mut legacy = ContextSummary::new(thread_id, 42, 9, "legacy");
    legacy.metadata = json!({ "mode": "llm" });
    assert_eq!(summary_message_cursor(&legacy), 0);

    let mut manual = ContextSummary::new(thread_id, 42, 9, "manual");
    manual.metadata = json!({ "mode": "manual" });
    assert_eq!(summary_message_cursor(&manual), 9);
}

#[test]
fn structured_checkpoint_coverage_is_server_owned_and_monotonic() {
    let thread_id = Uuid::new_v4();
    let mut summary = ContextSummary::new(thread_id, 99, 12, "rendered");
    summary.metadata = json!({ "coveredMessageCount": 2 });
    let mut checkpoint = ContextCheckpoint::manual(
        thread_id,
        ContextCheckpointCoverage {
            through_seq: 99,
            through_message_count: 10,
        },
        "goal",
    );
    checkpoint.mode = ContextCheckpointMode::StructuredLocal;
    summary.checkpoint = Some(checkpoint);

    assert_eq!(summary_message_cursor(&summary), 10);
}

#[test]
fn native_provider_checkpoint_does_not_advance_local_coverage() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    let mut previous = ContextSummary::new(thread.id, 8, 2, "durable");
    previous.metadata = json!({
        "mode": "structured_local",
        "coveredMessageCount": 2,
    });
    let mut checkpoint = ContextCheckpoint::manual(
        thread.id,
        ContextCheckpointCoverage {
            through_seq: 8,
            through_message_count: 2,
        },
        "durable goal",
    );
    checkpoint.mode = ContextCheckpointMode::StructuredLocal;
    previous.checkpoint = Some(checkpoint);
    store
        .append_event(AgentEvent::new(
            thread.id,
            None,
            0,
            AgentEventPayload::ContextCompacted {
                summary: previous,
                details: None,
            },
        ))
        .expect("append checkpoint");
    let cursor = ProviderConversationCursor {
        response_id: "response-1".to_string(),
        compatibility_hash: "compat-1".to_string(),
        response_items: vec![json!({"type": "compaction", "id": "compact-1"})],
        state_kind: opentopia_core::ProviderContextStateKind::Hybrid,
        compaction_item_count: 1,
    };

    let settings = AppSettings::from_env(PermissionMode::Auto);
    let native =
        build_native_provider_checkpoint(&store, settings.active_provider(), thread.id, &cursor)
            .expect("build native checkpoint")
            .expect("native checkpoint");
    let checkpoint = native.checkpoint.expect("checkpoint");
    assert_eq!(checkpoint.mode, ContextCheckpointMode::NativeProvider);
    assert_eq!(checkpoint.coverage.through_seq, 8);
    assert_eq!(checkpoint.coverage.through_message_count, 2);
    assert_eq!(
        checkpoint.provider_compatibility_hash.as_deref(),
        Some("compat-1")
    );
}

#[test]
fn provider_model_change_invalidates_persisted_cursor_with_a_reason() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    let mut settings = AppSettings::from_env(PermissionMode::Auto);
    let provider = settings.active_provider_mut();
    provider.kind = ProviderKind::OpenAiResponses;
    provider.store_responses = true;
    provider.model = "new-model".to_string();
    store
        .save_provider_conversation_state(&ProviderConversationState {
            thread_id: thread.id,
            agent_path: "/root".to_string(),
            provider_id: provider.id.clone(),
            model: "old-model".to_string(),
            adapter_identity: ProviderKind::OpenAiResponses.as_str().to_string(),
            response_id: "response-1".to_string(),
            compatibility_hash: "hash".to_string(),
            response_items: Vec::new(),
            state_kind: opentopia_core::ProviderContextStateKind::StoredResponse,
            compaction_item_count: 0,
            checkpoint_id: None,
            updated_at: Utc::now(),
        })
        .expect("save cursor");

    let taken = load_provider_cursor(&store, settings.active_provider(), thread.id, "/root")
        .expect("load cursor");
    assert!(taken.cursor.is_none());
    let invalidation = taken.invalidation.expect("invalidation");
    assert!(invalidation
        .reason
        .contains("provider, model, or adapter changed"));
    assert!(invalidation.reason.contains("new-model"));
}

#[test]
fn failed_provider_request_keeps_the_exact_sent_transcript_as_the_next_prefix() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    let mut settings = AppSettings::from_env(PermissionMode::Auto);
    let provider = settings.active_provider_mut();
    provider.apply_legacy_kind_preset(ProviderKind::OpenAiCompatible);
    provider.preferred_adapter = Some(ProviderAdapterKind::OpenAiChat);
    let provider = settings.active_provider().clone();
    let request_id = Uuid::new_v4();
    let transcript = opentopia_core::ProviderWireTranscript {
        format: "openai_chat_native_messages_v1".to_string(),
        items: vec![
            json!({"role": "developer", "content": "stable context"}),
            json!({"role": "user", "content": "request that failed"}),
        ],
    };
    let mut payloads = vec![AgentEventPayload::ProviderRequestSent {
        request_id,
        round: 1,
        attempt: 1,
        adapter: "openai_chat".to_string(),
        method: "POST".to_string(),
        endpoint: "https://example.invalid/v1/chat/completions".to_string(),
        cache_trace: None,
        body: Value::Null,
        checkpoint: Some(opentopia_core::ProviderRequestCheckpoint {
            compatibility_hash: "compat-1".to_string(),
            transcript: transcript.clone(),
        }),
    }];

    persist_provider_request_checkpoints(
        &store,
        &provider,
        thread.id,
        "/root",
        &mut payloads,
    )
    .expect("persist request checkpoint");

    let failed_result: anyhow::Result<opentopia_core::AgentTurnResult> =
        Err(anyhow::anyhow!("provider returned 403"));
    assert!(persist_provider_cursor(
        &store,
        &provider,
        thread.id,
        "/root",
        &failed_result,
    )
    .expect("retain checkpoint after failure")
    .is_none());
    let cancelled_result = Ok(opentopia_core::AgentTurnResult {
        events: Vec::new(),
        outcome: AgentTurnOutcome::Cancelled {
            reason: "cancelled while waiting for provider".to_string(),
        },
        provider_cursor: None,
    });
    assert!(persist_provider_cursor(
        &store,
        &provider,
        thread.id,
        "/root",
        &cancelled_result,
    )
    .expect("retain checkpoint after cancellation")
    .is_none());

    let state = store
        .get_provider_conversation_state(thread.id, "/root")
        .expect("load state")
        .expect("request checkpoint state");
    let (saved_transcript, provider_items) =
        opentopia_core::split_provider_transcript_state(state.response_items);
    assert_eq!(saved_transcript, Some(transcript));
    assert!(provider_items.is_empty());
    assert!(payloads[0].take_provider_request_checkpoint().is_none());

    let loaded = load_provider_cursor(&store, &provider, thread.id, "/root")
        .expect("load failed-request cursor")
        .cursor
        .expect("failed-request cursor");
    assert_eq!(loaded.compatibility_hash, "compat-1");
}

#[test]
fn provider_adapter_change_invalidates_persisted_cursor() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    let mut settings = AppSettings::from_env(PermissionMode::Auto);
    let provider = settings.active_provider_mut();
    provider.kind = ProviderKind::OpenAiResponses;
    provider.store_responses = true;
    store
        .save_provider_conversation_state(&ProviderConversationState {
            thread_id: thread.id,
            agent_path: "/root".to_string(),
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            adapter_identity: ProviderKind::OpenAiCompatible.as_str().to_string(),
            response_id: String::new(),
            compatibility_hash: "hash".to_string(),
            response_items: vec![json!({
                "type": "openai_chat_assistant_state",
                "tool_call_ids": ["call_1"],
            })],
            state_kind: opentopia_core::ProviderContextStateKind::TranscriptItems,
            compaction_item_count: 0,
            checkpoint_id: None,
            updated_at: Utc::now(),
        })
        .expect("save cursor");

    let loaded = load_provider_cursor(&store, settings.active_provider(), thread.id, "/root")
        .expect("load cursor");

    assert!(loaded.cursor.is_none());
    assert!(loaded
        .invalidation
        .expect("adapter invalidation")
        .reason
        .contains("adapter changed"));
    assert!(store
        .get_provider_conversation_state(thread.id, "/root")
        .unwrap()
        .is_none());
}

#[test]
fn loading_a_compatible_provider_cursor_is_non_destructive() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    let mut settings = AppSettings::from_env(PermissionMode::Auto);
    let provider = settings.active_provider_mut();
    provider.kind = ProviderKind::OpenAiResponses;
    provider.store_responses = true;
    store
        .save_provider_conversation_state(&ProviderConversationState {
            thread_id: thread.id,
            agent_path: "/root".to_string(),
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            adapter_identity: provider.resolved_route().adapter_identity(),
            response_id: "resp_1".to_string(),
            compatibility_hash: "hash".to_string(),
            response_items: Vec::new(),
            state_kind: opentopia_core::ProviderContextStateKind::StoredResponse,
            compaction_item_count: 0,
            checkpoint_id: None,
            updated_at: Utc::now(),
        })
        .expect("save cursor");

    let first =
        load_provider_cursor(&store, settings.active_provider(), thread.id, "/root").unwrap();
    let second =
        load_provider_cursor(&store, settings.active_provider(), thread.id, "/root").unwrap();

    assert_eq!(first.cursor.unwrap().response_id, "resp_1");
    assert_eq!(second.cursor.unwrap().response_id, "resp_1");
    assert!(store
        .get_provider_conversation_state(thread.id, "/root")
        .unwrap()
        .is_some());
}

#[test]
fn provider_cursor_is_stale_only_when_async_result_follows_last_model_request() {
    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread(None, std::env::current_dir().expect("cwd"))
        .expect("create thread");
    store
        .append_event(AgentEvent::new(
            thread.id,
            None,
            0,
            AgentEventPayload::ModelRequest {
                request_id: Uuid::new_v4(),
                round: 1,
                request: Value::Null,
            },
        ))
        .expect("append model request");
    store
        .append_event(AgentEvent::new(
            thread.id,
            None,
            0,
            AgentEventPayload::ToolCallFinished {
                result: ToolResult {
                    call_id: Uuid::new_v4(),
                    output: "finished".to_string(),
                    content: Vec::new(),
                    metadata: json!({
                        "asyncToolResult": true,
                        "agentPath": "/root",
                    }),
                },
            },
        ))
        .expect("append async result");

    assert!(
        provider_cursor_misses_async_result(&store, thread.id, "/root").expect("inspect cursor")
    );
    assert!(
        !provider_cursor_misses_async_result(&store, thread.id, "/root/child")
            .expect("inspect another agent")
    );

    store
        .append_event(AgentEvent::new(
            thread.id,
            None,
            0,
            AgentEventPayload::ModelRequest {
                request_id: Uuid::new_v4(),
                round: 2,
                request: Value::Null,
            },
        ))
        .expect("append incorporating request");
    assert!(
        !provider_cursor_misses_async_result(&store, thread.id, "/root")
            .expect("inspect refreshed cursor")
    );
}

#[test]
fn recent_tail_keeps_complete_turns_and_replays_ingressed_tools_verbatim() {
    let thread_id = Uuid::new_v4();
    let messages = vec![
        Message::text(thread_id, MessageRole::User, "old ".repeat(200)),
        Message::text(thread_id, MessageRole::Assistant, "old answer ".repeat(200)),
        Message::text(thread_id, MessageRole::User, "latest request"),
        Message::text(thread_id, MessageRole::Assistant, "latest answer"),
    ];
    let (tail, _) = recent_conversation_tail(&messages, 100, &[]);
    assert_eq!(tail.len(), 2);
    assert!(tail[0].content.contains("latest request"));
    assert!(tail[1].content.contains("latest answer"));

    let result = ToolResult {
        call_id: Uuid::new_v4(),
        output: "x".repeat(40_000),
        content: Vec::new(),
        metadata: json!({ "artifactId": "artifact-123" }),
    };
    let replayed = message_model_content_parts(&MessagePart::ToolResult {
        result: result.clone(),
    });
    let rendered = replayed
        .iter()
        .map(|part| match part {
            ModelContentPart::Text { text } => text.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(rendered, result.output);
}

#[test]
fn thread_snapshot_reemits_only_when_its_effective_signature_changes() {
    let snapshot = ThreadContextSnapshot {
        captured_at: Utc::now(),
        provider_id: "provider".to_string(),
        provider_kind: "openai_responses".to_string(),
        provider_adapter: "open_ai_responses:v1".to_string(),
        model: "model-a".to_string(),
        workspace_root: PathBuf::from("workspace"),
        cwd: PathBuf::from("workspace"),
        experience_mode: "code".to_string(),
        permission_mode: "auto".to_string(),
        sandbox_mode: "workspace_write".to_string(),
        instructions: Vec::new(),
        tool_catalog_hash: "tools-a".to_string(),
        world_state_hash: "world-a".to_string(),
        context_hash: "context-a".to_string(),
    };
    let mut unchanged = snapshot.clone();
    unchanged.captured_at = Utc::now();
    assert!(!thread_context_snapshot_changed(&snapshot, &unchanged));

    let mut changed_adapter = unchanged.clone();
    changed_adapter.provider_adapter = "open_ai_chat:v1".to_string();
    assert!(thread_context_snapshot_changed(&snapshot, &changed_adapter));

    let mut changed_tools = unchanged;
    changed_tools.tool_catalog_hash = "tools-b".to_string();
    assert!(thread_context_snapshot_changed(&snapshot, &changed_tools));
}
