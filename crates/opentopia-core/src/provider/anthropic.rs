use super::{
    apply_provider_auth, chat_finish_reason, compatibility_probe_candidate, emit_response_deltas,
    ensure_visual_input_supported, is_tool_call_protocol_error, model_response_observation,
    next_stream_chunk, parse_model_usage, parse_required_tool_arguments, provider_api_key,
    redact_transport_value, require_function_tools, send_provider_request_with_network_retries,
    stream_idle_timeout, tool_call_protocol_error_observation, truncate_observation_text,
    validate_provider_response_protocol, validate_tool_probe_response, ModelFinishReason,
    ModelProvider, ModelRequest, ModelResponse, ModelStreamCallback, ModelStreamDelta, ModelUsage,
    OpenAiProbeOutcome, PreparedProviderRequest, ProviderAdapterError, ProviderResponseCommitMode,
    ProviderToolCall, ProviderToolCandidate, ProviderTransportCallback, ProviderTransportEvent,
    SseDecoder, StreamingToolCall,
};
use crate::settings::{
    ProviderAdapterKind, ProviderAuthKind, ProviderFeatureSupport, ProviderHealthCheck,
    ProviderSettings, ProviderToolProtocolCapabilities,
};
use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

mod codec;

use codec::anthropic_messages;
#[cfg(test)]
pub(in crate::provider) use codec::anthropic_tool_result;
pub(in crate::provider) use codec::{anthropic_system_instructions, anthropic_tools};
/// Native Anthropic Messages API adapter. This intentionally does not reuse
/// the OpenAI-compatible transport: headers, request shape, tool calls, and
/// streamed events are different protocols.
#[derive(Debug, Clone)]
pub struct AnthropicMessagesProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    auth: ProviderAuthKind,
    model: String,
    temperature: Option<f64>,
    max_output_tokens: Option<u32>,
    pub(super) reasoning_effort: Option<String>,
    supports_vision: bool,
    pub(super) tool_protocol: ProviderToolProtocolCapabilities,
}

impl AnthropicMessagesProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            auth: ProviderAuthKind::XApiKey,
            model: model.into(),
            temperature: None,
            max_output_tokens: None,
            reasoning_effort: None,
            supports_vision: true,
            tool_protocol: ProviderToolProtocolCapabilities {
                function_tools: ProviderFeatureSupport::Supported,
                ..ProviderToolProtocolCapabilities::default()
            },
        }
    }

    pub fn from_settings(settings: &ProviderSettings) -> Option<Self> {
        let api_key = provider_api_key(settings)?;
        let mut provider = Self::new(settings.base_url.clone(), api_key, settings.model.clone());
        provider.auth = settings.effective_auth();
        if let Some(temperature) = settings.temperature_for_model() {
            provider.temperature = Some(temperature);
        }
        provider.max_output_tokens = settings.max_output_tokens_for_model();
        provider.reasoning_effort = settings.reasoning_effort_for_model();
        provider.supports_vision = settings.supports_vision_for_model();
        provider.tool_protocol = settings
            .adapter_profile_for_model_and_adapter(
                &settings.model,
                ProviderAdapterKind::AnthropicMessages,
            )
            .map(|profile| profile.tool_protocol)
            .unwrap_or_default();
        Some(provider)
    }

    pub(super) async fn probe_tool_capabilities(&self) -> (OpenAiProbeOutcome, OpenAiProbeOutcome) {
        const TOOL_PROBE_TOKEN: &str = "opentopia-tool-probe-v1";
        tokio::join!(
            self.probe_tool_roundtrip(false, TOOL_PROBE_TOKEN),
            self.probe_tool_roundtrip(true, TOOL_PROBE_TOKEN),
        )
    }

    async fn probe_tool_roundtrip(
        &self,
        streamed: bool,
        expected_token: &str,
    ) -> OpenAiProbeOutcome {
        let endpoint = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let candidate = compatibility_probe_candidate(expected_token);
        let payload = json!({
            "model": self.model,
            "max_tokens": 64,
            "stream": streamed,
            "messages": [{
                "role": "user",
                "content": format!(
                    "Call compatibility_probe exactly once with token {expected_token}."
                )
            }],
            "tools": [{
                "name": &candidate.name,
                "description": &candidate.description,
                "input_schema": &candidate.input_schema,
            }],
            "tool_choice": {
                "type": "tool",
                "name": "compatibility_probe"
            }
        });
        let response = tokio::time::timeout(
            Duration::from_secs(20),
            apply_provider_auth(self.client.post(endpoint), self.auth, &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header(CONTENT_TYPE, "application/json")
                .json(&payload)
                .send(),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Unknown,
                    detail: Some(error.to_string()),
                }
            }
            Err(_) => {
                return OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Unknown,
                    detail: Some("request timed out after 20 seconds".to_string()),
                }
            }
        };
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return OpenAiProbeOutcome {
                support: if matches!(status.as_u16(), 400 | 404 | 405 | 422 | 501) {
                    ProviderFeatureSupport::Unsupported
                } else {
                    ProviderFeatureSupport::Unknown
                },
                detail: Some(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    truncate_observation_text(body.trim())
                )),
            };
        }
        let candidates = [candidate];
        let mut ignore_delta = |_| Ok(());
        let decoded = tokio::time::timeout(
            Duration::from_secs(20),
            decode_anthropic_messages_response(
                response,
                streamed,
                &candidates,
                &mut ignore_delta,
                false,
            ),
        )
        .await;
        match decoded {
            Ok(Ok(response)) => match validate_tool_probe_response(&response, expected_token) {
                Ok(()) => OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Supported,
                    detail: None,
                },
                Err(detail) => OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Unsupported,
                    detail: Some(detail),
                },
            },
            Ok(Err(error)) => OpenAiProbeOutcome {
                support: ProviderFeatureSupport::Unsupported,
                detail: Some(error.to_string()),
            },
            Err(_) => OpenAiProbeOutcome {
                support: ProviderFeatureSupport::Unknown,
                detail: Some("tool response did not complete within 20 seconds".to_string()),
            },
        }
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.temperature = Some(0.0);
        self.max_output_tokens = Some(self.max_output_tokens.unwrap_or(1_024).min(1_024));
        if self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("low".to_string());
        }
        self
    }

    fn prepare_messages_request(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        ensure_visual_input_supported(&request, self.supports_vision)?;
        if !request.tool_candidates.is_empty() {
            require_function_tools("Anthropic Messages", self.tool_protocol)?;
        }
        let endpoint = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let tool_capable = !request.tool_candidates.is_empty();
        let stream = !tool_capable
            || self.tool_protocol.streaming_tools == ProviderFeatureSupport::Supported;
        let mut payload = json!({
            "model": self.model,
            "max_tokens": self.max_output_tokens.unwrap_or(4_096),
            "stream": stream,
            "messages": anthropic_messages(&request),
        });
        if let Some(temperature) = self.temperature {
            payload["temperature"] = json!(temperature);
        }
        if let Some(reasoning_effort) = self
            .reasoning_effort
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            if reasoning_effort == "none" {
                payload["thinking"] = json!({ "type": "disabled" });
            } else if model_supports_anthropic_adaptive_thinking(&self.model)
                && anthropic_effort_value(reasoning_effort).is_some()
            {
                payload["thinking"] = json!({ "type": "adaptive" });
                payload["output_config"] = json!({ "effort": reasoning_effort });
            }
        }
        let instructions = anthropic_system_instructions(&request);
        if !instructions.trim().is_empty() {
            payload["system"] = json!(instructions);
        }
        if !request.tool_candidates.is_empty() {
            payload["tools"] = json!(anthropic_tools(&request.tool_candidates));
        }
        Ok(PreparedProviderRequest {
            request_id,
            adapter: "anthropic_messages".to_string(),
            method: "POST".to_string(),
            endpoint,
            observation_body: redact_transport_value(&payload),
            cache_trace: crate::build_provider_cache_trace(&payload, None, false),
            body: payload,
            logical_request: request,
            wire_transcript: None,
            tool_contracts: Vec::new(),
            response_commit: if tool_capable {
                ProviderResponseCommitMode::Atomic
            } else {
                ProviderResponseCommitMode::Streaming
            },
        })
    }

    async fn execute_messages_request(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let (response, attempt) = send_provider_request_with_network_retries(
            || {
                apply_provider_auth(
                    self.client.post(&prepared.endpoint),
                    self.auth,
                    &self.api_key,
                )
                .header("anthropic-version", "2023-06-01")
                .header(CONTENT_TYPE, "application/json")
                .json(&prepared.body)
            },
            1,
            &prepared.observation_body,
            prepared.cache_trace.as_ref(),
            on_transport,
        )
        .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            on_transport(ProviderTransportEvent::Response {
                attempt,
                status: Some(status.as_u16()),
                response_id: None,
                body: json!({ "error": truncate_observation_text(&body) }),
            })?;
            anyhow::bail!("Anthropic Messages request failed ({status}): {body}");
        }

        let streamed = prepared
            .body
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let decoded = decode_anthropic_messages_response(
            response,
            streamed,
            &prepared.logical_request.tool_candidates,
            on_delta,
            true,
        )
        .await;
        let response = match decoded {
            Ok(response) => response,
            Err(error)
                if streamed
                    && prepared.response_commit == ProviderResponseCommitMode::Atomic
                    && is_tool_call_protocol_error(&error) =>
            {
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: tool_call_protocol_error_observation(
                        &error,
                        Some("capability_profile_stale"),
                    ),
                })?;
                return Err(ProviderAdapterError::CapabilityProfileStale {
                    capability: "streaming_tools",
                    detail: error.to_string(),
                }
                .into());
            }
            Err(error) => {
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: tool_call_protocol_error_observation(&error, None),
                })?;
                return Err(error);
            }
        };
        on_transport(ProviderTransportEvent::Response {
            attempt,
            status: Some(status.as_u16()),
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }
}

async fn decode_anthropic_messages_response(
    mut response: reqwest::Response,
    streamed: bool,
    tool_candidates: &[ProviderToolCandidate],
    on_delta: &mut ModelStreamCallback<'_>,
    emit_nonstream_deltas: bool,
) -> anyhow::Result<ModelResponse> {
    if !streamed {
        let body: Value = response.json().await?;
        let response = validate_provider_response_protocol(parse_anthropic_messages_body(
            &body,
            tool_candidates,
        )?)?;
        if emit_nonstream_deltas {
            emit_response_deltas(&response, on_delta)?;
        }
        return Ok(response);
    }

    let mut decoder = SseDecoder::default();
    let mut accumulator = AnthropicStreamAccumulator::default();
    let idle_timeout = stream_idle_timeout();
    loop {
        let Some(chunk) = next_stream_chunk(&mut response, idle_timeout).await? else {
            break;
        };
        for data in decoder.push(&chunk)? {
            let event: Value = serde_json::from_str(&data).map_err(|error| {
                anyhow::anyhow!("invalid Anthropic Messages SSE data: {error}: {data}")
            })?;
            accumulator.apply(&event, on_delta)?;
        }
    }
    for data in decoder.finish()? {
        let event: Value = serde_json::from_str(&data).map_err(|error| {
            anyhow::anyhow!("invalid Anthropic Messages SSE data: {error}: {data}")
        })?;
        accumulator.apply(&event, on_delta)?;
    }
    validate_provider_response_protocol(accumulator.finish_with_tools(tool_candidates)?)
}

fn parse_anthropic_messages_body(
    body: &Value,
    tool_candidates: &[ProviderToolCandidate],
) -> anyhow::Result<ModelResponse> {
    if let Some(error) = body.get("error") {
        anyhow::bail!("Anthropic Messages response returned an error: {error}");
    }
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for (index, block) in body
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(value) = block.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                }
            }
            Some("tool_use") => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "provider tool-call protocol error: Anthropic tool_use block {index} was missing a name"
                        )
                    })?;
                if !tool_candidates.is_empty()
                    && !tool_candidates
                        .iter()
                        .any(|candidate| candidate.name == name)
                {
                    anyhow::bail!(
                        "provider tool-call protocol error: Anthropic returned unknown tool '{name}'"
                    );
                }
                let input = block.get("input").filter(|value| value.is_object()).ok_or_else(|| {
                    anyhow::anyhow!(
                        "provider tool-call protocol error: Anthropic tool_use '{name}' input was not an object"
                    )
                })?;
                tool_calls.push(ProviderToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{index}")),
                    name: name.to_string(),
                    arguments: input.clone(),
                });
            }
            _ => {}
        }
    }
    let finish_reason = if tool_calls.is_empty() {
        body.get("stop_reason")
            .and_then(Value::as_str)
            .map(chat_finish_reason)
            .unwrap_or(ModelFinishReason::Stop)
    } else {
        ModelFinishReason::ToolCalls
    };
    Ok(ModelResponse {
        text,
        tool_calls,
        usage: parse_model_usage(body.get("usage")),
        response_id: body.get("id").and_then(Value::as_str).map(str::to_string),
        provider_items: Vec::new(),
        finish_reason,
    })
}

#[async_trait]
impl ModelProvider for AnthropicMessagesProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, &mut |_| Ok(()), &mut |_| Ok(()))
            .await
    }

    fn prepare(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.prepare_messages_request(request_id, request)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let prepared = self.prepare(Uuid::new_v4(), request)?;
        self.stream_prepared(prepared, on_delta, &mut |_| Ok(()))
            .await
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        self.execute_messages_request(prepared, on_delta, on_transport)
            .await
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();
        // Catalog access alone does not prove the selected model can serve a
        // conversation (for example, an exhausted account may still list
        // models). Validate the native adapter with one minimal Messages call.
        let endpoint = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        match tokio::time::timeout(
            Duration::from_secs(20),
            apply_provider_auth(self.client.post(endpoint), self.auth, &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header(CONTENT_TYPE, "application/json")
                .json(&json!({
                    "model": self.model,
                    "max_tokens": 1,
                    "stream": false,
                    "messages": [{"role": "user", "content": "Reply with OK."}],
                }))
                .send(),
        )
        .await
        {
            Ok(Ok(response)) => {
                let reachable = response.status().is_success();
                Ok(ProviderHealthCheck {
                    reachable,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    model_available: reachable,
                    error: (!reachable).then(|| format!("HTTP {}", response.status())),
                    openai_compatibility: None,
                })
            }
            Ok(Err(error)) => Ok(ProviderHealthCheck {
                reachable: false,
                latency_ms: Some(start.elapsed().as_millis() as u64),
                model_available: false,
                error: Some(error.to_string()),
                openai_compatibility: None,
            }),
            Err(_) => Ok(ProviderHealthCheck {
                reachable: false,
                latency_ms: None,
                model_available: false,
                error: Some("timeout".to_string()),
                openai_compatibility: None,
            }),
        }
    }
}

#[derive(Debug, Default)]
struct AnthropicStreamAccumulator {
    text: String,
    tool_calls: BTreeMap<usize, StreamingToolCall>,
    usage: Option<ModelUsage>,
    response_id: Option<String>,
    finish_reason: Option<ModelFinishReason>,
}

impl AnthropicStreamAccumulator {
    fn apply(
        &mut self,
        event: &Value,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(event_type, "error") || event.get("error").is_some() {
            anyhow::bail!("Anthropic Messages stream returned an error: {event}");
        }
        match event_type {
            "message_start" => {
                self.response_id = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.update_usage(event.pointer("/message/usage"), on_delta)?;
            }
            "content_block_start" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = event.get("content_block").unwrap_or(&Value::Null);
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let call = self.tool_calls.entry(index).or_default();
                    call.id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    call.name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if block.get("input").is_some_and(|input| input != &json!({})) {
                        call.arguments = block["input"].to_string();
                    }
                    on_delta(ModelStreamDelta::ToolCall {
                        index,
                        id: (!call.id.is_empty()).then(|| call.id.clone()),
                        name: (!call.name.is_empty()).then(|| call.name.clone()),
                        arguments_delta: String::new(),
                    })?;
                }
            }
            "content_block_delta" => {
                let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = event.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            self.text.push_str(text);
                            on_delta(ModelStreamDelta::Text {
                                text: text.to_string(),
                            })?;
                        }
                    }
                    Some("input_json_delta") => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let call = self.tool_calls.entry(index).or_default();
                        call.arguments.push_str(partial);
                        on_delta(ModelStreamDelta::ToolCall {
                            index,
                            id: (!call.id.is_empty()).then(|| call.id.clone()),
                            name: (!call.name.is_empty()).then(|| call.name.clone()),
                            arguments_delta: partial.to_string(),
                        })?;
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.finish_reason = Some(chat_finish_reason(reason));
                }
                self.update_usage(event.get("usage"), on_delta)?;
            }
            "message_stop" => {
                self.finish_reason.get_or_insert(ModelFinishReason::Stop);
            }
            _ => {}
        }
        Ok(())
    }

    fn update_usage(
        &mut self,
        value: Option<&Value>,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<()> {
        let Some(value) = value else {
            return Ok(());
        };
        let previous = self.usage.clone().unwrap_or_default();
        let mut usage = parse_model_usage(Some(value)).unwrap_or_default();
        if value.get("input_tokens").is_none() {
            usage.input_tokens = previous.input_tokens;
        }
        if value.get("output_tokens").is_none() {
            usage.output_tokens = previous.output_tokens;
        }
        usage.total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
        self.usage = Some(usage.clone());
        on_delta(ModelStreamDelta::Usage { usage })
    }

    #[cfg(test)]
    fn finish(self) -> anyhow::Result<ModelResponse> {
        self.finish_with_tools(&[])
    }

    fn finish_with_tools(
        self,
        tool_candidates: &[ProviderToolCandidate],
    ) -> anyhow::Result<ModelResponse> {
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|(index, call)| {
                if call.name.is_empty() {
                    anyhow::bail!(
                        "provider tool-call protocol error: Anthropic tool call {index} was missing a name"
                    );
                }
                if !tool_candidates.is_empty()
                    && !tool_candidates
                        .iter()
                        .any(|candidate| candidate.name == call.name)
                {
                    anyhow::bail!(
                        "provider tool-call protocol error: Anthropic returned unknown tool '{}'",
                        call.name
                    );
                }
                let arguments = if call.arguments.trim().is_empty() {
                    json!({})
                } else {
                    parse_required_tool_arguments(
                        Some(&Value::String(call.arguments)),
                        "Anthropic tool_use.input",
                        Some(&call.name),
                    )?
                };
                Ok(ProviderToolCall {
                    id: if call.id.is_empty() {
                        format!("call_{index}")
                    } else {
                        call.id
                    },
                    name: call.name,
                    arguments,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let finish_reason = if !tool_calls.is_empty() {
            ModelFinishReason::ToolCalls
        } else {
            self.finish_reason
                .unwrap_or(ModelFinishReason::StreamInterrupted)
        };
        Ok(ModelResponse {
            text: self.text,
            tool_calls,
            usage: self.usage,
            response_id: self.response_id,
            provider_items: Vec::new(),
            finish_reason,
        })
    }
}

fn model_supports_anthropic_adaptive_thinking(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    let model = model.rsplit('/').next().unwrap_or(&model);
    let model = model.strip_prefix("anthropic.").unwrap_or(model);
    [
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-sonnet-4-6",
        "claude-sonnet-5",
        "claude-fable-5",
        "claude-mythos-5",
        "claude-mythos-preview",
    ]
    .iter()
    .any(|prefix| model.starts_with(prefix))
}

fn anthropic_effort_value(value: &str) -> Option<&str> {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max").then_some(value)
}

#[cfg(test)]
mod tests {
    use super::super::{
        ModelInputContent, ModelInputLedger, ModelUserInput, PromptCacheBreakpointPolicy,
    };
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn model_request() -> ModelRequest {
        ModelRequest {
            instructions: Default::default(),
            input: ModelInputLedger {
                current_user: ModelUserInput {
                    message: "current".to_string(),
                    content: Vec::new(),
                },
                ..Default::default()
            },
            tool_candidates: Vec::new(),
            previous_response_items: Vec::new(),
            provider_transcript: None,
            previous_response_id: None,
            prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy::StableOnly,
            final_output_json_schema: None,
        }
    }

    #[test]
    fn anthropic_provider_uses_native_messages_protocol() {
        let mut provider = AnthropicMessagesProvider::new(
            "https://api.anthropic.com",
            "test-key",
            "claude-sonnet-4-20250514",
        );
        provider.tool_protocol.streaming_tools = ProviderFeatureSupport::Supported;
        let mut request = model_request();
        request.tool_candidates.push(ProviderToolCandidate {
            name: "read_file".to_string(),
            description: "Read a workspace file".to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
            ..Default::default()
        });
        request
            .input
            .current_user
            .content
            .push(ModelInputContent::image(
                "image/png",
                vec![0x89, b'P', b'N', b'G'],
            ));

        let prepared = provider.prepare(Uuid::nil(), request).unwrap();

        assert_eq!(prepared.adapter, "anthropic_messages");
        assert_eq!(prepared.endpoint, "https://api.anthropic.com/v1/messages");
        assert_eq!(prepared.body["stream"], true);
        assert_eq!(prepared.body["max_tokens"], 4_096);
        assert_eq!(prepared.body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(
            prepared.body["messages"][0]["content"][1]["source"]["type"],
            "base64"
        );
    }

    #[test]
    fn anthropic_adaptive_thinking_uses_output_config_effort() {
        let mut provider = AnthropicMessagesProvider::new(
            "https://api.anthropic.com",
            "test-key",
            "claude-sonnet-4-6",
        );
        provider.reasoning_effort = Some("max".to_string());

        let prepared = provider.prepare(Uuid::nil(), model_request()).unwrap();

        assert_eq!(prepared.body["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(prepared.body["output_config"], json!({ "effort": "max" }));
    }

    #[test]
    fn anthropic_stream_accumulates_text_tools_and_usage() {
        let mut accumulator = AnthropicStreamAccumulator::default();
        accumulator
            .apply(
                &json!({
                    "type": "message_start",
                    "message": { "id": "msg_1", "usage": { "input_tokens": 12 } }
                }),
                &mut |_| Ok(()),
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use", "id": "tool_1", "name": "read_file", "input": {}
                    }
                }),
                &mut |_| Ok(()),
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": { "type": "input_json_delta", "partial_json": "{\"path\":\"README.md\"}" }
                }),
                &mut |_| Ok(()),
            )
            .unwrap();
        accumulator
            .apply(
                &json!({
                    "type": "message_delta",
                    "delta": { "stop_reason": "tool_use" },
                    "usage": { "output_tokens": 7 }
                }),
                &mut |_| Ok(()),
            )
            .unwrap();

        let response = accumulator.finish().unwrap();
        assert_eq!(response.finish_reason, ModelFinishReason::ToolCalls);
        assert_eq!(response.response_id.as_deref(), Some("msg_1"));
        assert_eq!(response.usage.unwrap().total_tokens, 19);
        assert_eq!(response.tool_calls[0].id, "tool_1");
        assert_eq!(response.tool_calls[0].arguments["path"], "README.md");
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

    #[tokio::test]
    async fn anthropic_probe_distinguishes_nonstream_and_streaming_tool_protocols() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_http_request(&mut socket).await;
                if request.contains(r#""stream":true"#) {
                    socket
                        .write_all(
                            concat!(
                                "HTTP/1.1 200 OK\r\n",
                                "Content-Type: text/event-stream\r\n",
                                "Connection: close\r\n\r\n",
                                "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_probe\"}}\n\n",
                                "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"probe_call\",\"name\":\"compatibility_probe\",\"input\":{}}}\n\n",
                                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\"}}\n\n",
                                "data: {\"type\":\"message_stop\"}\n\n"
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                } else {
                    let body = r#"{"id":"msg_probe","content":[{"type":"tool_use","id":"probe_call","name":"compatibility_probe","input":{"token":"opentopia-tool-probe-v1"}}],"stop_reason":"tool_use"}"#;
                    socket
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(), body
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                socket.shutdown().await.unwrap();
            }
        });

        let provider =
            AnthropicMessagesProvider::new(format!("http://{address}"), "test-key", "probe-model");
        let (nonstream, streaming) = provider.probe_tool_capabilities().await;
        server.await.unwrap();

        assert_eq!(nonstream.support, ProviderFeatureSupport::Supported);
        assert_eq!(streaming.support, ProviderFeatureSupport::Unsupported);
        assert!(streaming
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("provider tool-call protocol error")));
    }
}
