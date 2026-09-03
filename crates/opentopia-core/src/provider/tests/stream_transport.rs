#[test]
fn base64_encoding_handles_all_padding_cases() {
    assert_eq!(encode_base64(b""), "");
    assert_eq!(encode_base64(b"f"), "Zg==");
    assert_eq!(encode_base64(b"fo"), "Zm8=");
    assert_eq!(encode_base64(b"foo"), "Zm9v");
}

#[test]
fn decodes_sse_across_arbitrary_chunks() {
    let mut decoder = SseDecoder::default();

    assert!(decoder
        .push(b"data: {\"choices\":[{\"del")
        .unwrap()
        .is_empty());
    let events = decoder
        .push(b"ta\":{\"content\":\"hello\"}}]}\r\n\r\ndata: [DO")
        .unwrap();
    assert_eq!(
        events,
        vec![r#"{"choices":[{"delta":{"content":"hello"}}]}"#]
    );
    assert_eq!(decoder.push(b"NE]\n\n").unwrap(), vec!["[DONE]"]);
    assert!(decoder.finish().unwrap().is_empty());
}

#[test]
fn accumulates_streamed_text_tool_arguments_and_usage() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut deltas = Vec::new();
    let mut collect = |delta| {
        deltas.push(delta);
        Ok(())
    };
    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "content": "Inspecting ",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_read",
                        "function": {"name": "read_file", "arguments": "{\"path\":"}
                    }]
                }}]
            }),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "content": "now",
                    "tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "\"src/lib.rs\"}"}
                    }]
                }}]
            }),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 5,
                    "total_tokens": 25
                }
            }),
            &mut collect,
        )
        .unwrap();

    let response = accumulator.finish().unwrap();

    assert_eq!(response.text, "Inspecting now");
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "src/lib.rs" }),
        }]
    );
    assert_eq!(
        response.usage,
        Some(ModelUsage {
            input_tokens: 20,
            output_tokens: 5,
            total_tokens: 25,
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        })
    );
    assert_eq!(
        deltas,
        vec![
            ModelStreamDelta::Text {
                text: "Inspecting ".to_string()
            },
            ModelStreamDelta::ToolCall {
                index: 0,
                id: Some("call_read".to_string()),
                name: Some("read_file".to_string()),
                arguments_delta: "{\"path\":".to_string(),
            },
            ModelStreamDelta::Text {
                text: "now".to_string()
            },
            ModelStreamDelta::ToolCall {
                index: 0,
                id: None,
                name: None,
                arguments_delta: "\"src/lib.rs\"}".to_string(),
            },
            ModelStreamDelta::Usage {
                usage: ModelUsage {
                    input_tokens: 20,
                    output_tokens: 5,
                    total_tokens: 25,
                    cached_input_tokens: None,
                    cache_write_tokens: None,
                    reasoning_tokens: None,
                }
            }
        ]
    );
}

#[test]
fn normalizes_repeated_and_cumulative_chat_tool_call_chunks() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut deltas = Vec::new();
    let mut collect = |delta| {
        deltas.push(delta);
        Ok(())
    };

    for event in [
        json!({"choices": [{"delta": {"tool_calls": [{
            "id": "call_shell",
            "function": {"name": "shell", "arguments": "{\"command\":"}
        }]}}]}),
        json!({"choices": [{"delta": {"tool_calls": [{
            "id": "call_shell",
            "function": {"name": "shell", "arguments": "{\"command\":\"cargo test\"}"}
        }]}}]}),
    ] {
        accumulator.apply(&event, &mut collect).unwrap();
    }

    let response = accumulator.finish().unwrap();
    assert_eq!(response.tool_calls[0].id, "call_shell");
    assert_eq!(response.tool_calls[0].name, "shell");
    assert_eq!(
        response.tool_calls[0].arguments,
        json!({"command": "cargo test"})
    );
    let tool_deltas = deltas
        .into_iter()
        .filter_map(|delta| match delta {
            ModelStreamDelta::ToolCall {
                arguments_delta, ..
            } => Some(arguments_delta),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_deltas, vec!["{\"command\":", "\"cargo test\"}"]);
}

#[test]
fn keeps_parallel_chat_tool_calls_distinct_when_indices_are_missing() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut collect = |_| Ok(());

    accumulator
        .apply(
            &json!({"choices": [{"delta": {"tool_calls": [
                {"id": "call_a", "function": {"name": "lookup", "arguments": "{\"query\":\"a\"}"}},
                {"id": "call_b", "function": {"name": "lookup", "arguments": "{\"query\":\"b\"}"}}
            ]}}]}),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({"choices": [{"delta": {"tool_calls": [
                {"id": "call_b", "function": {"name": "lookup", "arguments": "{\"query\":\"b\"}"}}
            ]}}]}),
            &mut collect,
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(response.tool_calls[0].id, "call_a");
    assert_eq!(response.tool_calls[0].arguments, json!({"query": "a"}));
    assert_eq!(response.tool_calls[1].id, "call_b");
    assert_eq!(response.tool_calls[1].arguments, json!({"query": "b"}));
}

#[test]
fn accepts_object_tool_arguments_from_compatible_chat_streams() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut collect = |_| Ok(());

    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_read",
                        "function": {
                            "name": "read_file",
                            "arguments": {"path": "src/lib.rs"}
                        }
                    }]
                }}]
            }),
            &mut collect,
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_read".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "src/lib.rs" }),
        }]
    );
}

#[test]
fn uses_final_message_tool_argument_snapshot_when_deltas_omit_it() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut collect = |_| Ok(());

    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_shell",
                        "function": {"name": "shell"}
                    }]
                }}]
            }),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({
                "choices": [{
                    "message": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_shell",
                            "function": {
                                "name": "shell",
                                "arguments": {"command": "cargo test"}
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }),
            &mut collect,
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(response.finish_reason, ModelFinishReason::ToolCalls);
    assert_eq!(
        response.tool_calls,
        vec![ProviderToolCall {
            id: "call_shell".to_string(),
            name: "shell".to_string(),
            arguments: json!({ "command": "cargo test" }),
        }]
    );
}

#[test]
fn rejects_streamed_tool_calls_that_never_carry_arguments() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    accumulator
        .apply(
            &json!({
                "choices": [{
                    "delta": {
                        "tool_calls": [{
                            "index": 0,
                            "id": "call_missing",
                            "function": {"name": "shell"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            }),
            &mut |_| Ok(()),
        )
        .unwrap();

    let error = accumulator
        .finish()
        .expect_err("missing arguments must fail");
    assert!(error.to_string().contains("function.arguments was absent"));
}

#[test]
fn emits_provider_supplied_reasoning_deltas_without_synthesizing_text() {
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut deltas = Vec::new();
    let mut collect = |delta| {
        deltas.push(delta);
        Ok(())
    };

    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "reasoning_content": "检查工作区"
                }}]
            }),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {
                    "reasoning": {
                        "summary": [{"type": "summary_text", "text": "并制定计划"}]
                    }
                }}]
            }),
            &mut collect,
        )
        .unwrap();
    accumulator
        .apply(
            &json!({
                "choices": [{"delta": {"content": "开始执行"}}]
            }),
            &mut collect,
        )
        .unwrap();

    let response = accumulator.finish().unwrap();
    assert_eq!(response.text, "开始执行");
    assert_eq!(
        deltas,
        vec![
            ModelStreamDelta::Reasoning {
                text: "检查工作区".to_string(),
            },
            ModelStreamDelta::Reasoning {
                text: "并制定计划".to_string(),
            },
            ModelStreamDelta::Text {
                text: "开始执行".to_string(),
            },
        ]
    );
}

#[tokio::test]
async fn default_stream_emits_one_complete_text_delta() {
    let provider = MockProvider;
    let mut deltas = Vec::new();
    let response = provider
        .stream(model_request(), &mut |delta| {
            deltas.push(delta);
            Ok(())
        })
        .await
        .unwrap();

    assert_eq!(deltas.len(), 1);
    assert_eq!(
        deltas[0],
        ModelStreamDelta::Text {
            text: response.text
        }
    );
}

#[tokio::test]
async fn openai_provider_requests_and_collects_real_sse_stream() {
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
                        "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
                        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":2,\"total_tokens\":11}}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
    });
    let mut provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    provider.tool_protocol.parallel_tool_calls = ProviderFeatureSupport::Supported;
    provider.temperature = Some(0.7);
    provider.max_output_tokens = Some(2048);
    provider.reasoning_effort = Some("high".to_string());
    let mut request = model_request();
    request.input.conversation.push(ModelConversationMessage {
        role: ModelConversationRole::Assistant,
        content: "history".to_string(),
        content_parts: Vec::new(),
        tool_calls: Vec::new(),
        tool_results: Vec::new(),
    });
    request.tool_candidates.push(ProviderToolCandidate {
        name: "read_file".to_string(),
        description: "Read a workspace file".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        ..Default::default()
    });
    let mut deltas = Vec::new();
    let delivery_order = Arc::new(Mutex::new(Vec::new()));
    let delta_delivery_order = Arc::clone(&delivery_order);
    let transport_delivery_order = Arc::clone(&delivery_order);
    let prepared = provider.prepare(Uuid::new_v4(), request).unwrap();

    let response = provider
        .stream_prepared(
            prepared,
            &mut |delta| {
                if matches!(delta, ModelStreamDelta::Text { .. }) {
                    delta_delivery_order
                        .lock()
                        .unwrap()
                        .push("text_delta");
                }
                deltas.push(delta);
                Ok(())
            },
            &mut |event| {
                if matches!(event, ProviderTransportEvent::ResponseCommitStarted { .. }) {
                    transport_delivery_order
                        .lock()
                        .unwrap()
                        .push("commit_started");
                }
                Ok(())
            },
        )
        .await
        .unwrap();
    server.await.unwrap();
    let raw_request = request_rx.await.unwrap();
    let (_, body) = raw_request.split_once("\r\n\r\n").unwrap();
    let payload: Value = serde_json::from_str(body).unwrap();

    assert_eq!(payload["stream"], true);
    assert!(payload.get("temperature").is_none());
    assert_eq!(payload["max_tokens"], 2048);
    assert_eq!(payload["reasoning_effort"], "high");
    assert_eq!(payload["stream_options"]["include_usage"], true);
    assert_eq!(payload["tool_choice"], "auto");
    assert_eq!(payload["parallel_tool_calls"], true);
    assert_eq!(payload["messages"][0]["role"], "system");
    assert_eq!(payload["messages"][1]["content"], "history");
    assert_eq!(payload["messages"][2]["content"], "current");
    assert_eq!(response.text, "hello world");
    assert_eq!(response.usage.unwrap().total_tokens, 11);
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, ModelStreamDelta::Text { .. }))
            .count(),
        2
    );
    assert_eq!(
        *delivery_order.lock().unwrap(),
        vec!["text_delta", "text_delta", "commit_started"],
        "text deltas from a tool-capable request must remain live while tool state stays atomic"
    );
}

#[tokio::test]
async fn openai_provider_reconnects_after_initial_network_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut first).await;
        drop(first);

        let (mut second, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut second).await;
        second
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"reconnected\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        second.shutdown().await.unwrap();
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), model_request()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(response.text, "reconnected");
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            retry_kind: ProviderRetryKind::Network,
            retry_index: Some(1),
            retry_limit: Some(PROVIDER_NETWORK_RETRY_LIMIT),
            cache_trace: Some(cache_trace),
            ..
        } if !cache_trace.segments.is_empty()
    )));
}

#[tokio::test]
async fn openai_provider_honors_rate_limit_retry_after() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut first).await;
        let body = r#"{"error":{"code":"rpm_limited"}}"#;
        first
            .write_all(
                format!(
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nRetry-After: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        first.shutdown().await.unwrap();

        let (mut second, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut second).await;
        second
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"after rate limit\"},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        second.shutdown().await.unwrap();
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), model_request()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(response.text, "after rate limit");
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            retry_kind: ProviderRetryKind::Network,
            retry_index: Some(1),
            reason,
            ..
        } if reason.contains("rate limited")
    )));
}

#[tokio::test]
async fn openai_provider_does_not_retry_a_permanent_quota_rate_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = read_http_request(&mut socket).await;
        let body = r#"{"error":{"code":"insufficient_user_quota"}}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.shutdown().await.unwrap();
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let mut transport = Vec::new();
    let error = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), model_request()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap_err();
    server.await.unwrap();

    assert!(error
        .downcast_ref::<ProviderAdapterError>()
        .is_some_and(|error| matches!(error, ProviderAdapterError::QuotaExhausted { .. })));
    assert!(!transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { .. })));
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Response {
            attempt: 1,
            status: Some(429),
            ..
        }
    )));
}

#[tokio::test]
async fn chat_provider_retries_sse_rate_limit_without_changing_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_http_request(&mut socket).await);
            let response = if attempt == 0 {
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"error\":{\"message\":\"Concurrency limit exceeded for account, please retry later\",\"type\":\"rate_limit_error\",\"retry_after\":0}}\n\n"
                )
            } else {
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_lookup\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\\\"retry\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
            };
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });

    let mut provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), tool_request()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();

    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.tool_calls[0].arguments, json!({"query": "retry"}));
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.contains(r#""stream":true"#)));
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            attempt: 2,
            retry_kind: ProviderRetryKind::Network,
            retry_index: Some(1),
            retry_limit: Some(retry_limit),
            reason,
            ..
        } if *retry_limit == super::transport::PROVIDER_RATE_LIMIT_RETRY_LIMIT
            && reason.contains("rate limited the streamed response")
    )));
    assert!(!transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            retry_kind: ProviderRetryKind::StateRecovery,
            ..
        }
    )));
}

#[tokio::test]
async fn responses_provider_retries_sse_rate_limit_without_changing_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_http_request(&mut socket).await);
            let response = if attempt == 0 {
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"type\":\"error\",\"error\":{\"message\":\"too many concurrent requests\",\"type\":\"rate_limit_error\",\"retry_after_ms\":0}}\n\n"
                )
            } else {
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_retry\"}}\n\n",
                    "data: {\"type\":\"response.output_item.added\",\"response_id\":\"resp_retry\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_lookup\",\"name\":\"lookup\",\"arguments\":\"\"}}\n\n",
                    "data: {\"type\":\"response.output_item.done\",\"response_id\":\"resp_retry\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_lookup\",\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\\\"retry\\\"}\"}}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_retry\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_lookup\",\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\\\"retry\\\"}\"}]}}\n\n",
                    "data: [DONE]\n\n"
                )
            };
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });

    let mut provider =
        OpenAiResponsesProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), tool_request()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    let requests = server.await.unwrap();

    assert_eq!(response.response_id.as_deref(), Some("resp_retry"));
    assert_eq!(response.tool_calls[0].name, "lookup");
    assert_eq!(response.tool_calls[0].arguments, json!({"query": "retry"}));
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.contains(r#""stream":true"#)));
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            attempt: 2,
            retry_kind: ProviderRetryKind::Network,
            retry_index: Some(1),
            retry_limit: Some(retry_limit),
            ..
        } if *retry_limit == super::transport::PROVIDER_RATE_LIMIT_RETRY_LIMIT
    )));
    assert!(!transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            retry_kind: ProviderRetryKind::StateRecovery,
            ..
        }
    )));
}

#[tokio::test]
async fn compatibility_probe_client_retries_429_without_bursting_the_caller() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_http_request(&mut socket).await);
            let (status, retry_after, body) = if attempt == 0 {
                (
                    "429 Too Many Requests",
                    "Retry-After: 0\r\n",
                    r#"{"error":{"code":"rpm_limited"}}"#,
                )
            } else {
                ("200 OK", "", r#"{"ok":true}"#)
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        }
        requests
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let probe_client = super::openai::OpenAiProbeClient::new(&provider);
    let response = probe_client
        .send("/chat/completions", &json!({"model": "test-model"}))
        .await
        .unwrap();
    let (response, _permit) = response.into_parts();
    let requests = server.await.unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn chat_provider_returns_schema_mismatched_arguments_without_transport_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        assert!(request.contains(r#""stream":true"#));
        socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_shell\",\"function\":{\"name\":\"shell\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
    });

    let mut provider = OpenAiCompatibleProvider::new(
        format!("http://{address}/v1"),
        "test-key",
        "schema-mismatch-model",
    );
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate {
        name: "shell".to_string(),
        description: "Run a command".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }),
        ..Default::default()
    }];
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), request.clone()).unwrap(),
            &mut |_| Ok(()),
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(response.tool_calls[0].arguments, json!({}));
    assert!(!transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { .. })));
    let next = provider.prepare(Uuid::new_v4(), request).unwrap();
    assert_eq!(next.body["stream"], true);
}

#[tokio::test]
async fn chat_provider_recovers_invalid_streamed_tool_json_once_without_streaming() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let first_request = read_http_request(&mut first).await;
        assert!(first_request.contains(r#""stream":true"#));
        first
            .write_all(
                concat!(
                    "HTTP/1.1 200 OK\r\n",
                    "Content-Type: text/event-stream\r\n",
                    "Connection: close\r\n\r\n",
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_shell\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                    "data: [DONE]\n\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        first.shutdown().await.unwrap();

        let (mut second, _) = listener.accept().await.unwrap();
        let second_request = read_http_request(&mut second).await;
        assert!(second_request.contains(r#""stream":false"#));
        assert!(!second_request.contains("stream_options"));
        assert!(second_request.contains("previous response could not be decoded as a valid tool call"));
        assert!(second_request.contains("Use double quotes for every object key and every string value"));
        let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"call_shell","type":"function","function":{"name":"shell","arguments":"{\"command\":\"cargo test\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        second
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        second.shutdown().await.unwrap();
    });

    let mut provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate::direct(
        "shell",
        "Run a command",
        json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        }),
    )];
    let mut deltas = Vec::new();
    let mut transport = Vec::new();
    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), request).unwrap(),
            &mut |delta| {
                deltas.push(delta);
                Ok(())
            },
            &mut |event| {
                transport.push(event);
                Ok(())
            },
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(response.tool_calls[0].name, "shell");
    assert_eq!(
        response.tool_calls[0].arguments,
        json!({"command": "cargo test"})
    );
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, ModelStreamDelta::ToolCall { .. }))
            .count(),
        1
    );
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Retry {
            attempt: 2,
            retry_kind: ProviderRetryKind::StateRecovery,
            retry_index: Some(1),
            retry_limit: Some(1),
            cache_trace: Some(cache_trace),
            ..
        } if !cache_trace.segments.is_empty()
    )));
}

#[tokio::test]
async fn chat_provider_reports_protocol_corruption_when_nonstreaming_recovery_is_also_invalid() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for streamed in [true, false] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            assert!(request.contains(&format!(r#""stream":{streamed}"#)));
            if streamed {
                socket
                    .write_all(
                        concat!(
                            "HTTP/1.1 200 OK\r\n",
                            "Content-Type: text/event-stream\r\n",
                            "Connection: close\r\n\r\n",
                            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_lookup\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"query\\\":\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                            "data: [DONE]\n\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            } else {
                let body = r#"{"choices":[{"message":{"tool_calls":[{"id":"call_lookup","type":"function","function":{"name":"lookup","arguments":"{\"query\":"}}]},"finish_reason":"tool_calls"}]}"#;
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
            socket.shutdown().await.unwrap();
        }
    });

    let mut provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    let mut deltas = Vec::new();
    let error = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), tool_request()).unwrap(),
            &mut |delta| {
                deltas.push(delta);
                Ok(())
            },
            &mut |_| Ok(()),
        )
        .await
        .expect_err("both malformed responses must fail");
    server.await.unwrap();

    let message = error.to_string();
    assert!(message.contains("provider tool-call protocol error"));
    assert!(message.contains("both streamed decoding and one non-streaming recovery failed"));
    assert!(!message.contains("capability profile is stale"));
    assert!(
        deltas.is_empty(),
        "invalid atomic attempts must not leak deltas"
    );
}

#[tokio::test]
async fn chat_provider_decodes_arguments_with_the_exact_advertised_strict_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        assert!(request.contains(r#""strict":true"#));
        assert!(request.contains(r#""type":["array","null"]"#));
        socket
                .write_all(
                    concat!(
                        "HTTP/1.1 200 OK\r\n",
                        "Content-Type: text/event-stream\r\n",
                        "Connection: close\r\n\r\n",
                        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_records\",\"function\":{\"name\":\"records\",\"arguments\":\"{\\\"columns\\\":null}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
                        "data: [DONE]\n\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
    });

    let mut provider = OpenAiCompatibleProvider::new(
        format!("http://{address}/v1"),
        "test-key",
        "strict-contract-model",
    );
    provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
    provider.tool_protocol.strict_function_tools = ProviderFeatureSupport::Supported;
    let mut request = model_request();
    request.tool_candidates = vec![ProviderToolCandidate::direct(
        "records",
        "Update records",
        json!({
            "type": "object",
            "properties": {
                "columns": { "type": "array", "items": { "type": "string" } }
            },
            "additionalProperties": false
        }),
    )];

    let response = provider
        .stream_prepared(
            provider.prepare(Uuid::new_v4(), request).unwrap(),
            &mut |_| Ok(()),
            &mut |_| Ok(()),
        )
        .await
        .unwrap();
    server.await.unwrap();

    assert_eq!(response.tool_calls[0].arguments, json!({}));
}

#[tokio::test]
async fn chat_provider_fails_fast_when_upstream_rejects_image_input() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_count_tx, request_count_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _request = read_http_request(&mut socket).await;
        let rejected =
            r#"{"error":{"message":"当前请求包含图片内容，上游不支持","type":"invalid_request"}}"#;
        socket
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
        let second_request =
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_ok();
        request_count_tx
            .send(if second_request { 2 } else { 1 })
            .unwrap();
    });

    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let mut request = model_request();
    request.input.current_user.content = vec![ModelInputContent::image("image/png", vec![1, 2, 3])];
    let prepared = provider.prepare(Uuid::nil(), request).unwrap();
    let mut transport = Vec::new();

    let error = provider
        .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
            transport.push(event);
            Ok(())
        })
        .await
        .expect_err("unsupported image input should fail after one provider request");
    server.await.unwrap();

    assert_eq!(request_count_rx.await.unwrap(), 1);
    assert!(
        error.to_string().contains("does not support image input"),
        "unexpected provider error: {error:#}"
    );
    assert!(transport.iter().any(|event| matches!(
        event,
        ProviderTransportEvent::Response {
            attempt: 1,
            status: Some(400),
            ..
        }
    )));
    assert!(!transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { .. })));
}

#[tokio::test]
async fn portable_message_request_does_not_repeat_the_same_fallback_after_http_400() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_count_tx, request_count_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        assert!(!request.contains(r#""role":"developer""#));
        let rejected = r#"{"error":"invalid model"}"#;
        socket
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
        let repeated = tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_ok();
        request_count_tx.send(if repeated { 2 } else { 1 }).unwrap();
    });
    let provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();
    let mut transport = Vec::new();

    let error = provider
        .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
            transport.push(event);
            Ok(())
        })
        .await
        .expect_err("the provider rejection should be returned without a duplicate retry");
    server.await.unwrap();

    assert!(error.to_string().contains("invalid model"));
    assert_eq!(request_count_rx.await.unwrap(), 1);
    assert!(!transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { .. })));
}

#[tokio::test]
async fn chat_transport_reports_a_stale_role_profile_without_reencoding() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (request_count_tx, request_count_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http_request(&mut socket).await;
        assert!(request.contains(r#""role":"developer""#));
        let rejected = r#"{"error":"unsupported developer role"}"#;
        socket
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        rejected.len(),
                        rejected
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        socket.shutdown().await.unwrap();
        let repeated = tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_ok();
        request_count_tx.send(if repeated { 2 } else { 1 }).unwrap();
    });
    let mut provider =
        OpenAiCompatibleProvider::new(format!("http://{address}/v1"), "test-key", "test-model");
    provider.chat_codec.instruction_encoding = ProviderInstructionEncoding::NativeRoles;
    let prepared = provider
        .prepare(Uuid::nil(), layered_model_request())
        .unwrap();
    let mut transport = Vec::new();

    let error = provider
        .stream_prepared(prepared, &mut |_| Ok(()), &mut |event| {
            transport.push(event);
            Ok(())
        })
        .await
        .expect_err("a stale adapter profile must not trigger hidden prompt reassembly");
    server.await.unwrap();

    assert!(error.to_string().contains("capability profile is stale"));
    assert_eq!(request_count_rx.await.unwrap(), 1);
    assert!(!transport
        .iter()
        .any(|event| matches!(event, ProviderTransportEvent::Retry { .. })));
}
