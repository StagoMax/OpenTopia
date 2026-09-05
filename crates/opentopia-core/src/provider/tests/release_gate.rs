#[test]
fn compiled_contract_restores_only_provider_introduced_nulls() {
    let candidate = ProviderToolCandidate::direct(
        "records",
        "Update records.",
        json!({
            "type": "object",
            "properties": {
                "columns": { "type": "array", "items": { "type": "string" } },
                "note": { "type": "string" },
                "explicitNullable": { "type": ["string", "null"] }
            },
            "additionalProperties": false
        }),
    );
    let capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };

    let compiled = compile_openai_tools(&[candidate], capabilities);
    let contract = &compiled.contracts[0];
    assert_eq!(
        compiled.tools[0]["function"]["parameters"],
        contract.wire_input_schema
    );
    let mut arguments = json!({
        "columns": null,
        "note": null,
        "explicitNullable": null
    });
    assert_eq!(
        tool_input_schema_error(&contract.wire_input_schema, &arguments, "arguments"),
        None
    );

    normalize_provider_arguments(
        &contract.logical_input_schema,
        &contract.wire_input_schema,
        &mut arguments,
    );

    assert_eq!(arguments, json!({ "explicitNullable": null }));
    assert_eq!(
        tool_input_schema_error(&contract.logical_input_schema, &arguments, "arguments"),
        None
    );
}

#[test]
fn prepared_openai_requests_retain_the_exact_advertised_contract() {
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate::direct(
        "records",
        "Update records.",
        json!({
            "type": "object",
            "properties": {
                "columns": { "type": "array", "items": { "type": "string" } }
            },
            "additionalProperties": false
        }),
    )];

    let mut chat =
        OpenAiCompatibleProvider::new("https://example.test/v1", "test-key", "strict-model");
    chat.tool_protocol.strict_function_tools = ProviderFeatureSupport::Supported;
    let chat = chat.prepare(Uuid::nil(), request.clone()).unwrap();
    assert_eq!(chat.tool_contracts.len(), 1);
    assert_eq!(
        chat.body["tools"][0]["function"]["parameters"],
        chat.tool_contracts[0].wire_input_schema
    );

    let mut responses =
        OpenAiResponsesProvider::new("https://example.test/v1", "test-key", "strict-model");
    responses.native_web_search = false;
    responses.tool_protocol.strict_function_tools = ProviderFeatureSupport::Supported;
    let responses = responses.prepare(Uuid::nil(), request).unwrap();
    assert_eq!(responses.tool_contracts.len(), 1);
    assert_eq!(
        responses.body["tools"][0]["parameters"],
        responses.tool_contracts[0].wire_input_schema
    );
}

#[test]
fn compiled_contract_normalizes_the_selected_root_union_branch() {
    let candidate = ProviderToolCandidate::direct(
        "spreadsheet",
        "Spreadsheet operations.",
        json!({
            "type": "object",
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["write_columns"] },
                        "path": { "type": "string" },
                        "columns": {
                            "type": "array",
                            "items": { "type": "array", "items": { "type": "string" } },
                            "minItems": 1
                        }
                    },
                    "required": ["action", "columns"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["filter_rows"] },
                        "path": { "type": "string" },
                        "conditions": {
                            "type": "array",
                            "items": { "type": "object" },
                            "minItems": 1
                        }
                    },
                    "required": ["action", "conditions"],
                    "additionalProperties": false
                }
            ]
        }),
    );
    let capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };
    let compiled = compile_openai_tools(&[candidate], capabilities);
    let contract = &compiled.contracts[0];
    let mut arguments = json!({
        "action": "write_columns",
        "path": null,
        "columns": [["sku", "quantity"]]
    });
    assert_eq!(
        tool_input_schema_error(&contract.wire_input_schema, &arguments, "arguments"),
        None
    );

    normalize_provider_arguments(
        &contract.logical_input_schema,
        &contract.wire_input_schema,
        &mut arguments,
    );

    assert_eq!(
        arguments,
        json!({
            "action": "write_columns",
            "columns": [["sku", "quantity"]]
        })
    );
    assert_eq!(
        tool_input_schema_error(&contract.logical_input_schema, &arguments, "arguments"),
        None
    );
}

#[test]
fn root_union_wire_contract_is_widened_but_logical_validation_stays_exact() {
    let tool = BackgroundOutputTool;
    let candidate = ProviderToolCandidate::direct(
        Tool::name(&tool),
        Tool::description(&tool),
        Tool::schema(&tool),
    );
    let capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };

    let compiled = compile_openai_tools(&[candidate], capabilities);
    let contract = &compiled.contracts[0];
    assert!(contract.logical_input_schema.get("oneOf").is_some());
    assert!(contract.wire_input_schema.get("oneOf").is_none());
    assert!(contract.wire_input_schema.get("anyOf").is_none());
    assert_eq!(contract.wire_input_schema["type"], "object");
    assert_eq!(compiled.tools[0]["function"]["strict"], false);

    // The widened provider shape allows all known action fields so a narrow
    // endpoint can advertise it as one object. The original tagged union still
    // rejects cross-action arguments before any tool executes.
    let cross_action_arguments = json!({
        "action": "list",
        "job_id": "job-from-another-action"
    });
    assert_eq!(
        tool_input_schema_error(
            &contract.wire_input_schema,
            &cross_action_arguments,
            "arguments"
        ),
        None
    );
    assert!(tool_input_schema_error(
        &contract.logical_input_schema,
        &cross_action_arguments,
        "arguments"
    )
    .is_some());
    assert!(Tool::input_error(&tool, &cross_action_arguments).is_some());
}

#[test]
fn atomic_spreadsheet_wire_contract_is_accepted_by_the_typed_runtime_contract() {
    let tool = SpreadsheetWriteRangeTool;
    let candidate = ProviderToolCandidate::direct(
        Tool::name(&tool),
        Tool::description(&tool),
        Tool::schema(&tool),
    );
    let capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };
    let compiled = compile_openai_tools(&[candidate], capabilities);
    let contract = &compiled.contracts[0];
    let mut arguments = json!({
        "path": "orders.xlsx",
        "template": null,
        "sheet": "Orders",
        "start": "A1",
        "rows": [[{ "type": "string", "value": "sku" }]]
    });
    assert_eq!(
        tool_input_schema_error(&contract.wire_input_schema, &arguments, "arguments"),
        None
    );

    normalize_provider_arguments(
        &contract.logical_input_schema,
        &contract.wire_input_schema,
        &mut arguments,
    );

    assert_eq!(Tool::input_error(&tool, &arguments), None);
    assert!(arguments["rows"].is_array());
    assert!(arguments.get("documentId").is_none());
}

#[test]
fn schema_union_errors_report_each_rejected_shape() {
    let schema = json!({
        "type": "object",
        "anyOf": [
            {
                "type": "object",
                "properties": { "patch": { "type": "string" } },
                "required": ["patch"],
                "additionalProperties": false
            },
            {
                "type": "object",
                "properties": { "operation": { "type": "object" } },
                "required": ["operation"],
                "additionalProperties": false
            }
        ]
    });

    assert_eq!(
            tool_input_schema_error(&schema, &json!({ "diff": "@@" }), "arguments")
                .as_deref(),
            Some(
                "arguments does not match any allowed input shape (option 1: arguments.patch is required; option 2: arguments.operation is required)"
            )
        );
}

#[test]
fn tool_argument_keys_follow_the_advertised_schema_without_alias_tables() {
    let schema = json!({
        "type": "object",
        "properties": {
            "yieldTimeMs": { "type": "integer" },
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "depends_on": { "type": "array", "items": { "type": "string" } }
                    },
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    });
    let mut arguments = json!({
        "yield_time_ms": 30_000,
        "rows": [{ "depends-on": ["step_1"] }]
    });

    let normalized = normalize_tool_argument_keys(&schema, &mut arguments);

    assert_eq!(
        arguments,
        json!({
            "yieldTimeMs": 30_000,
            "rows": [{ "depends_on": ["step_1"] }]
        })
    );
    assert_eq!(normalized.len(), 2);
    assert_eq!(
        tool_input_schema_error(&schema, &arguments, "arguments"),
        None
    );
}

#[test]
fn tool_argument_key_normalization_preserves_unknown_and_ambiguous_fields() {
    let schema = json!({
        "type": "object",
        "properties": {
            "fooBar": { "type": "string" },
            "foo_bar": { "type": "string" }
        },
        "additionalProperties": false
    });
    let mut arguments = json!({ "foo-bar": "ambiguous", "semanticExtra": true });

    assert!(normalize_tool_argument_keys(&schema, &mut arguments).is_empty());
    assert_eq!(
        arguments,
        json!({ "foo-bar": "ambiguous", "semanticExtra": true })
    );
    assert!(tool_input_schema_error(&schema, &arguments, "arguments").is_some());

    let permissive_schema = json!({
        "type": "object",
        "properties": { "displayName": { "type": "string" } },
        "additionalProperties": true
    });
    let mut freeform_arguments = json!({ "display_name": "literal data key" });
    assert!(normalize_tool_argument_keys(&permissive_schema, &mut freeform_arguments).is_empty());
    assert_eq!(
        freeform_arguments,
        json!({ "display_name": "literal data key" })
    );
}

#[test]
fn responses_tool_representation_is_capability_driven_with_function_fallback() {
    let portable = ProviderToolCandidate {
        name: "apply_patch".to_string(),
        description: "Apply a patch.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "patch": { "type": "string" } },
            "required": ["patch"]
        }),
        ..Default::default()
    };

    let fallback = responses_tools(
        std::slice::from_ref(&portable),
        ProviderToolProtocolCapabilities::default(),
    );
    assert_eq!(fallback[0]["type"], "function");

    let freeform = responses_tools(
        std::slice::from_ref(&portable),
        ProviderToolProtocolCapabilities {
            freeform_tools: ProviderFeatureSupport::Supported,
            ..ProviderToolProtocolCapabilities::default()
        },
    );
    assert_eq!(freeform[0]["type"], "custom");

    let native_candidate = ProviderToolCandidate {
        input_schema: json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string" },
                "operation": { "type": "object" }
            }
        }),
        ..portable
    };
    let hosted = responses_tools(
        &[native_candidate],
        ProviderToolProtocolCapabilities {
            hosted_apply_patch: ProviderFeatureSupport::Supported,
            freeform_tools: ProviderFeatureSupport::Supported,
            ..ProviderToolProtocolCapabilities::default()
        },
    );
    assert_eq!(hosted, vec![json!({ "type": "apply_patch" })]);
}

#[test]
fn responses_native_patch_call_and_output_preserve_wire_protocol() {
    let item = json!({
        "type": "apply_patch_call",
        "id": "apc_1",
        "call_id": "call_patch",
        "status": "completed",
        "operation": {
            "type": "update_file",
            "path": "src/lib.rs",
            "diff": "@@\n-old\n+new"
        }
    });
    let calls = extract_provider_tool_calls(&json!({ "output": [item.clone()] })).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "apply_patch");
    assert_eq!(calls[0].arguments["operation"]["type"], "update_file");
    assert_eq!(calls[0].arguments["operation"]["path"], "src/lib.rs");

    let mut request = model_request();
    request.previous_response_items = vec![item];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "call_patch".to_string(),
        name: "apply_patch".to_string(),
        output: "Done!".to_string(),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    }];
    let input = responses_input(&request);
    let output = input
        .iter()
        .find(|item| item.get("type") == Some(&json!("apply_patch_call_output")))
        .expect("native patch output");
    assert_eq!(output["call_id"], "call_patch");
    assert_eq!(output["status"], "completed");
}

#[test]
fn responses_phase_keeps_commentary_out_of_final_text_and_replay_items() {
    let commentary = json!({
        "type": "message",
        "id": "msg_commentary",
        "role": "assistant",
        "phase": "commentary",
        "content": [{"type": "output_text", "text": "I will inspect it."}]
    });
    let final_answer = json!({
        "type": "message",
        "id": "msg_final",
        "role": "assistant",
        "phase": "final_answer",
        "content": [{"type": "output_text", "text": "Implemented."}]
    });
    let completed = json!({
        "id": "resp_phase",
        "status": "completed",
        "output": [commentary.clone(), final_answer.clone()]
    });
    assert_eq!(extract_response_text(&completed), "Implemented.");

    let mut accumulator = ResponsesStreamAccumulator::default();
    let mut deltas = Vec::new();
    for event in [
        json!({"type":"response.output_item.added","output_index":0,"item":commentary}),
        json!({"type":"response.output_text.delta","output_index":0,"delta":"I will inspect it."}),
        json!({"type":"response.output_item.added","output_index":1,"item":final_answer}),
        json!({"type":"response.output_text.delta","output_index":1,"delta":"Implemented."}),
        json!({"type":"response.completed","response":completed}),
    ] {
        accumulator
            .apply(&event, &mut |delta| {
                deltas.push(delta);
                Ok(())
            })
            .unwrap();
    }
    let response = accumulator.finish().unwrap();
    assert_eq!(response.text, "Implemented.");
    assert_eq!(response.provider_items[0]["phase"], "commentary");
    assert_eq!(response.provider_items[1]["phase"], "final_answer");
    assert_eq!(
        deltas,
        vec![ModelStreamDelta::Text {
            text: "Implemented.".to_string()
        }]
    );
}

async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0, "client closed before sending a complete request");
        bytes.extend_from_slice(&buffer[..read]);
        let Some(headers_end) = find_bytes(&bytes, b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..headers_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if bytes.len() >= headers_end + 4 + content_length {
            return String::from_utf8(bytes).unwrap();
        }
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[test]
fn built_in_provider_driver_registry_is_complete_and_host_trusted() {
    let registry = ProviderDriverRegistry::built_in();
    let descriptors = registry.descriptors();
    assert_eq!(descriptors.len(), 5);
    for adapter in [
        ProviderAdapterKind::Mock,
        ProviderAdapterKind::OpenAiChat,
        ProviderAdapterKind::OpenAiResponses,
        ProviderAdapterKind::AnthropicMessages,
        ProviderAdapterKind::CodexAppServer,
    ] {
        let descriptor = registry.descriptor(adapter).expect("registered driver");
        assert_eq!(descriptor.id, adapter.as_str());
        assert_eq!(descriptor.adapter, adapter);
        assert_eq!(descriptor.trust, ProviderDriverTrust::BuiltIn);
    }
}

#[test]
fn provider_driver_registry_preserves_unconfigured_fallback_semantics() {
    let mut settings = ProviderSettings::default();
    settings.kind = ProviderKind::Mock;
    assert!(ProviderDriverRegistry::built_in()
        .create(&settings)
        .is_some());

    let mut unconfigured = settings;
    unconfigured.kind = ProviderKind::OpenAiCompatible;
    unconfigured.api_key_configured = false;
    // Do not let a developer/CI machine's real provider environment make
    // this fallback test configured by accident.
    unconfigured.api_key_source = "OPENTOPIA_TEST_MISSING_PROVIDER_KEY".to_string();
    assert!(configured_provider_from_settings(&unconfigured).is_none());
    let _fallback = provider_from_settings(&unconfigured);
}

#[test]
fn release_gate_provider_capability_tiers_lower_deferred_tools_safely() {
    let mut candidate = ProviderToolCandidate::direct(
        "github__search_issues",
        "Search repository issues.",
        json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"],
            "additionalProperties": false
        }),
    );
    candidate.disclosure = ProviderToolDisclosure::DeferredNamespace;
    candidate.namespace = Some(ProviderToolNamespace {
        name: "github".to_string(),
        description: "GitHub repository tools.".to_string(),
    });

    let native_capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        deferred_tool_loading: ProviderFeatureSupport::Supported,
        namespace_tools: ProviderFeatureSupport::Supported,
        hosted_tool_search: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };
    let native = responses_tools(std::slice::from_ref(&candidate), native_capabilities);
    let namespace = native
        .iter()
        .find(|tool| tool["type"] == "namespace")
        .expect("Responses namespace");
    assert_eq!(namespace["name"], "github");
    assert_eq!(namespace["tools"][0]["name"], "github__search_issues");
    assert_eq!(namespace["tools"][0]["defer_loading"], true);
    assert!(native.iter().any(|tool| tool["type"] == "tool_search"));

    let individual_capabilities = ProviderToolProtocolCapabilities {
        namespace_tools: ProviderFeatureSupport::Unsupported,
        ..native_capabilities
    };
    let mut individual = candidate.clone();
    individual.disclosure = ProviderToolDisclosure::DeferredIndividual;
    individual.namespace = None;
    let individual_tools = responses_tools(&[individual], individual_capabilities);
    assert_eq!(individual_tools[0]["type"], "function");
    assert_eq!(individual_tools[0]["defer_loading"], true);
    assert_eq!(individual_tools[1]["type"], "tool_search");

    // Chat Completions and Anthropic do not receive Responses-only fields.
    let chat = openai_tools(
        std::slice::from_ref(&candidate),
        ProviderToolProtocolCapabilities::default(),
    );
    assert_eq!(chat[0]["type"], "function");
    assert!(chat[0].get("defer_loading").is_none());
    let anthropic = anthropic_tools(std::slice::from_ref(&candidate));
    assert_eq!(anthropic[0]["name"], "github__search_issues");
    assert!(anthropic[0].get("defer_loading").is_none());

    let portable_responses = responses_tools(
        std::slice::from_ref(&candidate),
        ProviderToolProtocolCapabilities::default(),
    );
    assert_eq!(portable_responses.len(), 1);
    assert_eq!(portable_responses[0]["type"], "function");
    assert!(portable_responses[0].get("defer_loading").is_none());
}

#[test]
fn release_gate_provider_payloads_keep_initial_user_message_last() {
    let mut request = model_request();
    request.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "earlier question".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "earlier answer".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
    ];
    request.input.current_user.message = "current user request".to_string();

    let chat = OpenAiCompatibleProvider::new(
        "https://compatible.example/v1",
        "test-key",
        "compatible-model",
    )
    .prepare(Uuid::nil(), request.clone())
    .expect("chat payload");
    let chat_messages = chat.body["messages"].as_array().expect("chat messages");
    assert_eq!(chat_messages.last().unwrap()["role"], "user");
    assert!(chat_messages.last().unwrap()["content"]
        .as_str()
        .unwrap()
        .contains("current user request"));

    let responses =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.4")
            .prepare(Uuid::nil(), request.clone())
            .expect("Responses payload");
    let responses_input = responses.body["input"].as_array().expect("Responses input");
    assert_eq!(responses_input.last().unwrap()["role"], "user");

    let anthropic =
        AnthropicMessagesProvider::new("https://api.anthropic.com", "test-key", "claude-test")
            .prepare(Uuid::nil(), request)
            .expect("Anthropic payload");
    let anthropic_messages = anthropic.body["messages"]
        .as_array()
        .expect("Anthropic messages");
    assert_eq!(anthropic_messages.last().unwrap()["role"], "user");
}

#[test]
fn release_gate_cache_breakpoints_preserve_stable_prefix_and_user_tail() {
    let mut provider =
        OpenAiResponsesProvider::new("https://api.openai.com/v1", "test-key", "gpt-5.6");
    provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
    let mut request = layered_model_request();
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::RepositoryInstructions,
        ContextRole::Developer,
        "AGENTS.md",
        "stable repository instructions",
        ContextCacheScope::Stable,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    request.input.conversation = vec![
        ModelConversationMessage {
            role: ModelConversationRole::User,
            content: "inherited request".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        ModelConversationMessage {
            role: ModelConversationRole::Assistant,
            content: "inherited answer".to_string(),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
    ];
    request.instructions.items.push(ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        "opentopia:execution_branch",
        "branch instructions",
        ContextCacheScope::Thread,
        crate::model_context::ContextSensitivity::Workspace,
    ));
    request.input.current_user.message = "new user request".to_string();

    let prepared = provider
        .prepare(Uuid::nil(), request)
        .expect("cache payload");
    let input = prepared.body["input"].as_array().expect("Responses input");
    let current_user = input
        .iter()
        .find(|item| item["content"].to_string().contains("new user request"))
        .expect("current user message");
    assert_eq!(current_user["role"], "user");
    assert_eq!(
        current_user["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    assert!(input.iter().any(|item| {
        item["content"].as_array().is_some_and(|parts| {
            parts.iter().any(|part| {
                part["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("branch instructions"))
                    && part["prompt_cache_breakpoint"]["mode"] == "explicit"
            })
        })
    }));
    let inherited_user = input
        .iter()
        .find(|item| item["content"].to_string().contains("inherited request"))
        .expect("inherited user message");
    assert_eq!(
        inherited_user["content"][0]["prompt_cache_breakpoint"]["mode"],
        "explicit"
    );
    let inherited_assistant = input
        .iter()
        .find(|item| item["role"] == "assistant")
        .expect("inherited assistant message");
    assert!(inherited_assistant["content"]
        .to_string()
        .contains("inherited answer"));
    assert!(!inherited_assistant["content"]
        .to_string()
        .contains("prompt_cache_breakpoint"));
}

#[test]
fn release_gate_tool_search_continuation_appends_after_current_user() {
    let mut request = model_request();
    request.previous_response_items = vec![
        json!({ "type": "tool_search_call", "id": "ts_1", "arguments": {"query": "issues"} }),
        json!({ "type": "tool_search_output", "id": "tso_1", "tools": [{"type": "function", "name": "github__search_issues"}] }),
        json!({ "type": "function_call", "call_id": "call_1", "name": "github__search_issues", "arguments": "{\"query\":\"bug\"}" }),
    ];
    request.input.tool_results = vec![ProviderToolResult {
        call_id: "call_1".to_string(),
        name: "github__search_issues".to_string(),
        output: "[]".to_string(),
        content: Vec::new(),
        is_error: false,
        metadata: json!({}),
    }];

    let input = responses_input(&request);
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[1]["type"], "tool_search_call");
    assert_eq!(input[2]["type"], "tool_search_output");
    assert_eq!(input[3]["type"], "function_call");
    assert_eq!(input[4]["type"], "function_call_output");
}
