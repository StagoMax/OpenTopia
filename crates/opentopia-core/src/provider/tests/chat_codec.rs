#[test]
fn portable_chat_envelope_keeps_volatile_context_after_the_user_anchor() {
    let request = layered_model_request();

    let messages = openai_portable_messages(&request);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], "base instructions");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "current");
    assert_eq!(messages[2]["role"], "user");
    assert!(messages[2]["content"]
        .as_str()
        .unwrap()
        .contains("developer environment"));
}

#[test]
fn chat_tool_history_appends_provider_state_without_reordering_runtime_observations() {
    let call = |id: &str, name: &str| ProviderToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({ "id": id }),
    };
    let result = |call_id: &str, name: &str| ProviderToolResult {
        call_id: call_id.to_string(),
        name: name.to_string(),
        output: format!("{name} completed"),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    };
    let provider_state = |call_id: &str| {
        json!({
            "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
            "content": "",
            "reasoning_content": format!("reason about {call_id}"),
            "tool_call_ids": [call_id],
        })
    };

    // Round one ends with a runtime-owned observation. On round two the model
    // has emitted another provider-owned state. The prior encoder replayed all
    // provider state before every runtime observation, inserting `call_new`
    // before `runtime_old` and destroying the cached suffix.
    let mut first = model_request();
    first.input.tool_calls = vec![
        call("provider_initial", "read_file"),
        call("runtime_old", "runtime_step_reminder"),
    ];
    first.input.tool_results = vec![
        result("provider_initial", "read_file"),
        result("runtime_old", "runtime_step_reminder"),
    ];
    first.previous_response_items = vec![provider_state("provider_initial")];

    let mut second = first.clone();
    second
        .input
        .tool_calls
        .push(call("provider_new", "apply_patch"));
    second
        .input
        .tool_results
        .push(result("provider_new", "apply_patch"));
    second
        .previous_response_items
        .push(provider_state("provider_new"));

    for replay_chat_reasoning in [false, true] {
        let first_messages = openai_messages_with_reasoning(&first, replay_chat_reasoning);
        let second_messages = openai_messages_with_reasoning(&second, replay_chat_reasoning);
        assert_eq!(
            second_messages[..first_messages.len()],
            first_messages,
            "adding provider-owned state must append, never insert into prior history"
        );
    }

    let second_messages = openai_messages_with_reasoning(&second, true);
    assert_eq!(
        second_messages
            .iter()
            .filter_map(|message| message["tool_calls"].as_array())
            .flat_map(|calls| calls.iter())
            .map(|call| call["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["provider_initial", "provider_new"]
    );
    assert!(second_messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("runtime_old"))
    }));
}

#[test]
fn chat_cross_turn_transcript_is_extended_without_reclassifying_history() {
    let call = ProviderToolCall {
        id: "call_previous".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "previous.txt" }),
    };
    let result = ProviderToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        output: "previous output".to_string(),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    };
    let mut previous = layered_model_request();
    previous.input.current_user.message = "first turn".to_string();
    previous.input.tool_calls = vec![call.clone()];
    previous.input.tool_results = vec![result.clone()];
    previous.previous_response_items = vec![json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": "",
        "tool_call_ids": [&call.id],
    })];

    for (format, encode) in [
        (
            OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT,
            openai_messages as fn(&ModelRequest) -> Vec<Value>,
        ),
        (
            OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT,
            openai_portable_messages as fn(&ModelRequest) -> Vec<Value>,
        ),
    ] {
        let previous_request = encode(&previous);
        let mut completed_transcript = previous_request.clone();
        completed_transcript.push(json!({
            "role": "assistant",
            "content": "first answer",
        }));

        let mut next = layered_model_request();
        next.input.current_user.message = "second turn".to_string();
        // This is the legacy durable projection. It intentionally has a
        // different shape from the prior wire request; the cursor transcript
        // must own replay ordering whenever it is available.
        next.input.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "reconstructed history that must not be inserted".to_string(),
            content_parts: Vec::new(),
            tool_calls: vec![call.clone()],
            tool_results: vec![result.clone()],
        }];
        next.provider_transcript = Some(ProviderWireTranscript {
            format: format.to_string(),
            items: completed_transcript.clone(),
        });

        let next_request = encode(&next);
        assert_eq!(
            next_request[..completed_transcript.len()],
            completed_transcript,
            "the next turn must append to the exact completed wire transcript"
        );
        assert_eq!(next_request[completed_transcript.len()]["role"], "user");
        assert_eq!(
            next_request[completed_transcript.len()]["content"],
            "second turn"
        );
        assert!(!next_request.iter().any(|message| {
            message["content"] == "reconstructed history that must not be inserted"
        }));
        assert_eq!(
            previous_request,
            next_request[..previous_request.len()],
            "the full prior request remains a strict prefix across the turn boundary"
        );
    }
}

#[test]
fn failed_chat_request_transcript_is_the_next_turns_strict_prefix() {
    for (format, encode) in [
        (
            OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT,
            openai_messages as fn(&ModelRequest) -> Vec<Value>,
        ),
        (
            OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT,
            openai_portable_messages as fn(&ModelRequest) -> Vec<Value>,
        ),
    ] {
        let mut failed = layered_model_request();
        failed.input.current_user.message = "request that failed".to_string();
        let failed_request = encode(&failed);

        let mut next = layered_model_request();
        next.input.current_user.message = "continue after failure".to_string();
        next.input.conversation = vec![ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "reconstructed failed turn".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        }];
        next.provider_transcript = Some(ProviderWireTranscript {
            format: format.to_string(),
            items: failed_request.clone(),
        });

        let next_request = encode(&next);
        assert_eq!(
            next_request[..failed_request.len()],
            failed_request,
            "a failed provider request must remain the exact next-request prefix"
        );
        assert_eq!(next_request[failed_request.len()]["role"], "user");
        assert_eq!(
            next_request[failed_request.len()]["content"],
            "continue after failure"
        );
        assert!(!next_request
            .iter()
            .any(|message| message["content"] == "reconstructed failed turn"));
    }
}

#[test]
fn portable_chat_envelope_preserves_structured_cross_turn_tool_history() {
    let mut request = model_request();
    request.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "inspect both files".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: String::new(),
            content_parts: Vec::new(),
            tool_calls: vec![
                ProviderToolCall {
                    id: "call_a".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "a.txt"}),
                },
                ProviderToolCall {
                    id: "call_b".to_string(),
                    name: "read_file".to_string(),
                    arguments: json!({"path": "b.txt"}),
                },
            ],
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Tool,
            content: String::new(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: vec![
                ProviderToolResult {
                    call_id: "call_a".to_string(),
                    name: "read_file".to_string(),
                    output: "A".to_string(),
                    content: Vec::new(),
                    is_error: false,
                    metadata: json!({}),
                },
                ProviderToolResult {
                    call_id: "call_b".to_string(),
                    name: "read_file".to_string(),
                    output: "B".to_string(),
                    content: Vec::new(),
                    is_error: false,
                    metadata: json!({}),
                },
            ],
        },
    ];
    request.previous_response_items = vec![json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": "",
        "reasoning_content": "inspect in parallel",
        "tool_call_ids": ["call_a", "call_b"],
    })];

    let messages = openai_portable_messages_with_reasoning(&request, true);

    assert_eq!(
        messages
            .iter()
            .map(|message| message["role"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["system", "user", "assistant", "tool", "tool", "user"]
    );
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_a");
    assert_eq!(messages[2]["tool_calls"][1]["id"], "call_b");
    assert_eq!(messages[2]["reasoning_content"], "inspect in parallel");
    assert_eq!(messages[3]["tool_call_id"], "call_a");
    assert_eq!(messages[4]["tool_call_id"], "call_b");

    let messages_without_reasoning = openai_portable_messages_with_reasoning(&request, false);
    assert_eq!(
        messages_without_reasoning[2]["tool_calls"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(messages_without_reasoning[2]["content"], "");
    assert!(messages_without_reasoning[2]
        .get("reasoning_content")
        .is_none());
}

#[test]
fn deepseek_lowers_runtime_observations_without_fabricating_assistant_reasoning() {
    let mut request = model_request();
    request.input.tool_calls = vec![
        ProviderToolCall {
            id: "call_provider".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "status.txt" }),
        },
        ProviderToolCall {
            id: "step_reminder_1".to_string(),
            name: "runtime_step_reminder".to_string(),
            arguments: json!({ "stage": "repeated_tool_calls" }),
        },
    ];
    request.input.tool_results = vec![
        ProviderToolResult {
            call_id: "call_provider".to_string(),
            name: "read_file".to_string(),
            output: "done".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({}),
        },
        ProviderToolResult {
            call_id: "step_reminder_1".to_string(),
            name: "runtime_step_reminder".to_string(),
            output: "Repeated tool-call telemetry".to_string(),
            content: Vec::new(),
            is_error: false,
            metadata: json!({ "runtimeObservation": "step_reminder" }),
        },
    ];
    request.previous_response_items = vec![
        json!({
            "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
            "content": "I will inspect the status.",
            "reasoning_content": "The status file is needed.",
            "tool_call_ids": ["call_provider"],
        }),
        json!({
            "type": "function_call",
            "call_id": "step_reminder_1",
            "name": "runtime_step_reminder",
            "arguments": "{}",
        }),
    ];
    let provider = OpenAiCompatibleProvider::new(
        "https://api.deepseek.com/v1",
        "test-key",
        "deepseek-v4-flash",
    );

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let messages = prepared.body["messages"].as_array().unwrap();
    let assistant_tool_messages = messages
        .iter()
        .filter(|message| message.get("tool_calls").is_some())
        .collect::<Vec<_>>();

    assert_eq!(assistant_tool_messages.len(), 1);
    assert_eq!(
        assistant_tool_messages[0]["reasoning_content"],
        "The status file is needed."
    );
    assert_eq!(
        assistant_tool_messages[0]["tool_calls"][0]["id"],
        "call_provider"
    );
    assert!(messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("runtime_step_reminder"))
    }));
    assert!(!messages.iter().any(|message| {
        message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| calls.iter().any(|call| call["id"] == "step_reminder_1"))
    }));
}

#[test]
fn deepseek_replays_a_present_empty_reasoning_field_verbatim() {
    let mut request = model_request();
    request.input.tool_calls = vec![ProviderToolCall {
        id: "call_empty_reasoning".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "status.txt" }),
    }];
    request.previous_response_items = parse_model_response_body(&json!({
        "choices": [{
            "message": {
                "content": "",
                "reasoning_content": "",
                "tool_calls": [{
                    "id": "call_empty_reasoning",
                    "type": "function",
                    "function": { "name": "read_file", "arguments": "{}" }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }))
    .unwrap()
    .provider_items;

    let messages = openai_messages_with_reasoning(&request, true);
    let assistant = messages
        .iter()
        .find(|message| message.get("tool_calls").is_some())
        .unwrap();

    assert_eq!(assistant.get("reasoning_content"), Some(&json!("")));
}

#[test]
fn deepseek_lowers_legacy_provider_state_that_has_no_reasoning_field() {
    let mut request = model_request();
    request.input.tool_calls = vec![ProviderToolCall {
        id: "legacy_provider_call".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "status.txt" }),
    }];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "legacy_provider_call".to_string(),
        name: "read_file".to_string(),
        output: "done".to_string(),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    }];
    request.previous_response_items = vec![json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": "I inspected the file.",
        "tool_call_ids": ["legacy_provider_call"],
    })];
    let provider = OpenAiCompatibleProvider::new(
        "https://api.deepseek.com/v1",
        "test-key",
        "deepseek-v4-flash",
    );

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let messages = prepared.body["messages"].as_array().unwrap();
    assert!(!messages
        .iter()
        .any(|message| message.get("tool_calls").is_some()));
    assert!(messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("legacy_provider_call"))
    }));
}

#[test]
fn chat_reasoning_replay_is_driven_by_the_adapter_profile_not_the_model_name() {
    let base_url = "https://relay.example/v1";
    let model = "opaque-thinking-model";
    let mut settings = ProviderSettings {
        base_url: base_url.to_string(),
        model: model.to_string(),
        ..ProviderSettings::default()
    };
    settings.apply_adapter_profile(ProviderAdapterProfile {
        profile_version: PROVIDER_ADAPTER_PROFILE_VERSION,
        base_url: base_url.to_string(),
        model: model.to_string(),
        adapter: ProviderAdapterKind::OpenAiChat,
        instruction_encoding: ProviderInstructionEncoding::PortableChatEnvelope,
        reasoning_protocol: ProviderReasoningProtocol::ChatReasoningEffort,
        message_protocol: ProviderMessageProtocolCapabilities {
            requires_reasoning_content_for_tool_calls: true,
        },
        output_protocol: ProviderOutputProtocolCapabilities::default(),
        tool_protocol: ProviderToolProtocolCapabilities::default(),
        checked_at: Utc::now(),
    });

    let mut request = model_request();
    request.input.tool_calls = vec![ProviderToolCall {
        id: "profile_call".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "status.txt" }),
    }];
    request.previous_response_items = vec![json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": "",
        "reasoning_content": "profile-owned reasoning",
        "tool_call_ids": ["profile_call"],
    })];

    let profiled = OpenAiCompatibleProvider::new(base_url, "test-key", model)
        .with_generation_settings(&settings)
        .prepare(Uuid::nil(), request.clone())
        .unwrap();
    let profiled_assistant = profiled.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_calls").is_some())
        .unwrap();
    assert_eq!(
        profiled_assistant["reasoning_content"],
        "profile-owned reasoning"
    );

    let mut name_only_provider =
        OpenAiCompatibleProvider::new("https://relay.example/v1", "test-key", "deepseek-v4-flash");
    name_only_provider.chat_codec.instruction_encoding = ProviderInstructionEncoding::NativeRoles;
    let name_only = name_only_provider.prepare(Uuid::nil(), request).unwrap();
    let name_only_assistant = name_only.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message.get("tool_calls").is_some())
        .unwrap();
    assert!(name_only_assistant.get("reasoning_content").is_none());
}

#[test]
fn deepseek_lowers_legacy_conversation_tools_without_reasoning_state() {
    let mut request = model_request();
    request.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: String::new(),
            content_parts: Vec::new(),
            tool_calls: vec![ProviderToolCall {
                id: "legacy_call".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "legacy.txt" }),
            }],
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Tool,
            content: String::new(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: vec![ProviderToolResult {
                call_id: "legacy_call".to_string(),
                name: "read_file".to_string(),
                output: "legacy output".to_string(),
                content: Vec::new(),
                is_error: false,
                metadata: json!({}),
            }],
        },
    ];
    let provider = OpenAiCompatibleProvider::new(
        "https://api.deepseek.com/v1",
        "test-key",
        "deepseek-reasoner",
    );

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let messages = prepared.body["messages"].as_array().unwrap();
    assert!(!messages
        .iter()
        .any(|message| message.get("tool_calls").is_some()));
    assert!(messages.iter().any(|message| {
        message["role"] == "user"
            && message["content"]
                .as_str()
                .is_some_and(|content| content.contains("legacy_call"))
    }));
}

#[test]
fn unprobed_relay_starts_with_the_portable_chat_envelope() {
    let provider =
        OpenAiCompatibleProvider::new("https://api.example.test/v1", "test-key", "unknown-model");
    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();
    let messages = prepared.body["messages"].as_array().unwrap();

    assert_eq!(messages[0]["role"], "system");
    assert!(!messages
        .iter()
        .any(|message| message["role"] == "developer"));
}

#[test]
fn glm_disables_thinking_with_native_control() {
    let mut provider =
        OpenAiCompatibleProvider::new("https://api.example.test/v1", "test-key", "glm-5.2");
    provider.reasoning_protocol = ProviderReasoningProtocol::ChatThinkingReasoningEffort;
    provider.reasoning_effort = Some("none".to_string());

    let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

    assert_eq!(prepared.body["thinking"], json!({ "type": "disabled" }));
    assert!(prepared.body.get("reasoning_effort").is_none());
}

#[test]
fn glm_enables_thinking_with_native_control() {
    let mut provider =
        OpenAiCompatibleProvider::new("https://api.example.test/v1", "test-key", "glm-5.2");
    provider.reasoning_protocol = ProviderReasoningProtocol::ChatThinkingReasoningEffort;
    provider.reasoning_effort = Some("high".to_string());

    let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

    assert_eq!(prepared.body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(prepared.body["reasoning_effort"], "high");
}

#[test]
fn unprobed_new_model_does_not_guess_a_vendor_reasoning_protocol() {
    let provider = OpenAiCompatibleProvider::new("https://relay.example/v1", "test-key", "glm-5.3");

    let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

    assert!(prepared.body.get("thinking").is_none());
    assert!(prepared.body.get("reasoning_effort").is_none());
}

#[test]
fn deepseek_v4_maps_thinking_effort_and_omits_incompatible_tool_choice() {
    let mut provider = OpenAiCompatibleProvider::new(
        "https://api.deepseek.com/v1",
        "test-key",
        "deepseek-v4-flash",
    );
    provider.reasoning_protocol = ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice;
    provider.temperature = Some(0.7);
    provider.reasoning_effort = Some("xhigh".to_string());
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        }),
        ..Default::default()
    }];

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert_eq!(prepared.body["thinking"], json!({ "type": "enabled" }));
    assert_eq!(prepared.body["reasoning_effort"], "max");
    assert!(prepared.body.get("temperature").is_none());
    assert!(prepared.body.get("tool_choice").is_none());
}

#[test]
fn deepseek_v4_can_disable_thinking() {
    let mut provider =
        OpenAiCompatibleProvider::new("https://api.deepseek.com/v1", "test-key", "deepseek-v4-pro");
    provider.reasoning_protocol = ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice;
    provider.temperature = Some(0.7);
    provider.reasoning_effort = Some("none".to_string());
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: json!({ "type": "object", "properties": {} }),
        ..Default::default()
    }];

    let prepared = provider.prepare(Uuid::nil(), request).unwrap();

    assert_eq!(prepared.body["thinking"], json!({ "type": "disabled" }));
    assert!(prepared.body.get("reasoning_effort").is_none());
    assert_eq!(prepared.body["temperature"], 0.7);
    assert_eq!(prepared.body["tool_choice"], "auto");
}

#[test]
fn deepseek_tool_history_replays_reasoning_content() {
    let mut request = model_request();
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
    request.previous_response_items = vec![json!({
        "type": OPENAI_CHAT_ASSISTANT_STATE_TYPE,
        "content": "I will inspect the file.",
        "reasoning_content": "The file is needed for the next step.",
        "tool_call_ids": ["call_1"],
    })];

    let messages = openai_messages_with_reasoning(&request, true);
    let assistant = messages
        .iter()
        .find(|message| message.get("tool_calls").is_some())
        .expect("assistant tool-call message");

    assert_eq!(assistant["content"], "I will inspect the file.");
    assert_eq!(
        assistant["reasoning_content"],
        "The file is needed for the next step."
    );
    assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
}

#[test]
fn chat_response_preserves_reasoning_for_tool_replay() {
    let response = parse_model_response_body(&json!({
        "id": "chatcmpl_observation_only",
        "choices": [{
            "message": {
                "content": "",
                "reasoning_content": "I need the file.",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "read_file", "arguments": "{}" }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }))
    .unwrap();

    assert_eq!(response.provider_items.len(), 1);
    assert_eq!(
        response.provider_items[0]["type"],
        OPENAI_CHAT_ASSISTANT_STATE_TYPE
    );
    assert_eq!(
        response.provider_items[0]["reasoning_content"],
        "I need the file."
    );
    assert!(response.response_id.is_none());
}

#[test]
fn chat_response_does_not_fabricate_an_absent_reasoning_field() {
    let response = parse_model_response_body(&json!({
        "choices": [{
            "message": {
                "content": "",
                "tool_calls": [{
                    "id": "call_without_reasoning",
                    "type": "function",
                    "function": { "name": "read_file", "arguments": "{}" }
                }]
            },
            "finish_reason": "tool_calls"
        }]
    }))
    .unwrap();

    assert_eq!(response.provider_items.len(), 1);
    assert!(response.provider_items[0]
        .get("reasoning_content")
        .is_none());
}

#[test]
fn plain_tool_result_text_is_serialized_and_estimated_once() {
    let duplicate = ProviderToolResult {
        call_id: "call_text".to_string(),
        name: "read_file".to_string(),
        output: "same text".to_string(),
        content: vec![ModelInputContent::Text {
            text: "same text".to_string(),
        }],
        is_error: false,
        metadata: json!({}),
    };
    let legacy = ProviderToolResult {
        content: Vec::new(),
        ..duplicate.clone()
    };

    let payload: Value = serde_json::from_str(&provider_tool_result_content(&duplicate)).unwrap();
    assert_eq!(payload["output"], "same text");
    assert!(payload.get("content").is_none());
    assert_eq!(
        estimate_provider_tool_results(std::slice::from_ref(&duplicate)),
        estimate_provider_tool_results(std::slice::from_ref(&legacy))
    );
    assert_eq!(
        anthropic_tool_result(&duplicate)["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn image_capability_diagnostics_report_actual_image_count() {
    let request = model_request();
    assert_eq!(request_image_part_count(&request), 0);

    let mut request_with_image = request;
    request_with_image.input.current_user.content =
        vec![ModelContentPart::image("image/png", vec![1])];
    assert_eq!(request_image_part_count(&request_with_image), 1);
}

#[test]
fn detects_provider_image_capability_rejection() {
    assert!(provider_rejected_image_input(
        r#"{"message":"当前请求包含图片内容，上游不支持"}"#
    ));
    assert!(provider_rejected_image_input(
        r#"{"error":"vision input is not supported"}"#
    ));
    assert!(!provider_rejected_image_input(
        r#"{"error":"unsupported developer role"}"#
    ));
}

#[test]
fn detects_explicit_developer_role_rejection() {
    assert!(provider_rejected_developer_messages(
        r#"{"error":{"message":"messages[1].role: unknown variant `developer`, expected one of `system`, `user`, `assistant`, `tool`, `latest_reminder`"}}"#
    ));
    assert!(provider_rejected_developer_messages(
        r#"{"error":"developer role is not supported"}"#
    ));
    assert!(!provider_rejected_developer_messages(
        r#"{"error":"invalid model"}"#
    ));
}

#[test]
fn serializes_native_user_images_and_structured_tool_content() {
    let mut request = model_request();
    request.input.current_user.content = vec![
        ModelInputContent::image("image/png", vec![0x89, b'P', b'N', b'G']),
        ModelInputContent::json(json!({ "selection": 4 })),
        ModelInputContent::resource(
            "file:///workspace/spec.pdf",
            Some("application/pdf".to_string()),
            Some("spec.pdf".to_string()),
        ),
    ];
    request.input.tool_calls = vec![ProviderToolCall {
        id: "call_1".to_string(),
        name: "inspect".to_string(),
        arguments: json!({}),
    }];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "call_1".to_string(),
        name: "inspect".to_string(),
        output: "legacy".to_string(),
        content: vec![ModelInputContent::json(json!({ "ready": true }))],
        is_error: false,
        metadata: json!({}),
    }];

    let messages = openai_messages(&request);
    let user = &messages[1];
    assert_eq!(user["role"], "user");
    assert_eq!(
        user["content"][0],
        json!({ "type": "text", "text": "current" })
    );
    assert_eq!(user["content"][1]["type"], "image_url");
    assert_eq!(
        user["content"][1]["image_url"]["url"],
        "data:image/png;base64,iVBORw=="
    );
    assert_eq!(user["content"][2]["text"], "{\"selection\":4}");
    assert!(user["content"][3]["text"]
        .as_str()
        .unwrap()
        .contains("file:///workspace/spec.pdf"));

    let tool_content: Value =
        serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
    assert_eq!(tool_content["content"][0]["type"], "json");
    assert_eq!(
        tool_content["content"][0]["value"],
        json!({ "ready": true })
    );
}

#[test]
fn compacts_completed_tool_history_for_strict_compatible_providers() {
    let mut request = model_request();
    request.input.tool_calls = vec![ProviderToolCall {
        id: "call_1".to_string(),
        name: "read_file".to_string(),
        arguments: json!({ "path": "SPEC.md" }),
    }];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "call_1".to_string(),
        name: "read_file".to_string(),
        output: "contract".to_string(),
        content: vec![ModelInputContent::text("contract")],
        is_error: false,
        metadata: json!({ "bytes": 8 }),
    }];

    let messages = openai_portable_messages(&request);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["role"], "user");
    let history = messages[2]["content"].as_str().unwrap();
    assert!(history.contains("read_file"));
    assert!(history.contains("SPEC.md"));
    assert!(history.contains("contract"));
    assert!(!messages.iter().any(|message| message["role"] == "tool"));
}

#[test]
fn appends_native_tool_images_after_all_tool_messages() {
    let mut request = model_request();
    request.input.tool_calls = vec![
        ProviderToolCall {
            id: "call_first".to_string(),
            name: "browser_screenshot".to_string(),
            arguments: json!({}),
        },
        ProviderToolCall {
            id: "call_second".to_string(),
            name: "inspect_page".to_string(),
            arguments: json!({}),
        },
    ];
    request.input.tool_results = vec![
        ProviderToolResult {
            call_id: "call_first".to_string(),
            name: "browser_screenshot".to_string(),
            output: "first screenshot".to_string(),
            content: vec![ModelInputContent::image(
                "image/png",
                vec![0x89, b'P', b'N', b'G'],
            )],
            is_error: false,
            metadata: json!({}),
        },
        ProviderToolResult {
            call_id: "call_second".to_string(),
            name: "inspect_page".to_string(),
            output: "page inspected".to_string(),
            content: vec![
                ModelInputContent::json(json!({ "ready": true })),
                ModelInputContent::image("image/jpeg", vec![0xff, 0xd8, 0xff]),
            ],
            is_error: false,
            metadata: json!({}),
        },
    ];

    let messages = openai_messages(&request);

    assert_eq!(messages.len(), 7);
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_first");
    assert_eq!(messages[3]["tool_call_id"], "call_first");
    assert_eq!(messages[4]["role"], "assistant");
    assert_eq!(messages[4]["tool_calls"][0]["id"], "call_second");
    assert_eq!(messages[5]["tool_call_id"], "call_second");
    assert_eq!(messages[6]["role"], "user");

    let first_tool_content = messages[3]["content"].as_str().unwrap();
    let second_tool_content = messages[5]["content"].as_str().unwrap();
    assert!(!first_tool_content.contains("data:"));
    assert!(!second_tool_content.contains("data:"));
    let first_tool_content: Value = serde_json::from_str(first_tool_content).unwrap();
    let second_tool_content: Value = serde_json::from_str(second_tool_content).unwrap();
    assert_eq!(
        first_tool_content["content"][0],
        json!({
            "type": "image",
            "contentType": "image/png",
            "bytes": 4,
            "delivery": "native_companion"
        })
    );
    assert_eq!(second_tool_content["content"][0]["type"], "json");
    assert_eq!(
        second_tool_content["content"][1]["delivery"],
        "native_companion"
    );

    let companion = messages[6]["content"].as_array().unwrap();
    assert_eq!(companion.len(), 4);
    assert_eq!(
        companion[0],
        json!({
            "type": "text",
            "text": "Tool image output: browser_screenshot (call call_first)."
        })
    );
    assert_eq!(companion[1]["type"], "image_url");
    assert_eq!(
        companion[1]["image_url"]["url"],
        "data:image/png;base64,iVBORw=="
    );
    assert_eq!(
        companion[2]["text"],
        "Tool image output: inspect_page (call call_second)."
    );
    assert_eq!(
        companion[3]["image_url"]["url"],
        "data:image/jpeg;base64,/9j/"
    );
}

#[test]
fn responses_tool_images_remain_inside_function_call_output() {
    let result = ProviderToolResult {
        call_id: "call_attachment".to_string(),
        name: "view_attachment".to_string(),
        output: "attachment image output".to_string(),
        content: vec![ModelInputContent::image(
            "image/png",
            vec![0x89, b'P', b'N', b'G'],
        )],
        is_error: false,
        metadata: json!({ "provenance": "user_attachment" }),
    };

    let output = responses_tool_result_output(&result);
    let items = output.as_array().expect("typed function output items");
    assert_eq!(items[0]["type"], "input_text");
    assert!(items[0]["text"]
        .as_str()
        .unwrap()
        .contains("user_attachment"));
    assert_eq!(items[1]["type"], "input_image");
    assert_eq!(items[1]["detail"], "original");
    assert_eq!(items[1]["image_url"], "data:image/png;base64,iVBORw==");
}
