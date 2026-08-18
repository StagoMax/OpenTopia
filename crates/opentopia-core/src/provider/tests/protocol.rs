#[test]
fn parses_openai_chat_tool_calls() {
    let body = json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_read",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": "{\"path\":\"Cargo.toml\"}"
                    }
                }]
            }
        }],
        "usage": {
            "prompt_tokens": 41,
            "completion_tokens": 7,
            "total_tokens": 48,
            "prompt_tokens_details": {
                "cached_tokens": 12,
                "cache_write_tokens": 29
            },
            "completion_tokens_details": { "reasoning_tokens": 3 }
        }
    });

    let response = parse_model_response_body(&body).expect("response parses");

    assert_eq!(response.text, "");
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "Cargo.toml" }),
        }]
    );
    assert_eq!(
        response.usage,
        Some(ModelUsage {
            input_tokens: 41,
            output_tokens: 7,
            total_tokens: 48,
            cached_input_tokens: Some(12),
            cache_write_tokens: Some(29),
            reasoning_tokens: Some(3),
        })
    );
}

#[test]
fn chat_adapter_rejects_a_value_less_optional_tail() {
    let body = json!({
        "choices": [{
            "message": {
                "tool_calls": [{
                    "id": "call_artifact",
                    "type": "function",
                    "function": {
                        "name": "read_artifact",
                        "arguments": "{\"artifactId\":\"66a8bebc-6f24-4c11-a4ac-1f62ca2dc220\",\"offset\":"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    let candidates = vec![ProviderToolCandidate::direct(
        "read_artifact",
        "Read a bounded artifact window",
        json!({
            "type": "object",
            "properties": {
                "artifactId": { "type": "string" },
                "offset": { "type": ["integer", "null"] },
                "limit": { "type": ["integer", "null"] }
            },
            "required": ["artifactId"],
            "additionalProperties": false
        }),
    )];

    let error = parse_model_response_body_with_tools(&body, &candidates)
        .expect_err("truncated wire arguments must fail instead of changing tool semantics");

    assert!(error
        .to_string()
        .contains("provider tool-call protocol error"));
}

#[test]
fn chat_stream_adapter_rejects_the_same_truncated_tail() {
    let candidates = vec![ProviderToolCandidate::direct(
        "read_artifact",
        "Read a bounded artifact window",
        json!({
            "type": "object",
            "properties": {
                "artifactId": { "type": "string" },
                "offset": { "type": ["integer", "null"] }
            },
            "required": ["artifactId"],
            "additionalProperties": false
        }),
    )];
    let mut accumulator = OpenAiStreamAccumulator::default();
    accumulator
        .apply(
            &json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_artifact",
                            "function": {
                                "name": "read_artifact",
                                "arguments": "{\"artifactId\":\"artifact-1\",\"offset\":"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }),
            &mut |_| Ok(()),
        )
        .unwrap();

    let error = accumulator
        .finish_with_tools(&candidates)
        .expect_err("stream and non-stream decoders must share the fail-closed boundary");

    assert!(error
        .to_string()
        .contains("provider tool-call protocol error"));
}

#[test]
fn required_tool_argument_parser_never_repairs_incomplete_json() {
    for raw in [
        "{\"artifactId\":",
        "{\"artifactId\":\"artifact-1\",\"offset\":1",
        "{\"artifactId\":\"artifact-1\",\"unknown\":",
    ] {
        let error = parse_required_tool_arguments(
            Some(&Value::String(raw.to_string())),
            "function.arguments",
            Some("read_artifact"),
        )
        .expect_err("incomplete JSON must never be repaired");
        assert!(error
            .to_string()
            .contains("provider tool-call protocol error"));
    }
}

#[test]
fn parses_responses_function_calls() {
    let body = json!({
        "output_text": "",
        "output": [{
            "type": "function_call",
            "call_id": "call_search",
            "name": "search",
            "arguments": "{\"query\":\"AgentCore\",\"path\":\"crates\"}"
        }]
    });

    let response = parse_model_response_body(&body).expect("response parses");

    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_search".to_string(),
            name: "search".to_string(),
            arguments: json!({ "query": "AgentCore", "path": "crates" }),
        }]
    );
}

#[test]
fn responses_adapter_rejects_malformed_arguments_before_replay() {
    let body = json!({
        "id": "resp_malformed",
        "output": [{
            "type": "function_call",
            "call_id": "call_malformed",
            "name": "spawn_agent",
            "arguments": "{\"agent_type\":\"default\",\"fork_turns\":none,\"message\":\"review\"}"
        }]
    });
    let error = parse_model_response_body(&body)
        .expect_err("malformed arguments must not enter replay state");

    assert!(error
        .to_string()
        .contains("provider tool-call protocol error"));
}

#[test]
fn responses_web_search_citations_become_clickable_markdown_links() {
    let body = json!({
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "OpenTopia source",
                "annotations": [{
                    "type": "url_citation",
                    "start_index": 9,
                    "end_index": 15,
                    "url": "https://example.test/source",
                    "title": "Example source"
                }]
            }]
        }]
    });

    let response = parse_model_response_body(&body).expect("response parses");
    assert_eq!(
        response.text,
        "OpenTopia [source](https://example.test/source)"
    );
}

#[test]
fn orders_system_history_current_user_and_current_tool_messages() {
    let mut request = model_request();
    request.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "earlier user".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "earlier assistant".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
    ];
    request.input.tool_calls = vec![ProviderToolCall {
        id: "call_1".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "Cargo.toml" }),
    }];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "call_1".to_string(),
        name: "read_file".to_string(),
        output: "workspace".to_string(),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    }];

    let messages = openai_messages(&request);

    assert_eq!(messages.len(), 6);
    assert_eq!(
        messages[0],
        json!({ "role": "system", "content": "system" })
    );
    assert_eq!(
        messages[1],
        json!({ "role": "user", "content": "earlier user" })
    );
    assert_eq!(
        messages[2],
        json!({ "role": "assistant", "content": "earlier assistant" })
    );
    assert_eq!(messages[3], json!({ "role": "user", "content": "current" }));
    assert_eq!(messages[4]["role"], "assistant");
    assert_eq!(messages[4]["content"], "");
    assert_eq!(messages[4]["tool_calls"][0]["id"], "call_1");
    assert_eq!(messages[5]["role"], "tool");
    assert_eq!(messages[5]["tool_call_id"], "call_1");
}

#[test]
fn chat_messages_return_malformed_argument_diagnostics_as_a_tool_error() {
    let mut request = model_request();
    request.input.tool_calls = vec![ProviderToolCall {
        id: "call_malformed".to_string(),
        name: "spawn_agent".to_string(),
        arguments: json!({
            INVALID_TOOL_ARGUMENTS_JSON_KEY: {
                "reason": "expected value at line 1 column 47",
                "errorLine": 1,
                "errorColumn": 47,
                "redactedExcerpt": "\"**********\":none"
            }
        }),
    }];
    request.input.tool_results = vec![ProviderToolResult {
            call_id: "call_malformed".to_string(),
            name: "spawn_agent".to_string(),
            output: "Tool `spawn_agent` was not executed because function.arguments was invalid JSON at line 1, column 47. Retry with valid JSON.".to_string(),
            content: Vec::new(),
            is_error: true,
            metadata: json!({
                "invalidToolArgumentsJson": true,
                "executed": false,
                "retryable": true,
            }),
        }];

    let messages = openai_messages(&request);
    let assistant = &messages[messages.len() - 2];
    let result = &messages[messages.len() - 1];
    assert_eq!(assistant["role"], "assistant");
    assert_eq!(assistant["tool_calls"][0]["id"], "call_malformed");
    assert_eq!(
        assistant["tool_calls"][0]["function"]["name"],
        "spawn_agent"
    );
    assert_eq!(result["role"], "tool");
    assert_eq!(result["tool_call_id"], "call_malformed");
    assert!(result["content"]
        .as_str()
        .unwrap()
        .contains("was not executed"));
    assert!(result["content"]
        .as_str()
        .unwrap()
        .contains("Retry with valid JSON"));
}

#[test]
fn chat_messages_preserve_native_context_roles() {
    let request = layered_model_request();

    let messages = openai_messages(&request);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "base instructions");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "current");
    assert_eq!(messages[2]["role"], "developer");
    assert!(messages[2]["content"]
        .as_str()
        .unwrap()
        .contains("developer environment"));
    assert!(!messages
        .iter()
        .any(|message| message.to_string().contains("must not be duplicated")));
}

#[test]
fn responses_split_system_instructions_from_developer_input() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();

    let instructions = prepared.body["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("base instructions"));
    assert!(instructions.contains(NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION));
    assert_eq!(prepared.body["input"][0]["role"], "user");
    assert_eq!(prepared.body["input"][0]["content"], "current");
    assert_eq!(prepared.body["input"][1]["role"], "developer");
    assert!(prepared.body["input"][1]["content"]
        .as_str()
        .unwrap()
        .contains("developer environment"));
}

#[test]
fn responses_keeps_harness_labels_out_of_wire_messages_and_tools_separate() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    let mut request = layered_model_request();
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::CapabilityCatalog,
        ContextRole::Developer,
        "opentopia:skill_catalog",
        r#"{"skills":[{"name":"review"}]}"#,
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    request.tool_candidates.push(ProviderToolCandidate::direct(
        "read_file",
        "Read a workspace file",
        json!({ "type": "object", "properties": {} }),
    ));

    let text_transports = [
        serde_json::to_string(&openai_messages(&request)).unwrap(),
        anthropic_system_instructions(&request),
        codex_developer_instructions(&request, false),
    ];
    for transport in text_transports {
        assert!(transport.contains("<context_data"));
        assert!(!transport.contains("\"authority\""));
        assert!(!transport.contains("\"lifecycle\""));
        assert!(!transport.contains("Read a workspace file"));
    }
    let dynamic_tools = codex_dynamic_tools(&request.tool_candidates);
    assert_eq!(dynamic_tools[0]["name"], "read_file");
    assert_eq!(dynamic_tools[0]["description"], "Read a workspace file");

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let input = prepared.body["input"].to_string();

    assert!(input.contains("<context_data"));
    assert!(!input.contains("\"authority\""));
    assert!(!input.contains("\"lifecycle\""));
    assert!(!input.contains("Read a workspace file"));
    assert_eq!(prepared.body["tools"][1]["name"], "read_file");
    assert_eq!(
        prepared.body["tools"][1]["description"],
        "Read a workspace file"
    );
}

#[test]
fn responses_keeps_volatile_system_context_behind_the_user_anchor() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    let mut request = layered_model_request();
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::Environment,
        ContextRole::System,
        "volatile-system-state",
        "volatile system context",
        ContextCacheScope::Turn,
        crate::model_context::ContextSensitivity::Workspace,
    ));

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert!(!prepared.body["instructions"]
        .as_str()
        .unwrap()
        .contains("volatile system context"));
    let input = prepared.body["input"].as_array().unwrap();
    let user_index = input
        .iter()
        .position(|item| item["role"] == "user")
        .unwrap();
    let volatile_index = input
        .iter()
        .position(|item| item.to_string().contains("volatile system context"))
        .unwrap();
    assert!(volatile_index > user_index);
    assert_eq!(input[volatile_index]["role"], "system");
}

#[test]
fn responses_explicit_cache_marks_last_reusable_developer_prefix() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.6");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
    let mut request = layered_model_request();
    request.instructions.items.push(ModelContextItem::text(
        crate::model_context::ContextItemKind::RepositoryInstructions,
        ContextRole::Developer,
        "AGENTS.md",
        "stable repository instructions",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert_eq!(prepared.body["prompt_cache_options"]["mode"], "explicit");
    assert_eq!(prepared.body["prompt_cache_options"]["ttl"], "30m");
    assert_eq!(
        prepared.body["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    assert!(prepared.body["input"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("stable repository instructions"));
    assert_eq!(
        prepared.body["input"][1]["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
}

#[test]
fn responses_stateful_request_sends_only_incremental_input() {
    let provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    let mut request = layered_model_request();
    request.input.conversation = vec![ModelConversationMessage {
        role: ModelConversationRole::User,
        content: "already stored".to_string(),
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    }];
    request.previous_response_id = Some("resp_parent".to_string());

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert_eq!(prepared.body["previous_response_id"], "resp_parent");
    assert_eq!(prepared.body["input"].as_array().unwrap().len(), 2);
    assert!(!prepared.body["input"]
        .to_string()
        .contains("already stored"));
    assert!(prepared.body["input"]
        .to_string()
        .contains("developer environment"));
    assert_eq!(prepared.body["input"][0]["content"], "current");
    assert_eq!(prepared.body["input"][1]["role"], "developer");
}

#[test]
fn responses_branch_marks_inherited_history_as_a_shared_prefix() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.6");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
    let mut request = layered_model_request();
    request.input.conversation = vec![ModelConversationMessage {
        role: ModelConversationRole::User,
        content: "parent fork point".to_string(),
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    }];
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        "opentopia:execution_branch",
        "review this branch",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert_eq!(prepared.body["input"][0]["role"], "developer");
    assert!(prepared.body["input"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("review this branch"));
    assert_eq!(prepared.body["input"][1]["role"], "user");
    assert_eq!(
        prepared.body["input"][1]["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    assert_eq!(prepared.body["input"][2]["role"], "user");
    assert_eq!(
        prepared.body["input"][2]["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    assert_eq!(prepared.body["input"][3]["role"], "developer");
}

#[test]
fn responses_maps_legacy_cache_retention_and_native_compaction() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-test");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Legacy24h);
    provider.compaction_threshold_tokens = Some(96_000);

    let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

    assert_eq!(prepared.body["prompt_cache_retention"], "24h");
    assert!(prepared.body.get("prompt_cache_options").is_none());
    assert_eq!(
        prepared.body["context_management"],
        json!([{"type": "compaction", "compact_threshold": 96_000}])
    );
}

#[test]
fn responses_omits_explicit_cache_fields_for_unsupported_models() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.4");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);

    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();

    assert!(prepared.body.get("prompt_cache_options").is_none());
    assert!(!prepared.body["input"]
        .to_string()
        .contains("prompt_cache_breakpoint"));
}

#[test]
fn responses_stable_only_policy_does_not_anchor_user_messages() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.6");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
    let mut request = layered_model_request();
    request.prompt_cache_breakpoint_policy = PromptCacheBreakpointPolicy::StableOnly;
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        "one-shot-contract",
        "stable one-shot contract",
        ContextCacheScope::Stable,
        crate::model_context::ContextSensitivity::Public,
    ));

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let input = prepared.body["input"].as_array().unwrap();
    let stable = input
        .iter()
        .find(|item| {
            item["content"]
                .to_string()
                .contains("stable one-shot contract")
        })
        .unwrap();
    assert_eq!(
        stable["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    let user = input.iter().find(|item| item["role"] == "user").unwrap();
    assert!(!user["content"]
        .to_string()
        .contains("prompt_cache_breakpoint"));
}

#[test]
fn responses_append_only_user_prefix_survives_changing_turn_context() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.6");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);

    let mut first = layered_model_request();
    first.instructions.items.push(ModelContextItem::text(
        ContextItemKind::RepositoryInstructions,
        ContextRole::Developer,
        "AGENTS.md",
        "lineage header",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    first.input.current_user.message = "U1".to_string();
    let first = provider.prepare(Uuid::nil(), first).unwrap();

    let mut second = layered_model_request();
    second.instructions.items.push(ModelContextItem::text(
        ContextItemKind::RepositoryInstructions,
        ContextRole::Developer,
        "AGENTS.md",
        "lineage header",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    second.instructions.items.retain(|item| {
        item.source != "opentopia:environment" || item.text_content() != "developer environment"
    });
    second.instructions.items.push(ModelContextItem::text(
        ContextItemKind::Environment,
        ContextRole::Developer,
        "opentopia:environment",
        "changed turn state",
        ContextCacheScope::Turn,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    second.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "U1".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "A1".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
    ];
    second.input.current_user.message = "U2".to_string();
    let second = provider.prepare(Uuid::nil(), second).unwrap();

    let first_input = first.body["input"].as_array().unwrap();
    let second_input = second.body["input"].as_array().unwrap();
    assert_eq!(&first_input[..=1], &second_input[..=1]);
    assert!(first_input[2].to_string().contains("developer environment"));
    assert!(second_input
        .last()
        .unwrap()
        .to_string()
        .contains("changed turn state"));
}
