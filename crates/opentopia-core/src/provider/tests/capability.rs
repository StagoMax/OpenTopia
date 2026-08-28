#[test]
fn applying_probe_report_persists_profile_used_by_first_request() {
    let base_url = "https://saved-capability.example/v1";
    let model = "saved-capability-model";
    let mut settings = ProviderSettings {
        base_url: base_url.to_string(),
        model: model.to_string(),
        ..ProviderSettings::default()
    };
    settings.apply_openai_compatibility_report(OpenAiCompatibilityReport {
        base_url: base_url.to_string(),
        model: model.to_string(),
        selected_protocol: OpenAiProtocol::ChatCompletions,
        chat_completions: ProviderFeatureSupport::Supported,
        chat_function_tools: ProviderFeatureSupport::Supported,
        chat_strict_function_tools: ProviderFeatureSupport::Unsupported,
        chat_streaming_tools: ProviderFeatureSupport::Unsupported,
        chat_parallel_tool_calls: ProviderFeatureSupport::Unsupported,
        chat_json_schema_output: ProviderFeatureSupport::Unsupported,
        chat_message_protocol: ProviderMessageProtocolCapabilities::default(),
        chat_reasoning_protocol: Some(ProviderReasoningProtocol::ChatReasoningEffort),
        responses: ProviderFeatureSupport::Unsupported,
        responses_native_tools: ProviderFeatureSupport::Unsupported,
        responses_function_tools: ProviderFeatureSupport::Unsupported,
        responses_strict_function_tools: ProviderFeatureSupport::Unsupported,
        responses_streaming_tools: ProviderFeatureSupport::Unsupported,
        responses_parallel_tool_calls: ProviderFeatureSupport::Unsupported,
        responses_json_schema_output: ProviderFeatureSupport::Unsupported,
        responses_custom_tools: ProviderFeatureSupport::Unsupported,
        responses_apply_patch: ProviderFeatureSupport::Unsupported,
        responses_reasoning_protocol: Some(ProviderReasoningProtocol::ResponsesReasoning),
        developer_messages: ProviderFeatureSupport::Unsupported,
        message_compatibility: true,
        checked_at: chrono::Utc::now(),
        notes: Vec::new(),
    });
    assert_eq!(
        settings.model_settings[model].preferred_adapter,
        Some(ProviderAdapterKind::OpenAiChat)
    );
    let mut responses_report = settings.openai_compatibility.clone().unwrap();
    responses_report.selected_protocol = OpenAiProtocol::Responses;
    responses_report.responses = ProviderFeatureSupport::Supported;
    responses_report.responses_function_tools = ProviderFeatureSupport::Supported;
    responses_report.responses_streaming_tools = ProviderFeatureSupport::Supported;
    settings.apply_openai_compatibility_report(responses_report);
    assert_eq!(
        settings.model_settings[model].preferred_adapter,
        Some(ProviderAdapterKind::OpenAiResponses),
        "a probe-managed recommendation should follow a newly proven route"
    );
    settings
        .model_settings
        .entry(model.to_string())
        .or_default()
        .preferred_adapter = None;
    assert_eq!(
        settings.resolved_adapter_for_model(model),
        ProviderAdapterKind::OpenAiResponses,
        "routing without an explicit preference should select the richer proven profile"
    );
    let mut chat_report = settings.openai_compatibility.clone().unwrap();
    chat_report.selected_protocol = OpenAiProtocol::ChatCompletions;
    chat_report.responses = ProviderFeatureSupport::Unsupported;
    chat_report.responses_function_tools = ProviderFeatureSupport::Unsupported;
    settings.apply_openai_compatibility_report(chat_report);
    assert_eq!(
        settings.model_settings[model].preferred_adapter,
        Some(ProviderAdapterKind::OpenAiChat)
    );
    settings
        .model_settings
        .entry(model.to_string())
        .or_default()
        .preferred_adapter = Some(ProviderAdapterKind::OpenAiResponses);
    assert_eq!(
        settings.resolved_adapter_for_model(model),
        ProviderAdapterKind::OpenAiChat,
        "a preference whose adapter failed negotiation must not override the proven route"
    );

    let provider = OpenAiCompatibleProvider::new(base_url, "test-key", model)
        .with_generation_settings(&settings);
    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .expect("prepare first request from persisted profile");
    assert!(!prepared.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "developer"));

    let mut report = settings.openai_compatibility.clone().unwrap();
    report.developer_messages = ProviderFeatureSupport::Supported;
    report.message_compatibility = false;
    settings.apply_openai_compatibility_report(report);
    let native = OpenAiCompatibleProvider::new(base_url, "test-key", model)
        .with_generation_settings(&settings)
        .prepare(Uuid::nil(), layered_model_request())
        .expect("prepare first native request from persisted profile");
    assert!(native.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "developer"));
}

#[tokio::test]
async fn compatibility_probe_negotiates_thinking_for_an_unknown_model() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        // Responses is unavailable, so its strict function-tool probe falls
        // back once to the portable shape before the developer-role probe.
        loop {
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request(&mut socket).await;
            let is_responses = request.starts_with("POST /v1/responses ");
            let has_thinking = request.contains(r#""thinking":{"type":"enabled"}"#)
                && request.contains(r#""reasoning_effort":"high""#);
            let has_developer = request.contains(r#""role":"developer""#);
            let forces_tool = request.contains(r#""name":"compatibility_probe""#)
                && request.contains("opentopia-tool-probe-v1");
            let streams = request.contains(r#""stream":true"#);
            let (status, body) = if is_responses {
                ("404 Not Found", r#"{"error":"not found"}"#)
            } else if !has_thinking {
                (
                    "400 Bad Request",
                    r#"{"error":"thinking envelope required"}"#,
                )
            } else if has_developer {
                (
                    "400 Bad Request",
                    r#"{"error":"unsupported developer role"}"#,
                )
            } else if forces_tool && streams {
                (
                        "200 OK",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"probe_call\",\"function\":{\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
                    )
            } else if forces_tool {
                (
                    "200 OK",
                    r#"{"choices":[{"message":{"reasoning_content":"probe reasoning","tool_calls":[{"id":"probe_call","type":"function","function":{"name":"compatibility_probe","arguments":"{\"token\":\"opentopia-tool-probe-v1\"}"}}]},"finish_reason":"tool_calls"}]}"#,
                )
            } else {
                ("200 OK", "{}")
            };
            socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let base_url = format!("http://{address}/v1");
    let provider = OpenAiCompatibleProvider::new(&base_url, "test-key", "future-reasoner-1");
    let health = provider
        .probe_compatibility(ProviderAdapterKind::OpenAiChat)
        .await
        .unwrap();
    server.await.unwrap();

    assert!(health.reachable);
    assert!(health.model_available);
    let report = health.openai_compatibility.unwrap();
    assert_eq!(report.selected_protocol, OpenAiProtocol::ChatCompletions);
    assert_eq!(
        report.chat_reasoning_protocol,
        Some(ProviderReasoningProtocol::ChatThinkingReasoningEffort)
    );
    assert_eq!(report.chat_completions, ProviderFeatureSupport::Supported);
    assert_eq!(
        report.chat_function_tools,
        ProviderFeatureSupport::Supported
    );
    assert_eq!(
        report.chat_streaming_tools,
        ProviderFeatureSupport::Supported
    );
    assert_eq!(report.responses, ProviderFeatureSupport::Unsupported);
    assert_eq!(
        report.responses_native_tools,
        ProviderFeatureSupport::Unknown
    );
    assert_eq!(
        report.developer_messages,
        ProviderFeatureSupport::Unsupported
    );
    assert!(
        report
            .chat_message_protocol
            .requires_reasoning_content_for_tool_calls
    );
    assert!(report.message_compatibility);

    let mut settings = ProviderSettings {
        base_url: base_url.clone(),
        model: "future-reasoner-1".to_string(),
        ..ProviderSettings::default()
    };
    settings.apply_openai_compatibility_report(report);
    let profile = settings.active_adapter_profile().unwrap();
    assert_eq!(
        profile.instruction_encoding,
        ProviderInstructionEncoding::PortableChatEnvelope
    );
    assert!(
        profile
            .message_protocol
            .requires_reasoning_content_for_tool_calls
    );
    let prepared = OpenAiCompatibleProvider::new(&base_url, "test-key", "future-reasoner-1")
        .with_generation_settings(&settings)
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();
    assert!(!prepared.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "developer"));
}

#[tokio::test]
async fn compatibility_probe_reports_the_function_tool_failure_reason() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request(&mut socket).await;
            let is_tool_probe = request.contains(r#""name":"compatibility_probe""#)
                && request.contains("opentopia-tool-probe-v1");
            let (status, body) = if is_tool_probe {
                (
                    "400 Bad Request",
                    r#"{"error":"function tools are disabled for this API key"}"#,
                )
            } else {
                ("200 OK", "{}")
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "relay-model");
    let health = provider
        .probe_compatibility(ProviderAdapterKind::OpenAiChat)
        .await
        .unwrap();
    server.await.unwrap();

    assert!(health.reachable);
    assert!(!health.model_available);
    let error = health.error.expect("failed probes include a diagnostic");
    assert!(error.contains("model 'relay-model'"));
    assert!(error.contains("Chat function tools:"));
    assert!(error.contains("Responses function tools:"));
    assert!(error.contains("HTTP 400"));
    assert!(
        error.contains("function tools are disabled for this API key"),
        "the relay's response should reach the settings error"
    );
}

#[tokio::test]
async fn compatibility_probe_prefers_portable_chat_for_third_party_relays() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request(&mut socket).await;
            let is_responses = request.starts_with("POST /v1/responses ");
            let has_developer = request.contains(r#""role":"developer""#);
            let forces_tool = request.contains(r#""name":"compatibility_probe""#)
                && request.contains("opentopia-tool-probe-v1");
            let streams = request.contains(r#""stream":true"#);
            let (status, body) = if has_developer {
                (
                    "400 Bad Request",
                    r#"{"error":"unsupported developer role"}"#,
                )
            } else if forces_tool && is_responses && streams {
                (
                        "200 OK",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_probe\",\"status\":\"completed\",\"output\":[{\"type\":\"function_call\",\"call_id\":\"probe_call\",\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}]}}\n\ndata: [DONE]\n\n",
                    )
            } else if forces_tool && streams {
                (
                        "200 OK",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"probe_call\",\"function\":{\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
                    )
            } else if forces_tool && is_responses {
                (
                    "200 OK",
                    r#"{"output":[{"type":"function_call","call_id":"probe_call","name":"compatibility_probe","arguments":"{\"token\":\"opentopia-tool-probe-v1\"}"}]}"#,
                )
            } else if forces_tool {
                (
                    "200 OK",
                    r#"{"choices":[{"message":{"tool_calls":[{"id":"probe_call","type":"function","function":{"name":"compatibility_probe","arguments":"{\"token\":\"opentopia-tool-probe-v1\"}"}}]},"finish_reason":"tool_calls"}]}"#,
                )
            } else {
                ("200 OK", "{}")
            };
            socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let provider = OpenAiCompatibleProvider::new(
        format!("http://{address}/v1"),
        "test-key",
        "responses-probe-model",
    );
    let health = provider
        .probe_compatibility(ProviderAdapterKind::OpenAiChat)
        .await
        .unwrap();
    server.await.unwrap();
    let report = health.openai_compatibility.unwrap();

    assert_eq!(report.selected_protocol, OpenAiProtocol::ChatCompletions);
    assert_eq!(report.responses, ProviderFeatureSupport::Supported);
    assert_eq!(
        report.responses_native_tools,
        ProviderFeatureSupport::Unknown
    );
    assert_eq!(
        report.chat_streaming_tools,
        ProviderFeatureSupport::Supported
    );
    assert_eq!(
        report.responses_streaming_tools,
        ProviderFeatureSupport::Supported
    );
    assert_eq!(
        report.developer_messages,
        ProviderFeatureSupport::Unsupported
    );
    assert!(report.message_compatibility);
}

#[tokio::test]
async fn compatibility_probe_keeps_hosted_tools_unknown_for_third_party_relays() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok(Ok((mut socket, _))) =
                tokio::time::timeout(Duration::from_secs(2), listener.accept()).await
            else {
                break;
            };
            let request = read_http_request(&mut socket).await;
            let is_responses = request.starts_with("POST /v1/responses ");
            let has_native_web_search = request.contains(r#""type":"web_search""#);
            let has_developer = request.contains(r#""role":"developer""#);
            let forces_tool = request.contains(r#""name":"compatibility_probe""#)
                && request.contains("opentopia-tool-probe-v1");
            let streams = request.contains(r#""stream":true"#);
            let (status, body) = if is_responses && has_native_web_search {
                (
                    "400 Bad Request",
                    r#"{"error":"native Responses tools unsupported"}"#,
                )
            } else if has_developer {
                (
                    "400 Bad Request",
                    r#"{"error":"unsupported developer role"}"#,
                )
            } else if forces_tool && is_responses && streams {
                (
                        "200 OK",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_probe\",\"status\":\"completed\",\"output\":[{\"type\":\"function_call\",\"call_id\":\"probe_call\",\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}]}}\n\ndata: [DONE]\n\n",
                    )
            } else if forces_tool && streams {
                (
                        "200 OK",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"probe_call\",\"function\":{\"name\":\"compatibility_probe\",\"arguments\":\"{\\\"token\\\":\\\"opentopia-tool-probe-v1\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: [DONE]\n\n",
                    )
            } else if forces_tool && is_responses {
                (
                    "200 OK",
                    r#"{"output":[{"type":"function_call","call_id":"probe_call","name":"compatibility_probe","arguments":"{\"token\":\"opentopia-tool-probe-v1\"}"}]}"#,
                )
            } else if forces_tool {
                (
                    "200 OK",
                    r#"{"choices":[{"message":{"tool_calls":[{"id":"probe_call","type":"function","function":{"name":"compatibility_probe","arguments":"{\"token\":\"opentopia-tool-probe-v1\"}"}}]},"finish_reason":"tool_calls"}]}"#,
                )
            } else {
                ("200 OK", "{}")
            };
            socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            socket.shutdown().await.unwrap();
        }
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "relay-model");
    let health = provider
        .probe_compatibility(ProviderAdapterKind::OpenAiChat)
        .await
        .unwrap();
    server.await.unwrap();
    let report = health.openai_compatibility.unwrap();

    assert_eq!(report.selected_protocol, OpenAiProtocol::ChatCompletions);
    assert_eq!(
        report.responses_native_tools,
        ProviderFeatureSupport::Unknown
    );
    assert_eq!(
        report.chat_streaming_tools,
        ProviderFeatureSupport::Supported
    );
    assert_eq!(
        report.responses_streaming_tools,
        ProviderFeatureSupport::Supported
    );
    assert!(report.message_compatibility);
}

#[test]
fn responses_input_replays_typed_items_and_correlates_tool_outputs() {
    let mut request = model_request();
    request.previous_response_items = vec![
        json!({
            "type": "reasoning",
            "encrypted_content": "opaque",
            "summary": []
        }),
        json!({
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "read_file",
            "arguments": "{\"path\":\"Cargo.toml\"}"
        }),
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

    let input = responses_input(&request);

    assert_eq!(input[1]["type"], "reasoning");
    assert_eq!(input[2]["call_id"], "call_1");
    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_1");
    assert_eq!(
        input
            .iter()
            .filter(|item| item.get("type") == Some(&json!("function_call")))
            .count(),
        1
    );
}

#[tokio::test]
async fn responses_provider_prepares_redacted_body_and_collects_typed_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        request_tx.send(request).unwrap();
        socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_123\"}}\n\n",
                        "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"\"}}\n\n",
                        "data: {\"type\":\"response.function_call_arguments.delta\",\"response_id\":\"resp_123\",\"output_index\":0,\"delta\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}\n\n",
                        "data: {\"type\":\"response.output_item.done\",\"response_id\":\"resp_123\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}\n\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_1\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}],\"usage\":{\"input_tokens\":20,\"output_tokens\":5,\"total_tokens\":25,\"input_tokens_details\":{\"cached_tokens\":12,\"cache_write_tokens\":8}}}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
    });
    let mut provider =
        OpenAiResponsesProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.native_web_search = true;
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.input.current_user.content = vec![ModelInputContent::image("image/png", vec![1, 2, 3])];
    request.instructions.prompt_cache_key = Some("workspace-cache".to_string());
    request.tool_candidates = vec![ProviderToolCandidate {
        name: "read_file".to_string(),
        description: "Read a file".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        ..Default::default()
    }];
    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    assert_eq!(prepared.adapter, "openai_responses");
    assert!(prepared.observation_body.get("prompt_cache_key").is_none());
    assert!(prepared
        .observation_body
        .to_string()
        .contains("data URL omitted"));
    assert!(!prepared.observation_body.to_string().contains("AQID"));
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
            transport.push(event);
            Ok(())
        })
        .await
        .unwrap();
    server.await.unwrap();
    let raw_request = request_rx.await.unwrap();
    let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
    let payload: Value = serde_json::from_str(body).unwrap();

    assert_eq!(payload["stream"], true);
    assert_eq!(payload["store"], false);
    assert_eq!(payload["tools"][0], json!({ "type": "web_search" }));
    assert_eq!(payload["tools"][1]["name"], "read_file");
    assert!(payload["tools"][1].get("function").is_none());
    assert_eq!(payload["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(response.response_id.as_deref(), Some("resp_123"));
    assert_eq!(response.tool_calls[0].id, "call_1");
    assert_eq!(
        response.tool_calls[0].arguments,
        json!({ "path": "Cargo.toml" })
    );
    assert_eq!(response.provider_items.len(), 1);
    let usage = response.usage.unwrap();
    assert_eq!(usage.total_tokens, 25);
    assert_eq!(usage.cached_input_tokens, Some(12));
    assert_eq!(usage.cache_write_tokens, Some(8));
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::ResponseHeaders { status: 200, .. }
    )));
    assert!(transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::OutputStarted { .. })));
    assert!(transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::StreamProgress { .. })));
    assert!(transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::ResponseCommitStarted { .. })));
    assert!(matches!(
        transport.last(),
        Some(ProviderTransportEvent::Response {
            status: Some(200),
            response_id: Some(response_id),
            ..
        }) if response_id == "resp_123"
    ));
}

#[tokio::test]
async fn responses_provider_replays_local_context_when_state_cursor_is_missing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut first_socket, _) = listener.accept().await.unwrap();
        let first_request = read_http_request(&mut first_socket).await;
        let rejected = r#"{"error":{"message":"previous_response_id resp_missing was not found","param":"previous_response_id"}}"#;
        first_socket
                .write_all(
                    format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        first_socket.shutdown().await.unwrap();

        let (mut second_socket, _) = listener.accept().await.unwrap();
        let second_request = read_http_request(&mut second_socket).await;
        second_socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_replayed\",\"output_text\":\"replayed locally\",\"output\":[]}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        second_socket.shutdown().await.unwrap();
        request_tx.send((first_request, second_request)).unwrap();
    });
    let mut provider =
        OpenAiResponsesProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.store_responses = true;
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut request = layered_model_request();
    request.input.conversation = vec![ModelConversationMessage {
        role: ModelConversationRole::User,
        content: "canonical local history".to_string(),
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    }];
    request.previous_response_id = Some("resp_missing".to_string());
    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let mut transport = Vec::new();

    let response = provider
        .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
            transport.push(event);
            Ok(())
        })
        .await
        .unwrap();
    server.await.unwrap();
    let (first_request, second_request) = request_rx.await.unwrap();
    let first: Value =
        serde_json::from_str(first_request.split_once("\r\n\r\n").unwrap().1).unwrap();
    let second: Value =
        serde_json::from_str(second_request.split_once("\r\n\r\n").unwrap().1).unwrap();

    assert_eq!(response.text, "replayed locally");
    assert_eq!(response.response_id.as_deref(), Some("resp_replayed"));
    assert_eq!(first["previous_response_id"], "resp_missing");
    assert_eq!(first["input"].as_array().unwrap().len(), 2);
    assert!(second.get("previous_response_id").is_none());
    assert!(second["input"]
        .to_string()
        .contains("canonical local history"));
    assert!(transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { attempt: 2, .. })));
}

#[test]
fn function_apply_patch_uses_one_portable_shape_across_openai_transports() {
    let candidate = ProviderToolCandidate {
        name: "apply_patch".to_string(),
        description: "Legacy multi-shape patch tool.".to_string(),
        input_schema: json!({
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
        }),
        ..Default::default()
    };

    let chat_tools = openai_tools(
        std::slice::from_ref(&candidate),
        ProviderToolProtocolCapabilities::default(),
    );
    let chat_function = &chat_tools[0]["function"];
    assert_eq!(chat_function["parameters"]["required"], json!(["patch"]));
    assert_eq!(
        chat_function["parameters"]["additionalProperties"],
        json!(false)
    );
    assert!(chat_function["parameters"].get("anyOf").is_none());
    assert!(chat_function["parameters"]["properties"]
        .get("operation")
        .is_none());
    assert!(chat_function["description"]
        .as_str()
        .unwrap()
        .contains("*** Begin Patch"));
    assert!(chat_function["description"]
        .as_str()
        .unwrap()
        .contains("Do not send `path`, `diff`, or `operation`"));

    let responses_tools = responses_tools(
        std::slice::from_ref(&candidate),
        ProviderToolProtocolCapabilities::default(),
    );
    assert_eq!(responses_tools[0]["type"], "function");
    assert_eq!(
        responses_tools[0]["parameters"]["required"],
        json!(["patch"])
    );
    assert!(responses_tools[0]["parameters"].get("anyOf").is_none());
}

#[test]
fn strict_function_schema_is_lowered_per_provider_and_per_tool() {
    let strict_capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };
    let strict_ready = ProviderToolCandidate {
        name: "search".to_string(),
        description: "Search records.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1}
            },
            "required": ["query"]
        }),
        ..Default::default()
    };

    let chat = openai_tools(std::slice::from_ref(&strict_ready), strict_capabilities);
    let chat_function = &chat[0]["function"];
    assert_eq!(chat_function["strict"], true);
    assert_eq!(
        chat_function["parameters"]["required"],
        json!(["limit", "query"])
    );
    assert_eq!(
        chat_function["parameters"]["properties"]["limit"]["type"],
        json!(["integer", "null"])
    );
    assert_eq!(chat_function["parameters"]["additionalProperties"], false);

    let responses = responses_tools(std::slice::from_ref(&strict_ready), strict_capabilities);
    assert_eq!(responses[0]["strict"], true);
    assert_eq!(
        responses[0]["parameters"]["properties"]["limit"]["type"],
        json!(["integer", "null"])
    );

    let portable_only = ProviderToolCandidate {
        name: "choose".to_string(),
        description: "Choose exactly one shape.".to_string(),
        input_schema: json!({
            "type": "object",
            "oneOf": [
                {"type": "object", "properties": {"a": {"type": "string"}}},
                {"type": "object", "properties": {"b": {"type": "string"}}}
            ]
        }),
        ..Default::default()
    };
    let fallback = responses_tools(&[portable_only.clone()], strict_capabilities);
    assert_eq!(fallback[0]["strict"], false);
    assert!(fallback[0]["parameters"].get("oneOf").is_some());
    let chat_fallback = openai_tools(&[portable_only], strict_capabilities);
    assert_eq!(chat_fallback[0]["function"]["strict"], false);
}

#[test]
fn strict_function_schema_preserves_discriminated_unions() {
    let schema = json!({
        "type": "object",
        "properties": {
            "window": {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["characters"] },
                            "offset": { "type": "integer" }
                        },
                        "required": ["mode"]
                    },
                    {
                        "type": "object",
                        "properties": {
                            "mode": { "type": "string", "enum": ["lines"] },
                            "startLine": { "type": "integer" }
                        },
                        "required": ["mode", "startLine"]
                    }
                ]
            }
        }
    });

    let lowered = openai_strict_function_schema(&schema).expect("lower tagged union");
    let union = &lowered["properties"]["window"]["anyOf"];
    assert!(union.is_array());
    assert!(lowered["properties"]["window"].get("oneOf").is_none());
    assert_eq!(
        lowered["properties"]["window"]["anyOf"][0]["additionalProperties"],
        false
    );
}

#[test]
fn strict_function_schema_preserves_root_discriminated_unions() {
    let schema = json!({
        "type": "object",
        "oneOf": [
            {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["read"] },
                    "path": { "type": "string" }
                },
                "required": ["action", "path"]
            },
            {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["list"] }
                },
                "required": ["action"]
            }
        ]
    });

    let lowered = openai_strict_function_schema(&schema).expect("lower root tagged union");
    assert!(lowered["anyOf"].is_array());
    assert!(lowered.get("properties").is_none());
    assert!(lowered.get("additionalProperties").is_none());
    assert_eq!(lowered["anyOf"][0]["additionalProperties"], false);
    assert_eq!(lowered["anyOf"][1]["additionalProperties"], false);
}

#[test]
fn root_discriminated_tools_compile_to_portable_objects_across_openai_transports() {
    let capabilities = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        strict_function_tools: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };
    let tools: [&dyn Tool; 4] = [
        &BackgroundOutputTool,
        &DocumentTool,
        &PdfTool,
        &SpreadsheetTool,
    ];
    let candidates = tools
        .iter()
        .map(|tool| ProviderToolCandidate::direct(tool.name(), tool.description(), tool.schema()))
        .collect::<Vec<_>>();

    let chat = openai_tools(&candidates, capabilities);
    let responses = responses_tools(&candidates, capabilities);
    for ((tool, chat_tool), responses_tool) in tools.iter().zip(&chat).zip(&responses) {
        let chat_function = &chat_tool["function"];
        for function in [chat_function, responses_tool] {
            let parameters = &function["parameters"];
            assert_eq!(parameters["type"], "object", "{}", tool.name());
            assert!(parameters["properties"].is_object(), "{}", tool.name());
            assert!(parameters.get("oneOf").is_none(), "{}", tool.name());
            assert!(parameters.get("anyOf").is_none(), "{}", tool.name());
            assert!(
                parameters["properties"]["action"]["enum"]
                    .as_array()
                    .is_some_and(|actions| actions.len() > 1),
                "{}",
                tool.name()
            );
            assert!(
                parameters["required"]
                    .as_array()
                    .is_some_and(|required| required.contains(&json!("action"))),
                "{}",
                tool.name()
            );
            assert_eq!(function["strict"], false, "{}", tool.name());
        }
    }
}
