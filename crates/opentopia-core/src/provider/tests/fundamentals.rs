#[test]
fn provider_auth_headers_are_independent_from_protocol_adapter() {
    let client = reqwest::Client::new();
    let bearer = apply_provider_auth(
        client.get("https://example.test/v1/models"),
        ProviderAuthKind::Bearer,
        "secret",
    )
    .build()
    .unwrap();
    assert_eq!(bearer.headers()[AUTHORIZATION], "Bearer secret");
    assert!(!bearer.headers().contains_key("x-api-key"));

    let api_key = apply_provider_auth(
        client.get("https://example.test/v1/models"),
        ProviderAuthKind::XApiKey,
        "secret",
    )
    .build()
    .unwrap();
    assert_eq!(api_key.headers()["x-api-key"], "secret");
    assert!(!api_key.headers().contains_key(AUTHORIZATION));

    let anonymous = apply_provider_auth(
        client.get("https://example.test/v1/models"),
        ProviderAuthKind::None,
        "",
    )
    .build()
    .unwrap();
    assert!(!anonymous.headers().contains_key(AUTHORIZATION));
    assert!(!anonymous.headers().contains_key("x-api-key"));
}

#[test]
fn adapters_choose_streaming_from_negotiated_tool_capability() {
    let mut chat =
        OpenAiCompatibleProvider::new("https://relay.example/v1", "test-key", "test-model");
    let conservative = chat.prepare(Uuid::nil(), tool_request()).unwrap();
    assert_eq!(conservative.body["stream"], false);
    assert_eq!(
        conservative.response_commit,
        ProviderResponseCommitMode::Atomic
    );
    chat.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    assert_eq!(
        chat.prepare(Uuid::nil(), tool_request()).unwrap().body["stream"],
        true
    );

    let mut responses =
        OpenAiResponsesProvider::new("https://relay.example/v1", "test-key", "test-model");
    responses.native_web_search = false;
    let conservative = responses.prepare(Uuid::nil(), tool_request()).unwrap();
    assert_eq!(conservative.body["stream"], false);
    assert_eq!(
        conservative.response_commit,
        ProviderResponseCommitMode::Atomic
    );
    responses.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    assert_eq!(
        responses.prepare(Uuid::nil(), tool_request()).unwrap().body["stream"],
        true
    );

    let mut anthropic =
        AnthropicMessagesProvider::new("https://relay.example", "test-key", "test-model");
    let conservative = anthropic.prepare(Uuid::nil(), tool_request()).unwrap();
    assert_eq!(conservative.body["stream"], false);
    assert_eq!(
        conservative.response_commit,
        ProviderResponseCommitMode::Atomic
    );
    anthropic.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    assert_eq!(
        anthropic.prepare(Uuid::nil(), tool_request()).unwrap().body["stream"],
        true
    );
}

#[test]
fn adapters_fail_before_transport_when_function_tool_negotiation_failed() {
    let mut chat =
        OpenAiCompatibleProvider::new("https://relay.example/v1", "test-key", "test-model");
    chat.tool_protocol.function_tools = ProviderFeatureSupport::Unsupported;
    let chat_error = chat.prepare(Uuid::nil(), tool_request()).unwrap_err();
    assert!(chat_error.to_string().contains("function_tools"));

    let mut responses =
        OpenAiResponsesProvider::new("https://relay.example/v1", "test-key", "test-model");
    responses.native_web_search = false;
    responses.tool_protocol.function_tools = ProviderFeatureSupport::Unsupported;
    let responses_error = responses.prepare(Uuid::nil(), tool_request()).unwrap_err();
    assert!(responses_error.to_string().contains("function_tools"));

    let mut anthropic =
        AnthropicMessagesProvider::new("https://relay.example", "test-key", "test-model");
    anthropic.tool_protocol.function_tools = ProviderFeatureSupport::Unsupported;
    let anthropic_error = anthropic.prepare(Uuid::nil(), tool_request()).unwrap_err();
    assert!(anthropic_error.to_string().contains("function_tools"));
}

#[test]
fn hosted_tools_also_require_atomic_commit() {
    let mut responses =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5");
    responses.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;

    let prepared = responses.prepare(Uuid::nil(), model_request()).unwrap();

    assert!(prepared.body["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));
    assert_eq!(prepared.body["stream"], true);
    assert_eq!(prepared.response_commit, ProviderResponseCommitMode::Atomic);
}

#[test]
fn invalid_tool_arguments_observation_is_structured_and_redacted() {
    let arguments =
        r#"{"agent_type":"sk-secret-value","fork_turns":none,"message":"password=hunter2"}"#;
    let error = parse_required_tool_arguments(
        Some(&Value::String(arguments.to_string())),
        "function.arguments",
        Some("spawn_agent"),
    )
    .unwrap_err();

    let observation = tool_call_protocol_error_observation(&error, Some("non_streaming_failed"));
    let details = &observation["invalidToolArguments"];
    assert_eq!(details["field"], "function.arguments");
    assert_eq!(details["toolName"], "spawn_agent");
    assert_eq!(details["argumentBytes"], arguments.len());
    assert_eq!(
        details["fingerprint"],
        content_fingerprint(arguments.as_bytes())
    );
    assert_eq!(details["errorLine"], 1);
    assert!(details["errorColumn"].as_u64().unwrap() > 0);
    assert!(details["errorOffset"].as_u64().unwrap() > 0);
    assert_eq!(observation["recovery"], "non_streaming_failed");

    let rendered = observation.to_string();
    assert!(rendered.contains("none"));
    assert!(!rendered.contains("sk-secret-value"));
    assert!(!rendered.contains("hunter2"));
    assert!(!rendered.contains("password="));
}

#[test]
fn token_estimate_breakdown_attributes_materialized_context_and_schemas() {
    let mut request = model_request();
    request.instructions.items = vec![
        ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            "base",
            "base instructions",
            ContextCacheScope::Stable,
            crate::model_context::ContextSensitivity::Public,
        ),
        ModelContextItem::text(
            ContextItemKind::User,
            ContextRole::User,
            "current",
            "question",
            ContextCacheScope::Turn,
            crate::model_context::ContextSensitivity::Workspace,
        ),
    ];
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        "test:branch",
        "branch rules",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    request.input.current_user.message = "question".to_string();
    request.tool_candidates = vec![ProviderToolCandidate {
        name: "read_file".to_string(),
        description: "Read one file".to_string(),
        input_schema: json!({"type": "object"}),
        ..Default::default()
    }];

    let breakdown = request.token_estimate_breakdown();

    assert_eq!(
        breakdown.base_instructions,
        request.instructions.items[0].token_estimate
    );
    assert_eq!(
        breakdown.current_user,
        estimate_tokens(&request.input.current_user.message)
    );
    assert!(breakdown.developer_instructions > 0);
    assert!(breakdown.tool_schemas > 0);
    assert_eq!(
        breakdown.total,
        breakdown.base_instructions
            + breakdown.developer_instructions
            + breakdown.current_user
            + breakdown.tool_schemas
    );
}

#[test]
fn token_estimate_breakdown_treats_images_as_typed_inputs() {
    let image = ModelContentPart::image("image/png", vec![0xff; 282_039]);
    let mut request = model_request();
    request.input.current_user.content = vec![image.clone()];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "image_result".to_string(),
        name: "view_image".to_string(),
        output: "image attached".to_string(),
        content: vec![image],
        is_error: false,
        metadata: json!({ "success": true }),
    }];
    request.instructions.items = vec![
        ModelContextItem::text(
            ContextItemKind::User,
            ContextRole::User,
            "current_user_message",
            &request.input.current_user.message,
            ContextCacheScope::Turn,
            crate::model_context::ContextSensitivity::Workspace,
        ),
        ModelContextItem::text(
            ContextItemKind::ToolResult,
            ContextRole::Tool,
            "tool_result:image_result",
            serde_json::to_string(&request.input.tool_results[0]).unwrap(),
            ContextCacheScope::Round,
            crate::model_context::ContextSensitivity::Sensitive,
        ),
    ];

    let serialized_user_content = estimate_serialized_slice(&request.input.current_user.content);
    let breakdown = request.token_estimate_breakdown();

    assert_eq!(
        breakdown.current_user,
        estimate_tokens(&request.input.current_user.message) + 282_039 / 16
    );
    assert!(
        serialized_user_content > breakdown.current_user.saturating_mul(8),
        "raw image bytes must not be estimated as a serialized integer array"
    );
    assert!(breakdown.tool_results < 20_000);
}

#[test]
fn codex_turn_input_preserves_order_around_local_image_attachments() {
    let mut request = model_request();
    request.input.current_user.content = vec![
        ModelContentPart::text("before image"),
        ModelContentPart::image("image/png", vec![0x89, b'P', b'N', b'G']),
        ModelContentPart::text("after image"),
        ModelContentPart::image("image/jpeg", vec![0xff, 0xd8, 0xff]),
    ];
    let mut paths = Vec::new();

    let input = codex_turn_input(&request, &mut paths).expect("build Codex local input");

    assert_eq!(input.len(), 4);
    assert_eq!(input[0]["type"], "text");
    assert!(input[0]["text"].as_str().unwrap().ends_with("before image"));
    assert_eq!(input[1]["type"], "localImage");
    assert_eq!(input[1]["detail"], "original");
    assert_eq!(input[2]["type"], "text");
    assert_eq!(input[2]["text"], "after image");
    assert_eq!(input[3]["type"], "localImage");
    assert_eq!(paths.len(), 2);
    assert_eq!(
        paths[0].extension().and_then(|value| value.to_str()),
        Some("png")
    );
    assert_eq!(
        paths[1].extension().and_then(|value| value.to_str()),
        Some("jpg")
    );
    assert_eq!(
        std::fs::read(&paths[0]).unwrap(),
        vec![0x89, b'P', b'N', b'G']
    );

    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn isolated_codex_host_profile_does_not_override_user_login() {
    assert!(is_isolated_codex_host_profile(
        Path::new("C:/Users/example/AppData/Local/OpenAI/codex-api"),
        Some("codex_vscode"),
    ));
    assert!(!is_isolated_codex_host_profile(
        Path::new("C:/Users/example/.codex"),
        Some("codex_vscode"),
    ));
    assert!(!is_isolated_codex_host_profile(
        Path::new("C:/Users/example/AppData/Local/OpenAI/codex-api"),
        None,
    ));
}

#[test]
fn visual_input_requires_the_declared_model_capability() {
    let mut request = model_request();
    request.input.current_user.content = vec![ModelContentPart::image("image/png", vec![1])];

    let error = ensure_visual_input_supported(&request, false).unwrap_err();

    assert!(error.to_string().contains("supportsVision"));
}

#[test]
fn codex_dynamic_tool_call_keeps_the_protocol_call_id() {
    let call = codex_dynamic_tool_call(&json!({
        "jsonrpc": "2.0",
        "id": "rpc-17",
        "method": "item/tool/call",
        "params": {
            "callId": "tool-42",
            "tool": "browser_observe",
            "arguments": "{\"includeScreenshot\":true}"
        }
    }))
    .expect("parse dynamic tool call");

    assert_eq!(call.call_id, "tool-42");
    assert_eq!(call.name, "browser_observe");
    assert_eq!(call.arguments["includeScreenshot"], true);
    assert_eq!(call.rpc_id, "rpc-17");
}

#[test]
fn codex_item_text_reads_the_app_server_agent_message_shape() {
    let text = codex_item_text(&json!({
        "type": "agentMessage",
        "id": "msg-1",
        "phase": "final_answer",
        "text": "final answer"
    }));

    assert_eq!(text, "final answer");
}

#[test]
fn model_decision_requires_normal_completion_non_empty_text_and_no_tools() {
    assert_eq!(
        ModelResponse::text("final response").decision(),
        ModelDecision::Final("final response".to_string())
    );

    let empty = ModelResponse::text("   ");
    assert_eq!(
        empty.decision(),
        ModelDecision::Incomplete(IncompleteReason::EmptyResponse)
    );

    let truncated = ModelResponse {
        text: "partial response".to_string(),
        finish_reason: ModelFinishReason::Length,
        ..ModelResponse::text("")
    };
    assert_eq!(
        truncated.decision(),
        ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
    );
}

#[test]
fn model_decision_rejects_tool_call_finish_without_calls() {
    let missing_call = ModelResponse {
        text: "I will use a tool.".to_string(),
        finish_reason: ModelFinishReason::ToolCalls,
        ..ModelResponse::text("")
    };

    assert_eq!(
        missing_call.decision(),
        ModelDecision::Incomplete(IncompleteReason::ProviderProtocol(
            "provider reported tool_calls but returned no tool call".to_string()
        ))
    );
    let protocol_error = validate_provider_response_protocol(missing_call)
        .expect_err("terminal provider validation must reject the response before commit");
    assert!(protocol_error
        .to_string()
        .contains("provider tool-call protocol error"));
}

#[test]
fn chat_stream_without_terminal_event_remains_interrupted() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    accumulator
        .apply(
            &json!({ "choices": [{ "delta": { "content": "partial" } }] }),
            &mut |_| Ok(()),
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(response.text, "partial");
    assert_eq!(
        response.decision(),
        ModelDecision::Incomplete(IncompleteReason::StreamInterrupted)
    );
}

#[test]
fn chat_stream_retains_length_finish_reason() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    accumulator
        .apply(
            &json!({ "choices": [{ "delta": { "content": "partial" } }] }),
            &mut |_| Ok(()),
        )
        .unwrap();
    accumulator
        .apply(
            &json!({ "choices": [{ "delta": {}, "finish_reason": "length" }] }),
            &mut |_| Ok(()),
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(response.finish_reason, ModelFinishReason::Length);
    assert_eq!(
        response.decision(),
        ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
    );
}

#[test]
fn responses_stream_retains_incomplete_and_interrupted_states() {
    let mut incomplete = ResponsesStreamAccumulator::default();
    incomplete
        .apply(
            &json!({
                "type": "response.incomplete",
                "response": {
                    "id": "resp_incomplete",
                    "status": "incomplete",
                    "incomplete_details": { "reason": "max_output_tokens" },
                    "output_text": "partial"
                }
            }),
            &mut |_| Ok(()),
        )
        .unwrap();
    let incomplete = incomplete.finish().unwrap();
    assert_eq!(incomplete.finish_reason, ModelFinishReason::Length);
    assert_eq!(
        incomplete.decision(),
        ModelDecision::Incomplete(IncompleteReason::OutputTokenLimit)
    );

    let interrupted = ResponsesStreamAccumulator::default().finish().unwrap();
    assert_eq!(
        interrupted.decision(),
        ModelDecision::Incomplete(IncompleteReason::StreamInterrupted)
    );
}

fn layered_model_request() -> ModelRequest {
    let mut request = model_request();
    request.prompt_cache_breakpoint_policy = PromptCacheBreakpointPolicy::AppendOnlyUsers;
    request.instructions.items = vec![
        ModelContextItem::text(
            crate::model_context::ContextItemKind::BaseInstructions,
            ContextRole::System,
            "opentopia:base",
            "base instructions",
            crate::model_context::ContextCacheScope::Stable,
            crate::model_context::ContextSensitivity::Public,
        ),
        ModelContextItem::text(
            crate::model_context::ContextItemKind::Environment,
            ContextRole::Developer,
            "opentopia:environment",
            "developer environment",
            crate::model_context::ContextCacheScope::Turn,
            crate::model_context::ContextSensitivity::Workspace,
        ),
    ];
    request
}

#[test]
fn chat_provider_maps_final_output_schema_to_strict_json_schema() {
    let mut provider =
        OpenAiCompatibleProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    provider.output_protocol.json_schema = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.final_output_json_schema = Some(json!({
        "type": "object",
        "properties": { "outcome": { "type": "string" } },
        "required": ["outcome"]
    }));
    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    assert_eq!(prepared.body["response_format"]["type"], "json_schema");
    assert_eq!(
        prepared.body["response_format"]["json_schema"]["strict"],
        true
    );
}

#[test]
fn responses_provider_maps_final_output_schema_to_text_format() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    provider.output_protocol.json_schema = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.final_output_json_schema = Some(json!({
        "type": "object",
        "properties": { "outcome": { "type": "string" } },
        "required": ["outcome"]
    }));
    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    assert_eq!(prepared.body["text"]["format"]["type"], "json_schema");
    assert_eq!(prepared.body["text"]["format"]["strict"], true);
}

#[test]
fn responses_provider_prioritizes_native_web_search_over_supplied_search_tools() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    let mut request = model_request();
    request.tool_candidates.push(ProviderToolCandidate {
        name: "mcp_search".to_string(),
        description: "Search the web through an MCP server".to_string(),
        input_schema: json!({ "type": "object", "properties": {} }),
        ..Default::default()
    });

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let tools = prepared.body["tools"].as_array().expect("tools array");
    assert_eq!(tools[0], json!({ "type": "web_search" }));
    assert_eq!(tools[1]["type"], "function");
    assert_eq!(tools[1]["name"], "mcp_search");
    assert_eq!(prepared.body["tool_choice"], "auto");
    assert!(prepared.body["instructions"]
        .as_str()
        .unwrap()
        .contains(NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION));
}

#[test]
fn responses_guardian_requests_do_not_receive_native_web_search() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test")
            .for_guardian();

    let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();
    let has_web_search = prepared.body["tools"].as_array().map_or(false, |tools| {
        tools.iter().any(|tool| tool["type"] == "web_search")
    });

    assert!(!has_web_search);
}

#[test]
fn guardian_openai_providers_keep_parallel_tool_calls_enabled() {
    let chat = OpenAiCompatibleProvider::new("https://api.openai.com/v1", "test-key", "gpt-test")
        .for_guardian();
    let responses =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test")
            .for_guardian();

    assert!(chat.parallel_tool_calls);
    assert!(responses.parallel_tool_calls);
}

#[test]
fn codex_app_server_routes_all_actions_through_opentopia_tools() {
    let mut settings = ProviderSettings::default();
    settings.kind = ProviderKind::CodexAppServer;
    let provider = CodexAppServerProvider::from_settings(&settings).unwrap();

    let normal_instructions =
        codex_developer_instructions(&model_request(), provider.native_web_search);
    assert!(!normal_instructions.contains("web search"));
    assert!(normal_instructions.contains("do not invoke built-in tools"));

    let guardian = provider.for_guardian();
    let guardian_instructions =
        codex_developer_instructions(&model_request(), guardian.native_web_search);
    assert!(!guardian_instructions.contains("web search"));
    assert!(guardian_instructions.contains("do not invoke built-in tools"));
}

#[test]
fn codex_builtin_actions_are_rejected_for_host_owned_execution() {
    for action in [
        "commandExecution",
        "fileChange",
        "mcpToolCall",
        "webSearch",
        "imageView",
        "collabToolCall",
    ] {
        assert!(is_codex_builtin_action(action));
    }
    assert!(!is_codex_builtin_action("dynamicToolCall"));
    assert!(!is_codex_builtin_action("agentMessage"));
}
