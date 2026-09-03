use super::super::{
    apply_provider_auth, provider_error_is_quota_exhausted, provider_rejected_image_input,
    provider_transcript_candidate_item, rejected_chat_profile_capability, request_image_part_count,
    truncate_observation_text, ModelFinishReason, ModelResponse, ModelStreamCallback,
    ModelStreamDelta, PreparedProviderRequest, ProviderAdapterError, ProviderResponseCommitMode,
    ProviderTransportCallback, ProviderTransportEvent,
};
use super::recovery::{
    recover_streamed_tool_call_non_streaming, schedule_rate_limited_stream_retry,
    OpenAiRecoveryProtocol, RecoveredToolResponse,
};
use super::stream::provider_stream_rate_limit;
use super::{
    decode_openai_chat_response, decode_openai_responses_response, model_response_observation,
    normalize_provider_tool_calls, tool_call_protocol_error_observation, OpenAiCompatibleProvider,
    OpenAiResponsesProvider, ResponsesRequestError, OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT,
};
use crate::provider::telemetry::{emit_response_headers, ProviderStreamTelemetry};
use crate::provider::transport::{
    provider_stream_stalled, send_provider_request_with_network_retries,
};
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};

impl OpenAiCompatibleProvider {
    pub(super) async fn execute_chat_request(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let mut next_attempt = 1;
        let mut stream_rate_limit_retries = 0;
        loop {
            let (response, attempt) = send_provider_request_with_network_retries(
                || {
                    apply_provider_auth(
                        self.client.post(&prepared.endpoint),
                        self.auth,
                        &self.api_key,
                    )
                    .header(CONTENT_TYPE, "application/json")
                    .json(&prepared.body)
                },
                next_attempt,
                &prepared.observation_body,
                prepared.cache_trace.as_ref(),
                on_transport,
            )
            .await?;
            let status = response.status();
            emit_response_headers(prepared.request_id, attempt, status.as_u16(), on_transport)?;
            if !status.is_success() {
                let body = response.text().await?;
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: json!({ "error": truncate_observation_text(&body) }),
                })?;
                if provider_error_is_quota_exhausted(&body) {
                    return Err(ProviderAdapterError::QuotaExhausted { detail: body }.into());
                }
                if status.as_u16() == 400 {
                    let image_parts = request_image_part_count(&prepared.logical_request);
                    if image_parts > 0 && provider_rejected_image_input(&body) {
                        anyhow::bail!(
                            "provider does not support image input (imageParts={image_parts}); choose a vision-capable model or disable image attachments: {}",
                            truncate_observation_text(&body)
                        );
                    }
                    if let Some(capability) =
                        rejected_chat_profile_capability(&prepared.body, &body)
                    {
                        return Err(ProviderAdapterError::CapabilityProfileStale {
                            capability,
                            detail: body,
                        }
                        .into());
                    }
                }
                anyhow::bail!("provider request failed ({status}): {body}");
            }

            let streamed = prepared
                .body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let atomic_response = prepared.response_commit == ProviderResponseCommitMode::Atomic;
            let atomic_stream = streamed && atomic_response;
            let mut telemetry = ProviderStreamTelemetry::new(prepared.request_id, attempt);
            let mut provisional_deltas = Vec::new();
            let mut tool_call_committed = false;
            let decoded = {
                let mut observed_delta = |mut delta| {
                    normalize_stream_tool_call(&mut delta, &prepared);
                    tool_call_committed |= delta.is_tool_call_done();
                    telemetry.observe(&delta, on_transport)?;
                    if atomic_response && delta.waits_for_atomic_commit() {
                        provisional_deltas.push(delta);
                        Ok(())
                    } else {
                        on_delta(delta)
                    }
                };
                decode_openai_chat_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    &mut observed_delta,
                    true,
                )
                .await
            };
            telemetry.finish_progress(on_transport)?;
            let (mut response, response_attempt, response_status, commit_deltas) = match decoded {
                Ok(response) => (response, attempt, status.as_u16(), provisional_deltas),
                Err(error) if tool_call_committed => {
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body: stream_decode_error_observation(&error),
                    })?;
                    return Err(error);
                }
                Err(error) if atomic_stream && provider_stream_rate_limit(&error).is_some() => {
                    let retry_after =
                        provider_stream_rate_limit(&error).and_then(|error| error.retry_after());
                    let Some(attempt) = schedule_rate_limited_stream_retry(
                        &prepared,
                        attempt,
                        status.as_u16(),
                        stream_rate_limit_retries,
                        retry_after,
                        &error,
                        on_transport,
                    )
                    .await?
                    else {
                        return Err(error);
                    };
                    stream_rate_limit_retries = stream_rate_limit_retries.saturating_add(1);
                    next_attempt = attempt;
                    continue;
                }
                Err(error) if provider_stream_stalled(&error) => {
                    let body = stream_decode_error_observation(&error);
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body,
                    })?;
                    return Err(error);
                }
                Err(error) if atomic_stream => {
                    let mut recovered_delta = |mut delta| {
                        normalize_stream_tool_call(&mut delta, &prepared);
                        on_delta(delta)
                    };
                    let RecoveredToolResponse {
                        response,
                        attempt,
                        status,
                    } = recover_streamed_tool_call_non_streaming(
                        OpenAiRecoveryProtocol::ChatCompletions,
                        &self.client,
                        self.auth,
                        &self.api_key,
                        &prepared,
                        attempt,
                        status.as_u16(),
                        &error,
                        &mut recovered_delta,
                        on_transport,
                    )
                    .await?;
                    (response, attempt, status, Vec::new())
                }
                Err(error) => {
                    let body = stream_decode_error_observation(&error);
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body,
                    })?;
                    return Err(error);
                }
            };
            normalize_provider_tool_calls(&mut response.tool_calls, &prepared.tool_contracts);
            if response_attempt == attempt {
                telemetry.emit_commit_started(on_transport)?;
            }
            for delta in commit_deltas {
                on_delta(delta)?;
            }
            on_transport(ProviderTransportEvent::Response {
                attempt: response_attempt,
                status: Some(response_status),
                response_id: response.response_id.clone(),
                body: model_response_observation(&response),
            })?;
            attach_completed_chat_transcript(&mut response, &prepared);
            return Ok(response);
        }
    }
}

fn attach_completed_chat_transcript(
    response: &mut ModelResponse,
    prepared: &PreparedProviderRequest,
) {
    if response.finish_reason != ModelFinishReason::Stop || !response.tool_calls.is_empty() {
        return;
    }
    let Some(mut transcript) = prepared.wire_transcript.clone() else {
        return;
    };
    transcript.items.push(json!({
        "role": "assistant",
        "content": &response.text,
    }));
    response
        .provider_items
        .push(provider_transcript_candidate_item(&transcript));
}

impl OpenAiResponsesProvider {
    pub(super) async fn execute_responses_request(
        &self,
        prepared: PreparedProviderRequest,
        attempt: usize,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let mut next_attempt = attempt;
        let mut stream_rate_limit_retries = 0;
        loop {
            let (response, transport_attempt) = send_provider_request_with_network_retries(
                || {
                    apply_provider_auth(
                        self.client.post(&prepared.endpoint),
                        self.auth,
                        &self.api_key,
                    )
                    .header(CONTENT_TYPE, "application/json")
                    .json(&prepared.body)
                },
                next_attempt,
                &prepared.observation_body,
                prepared.cache_trace.as_ref(),
                on_transport,
            )
            .await?;
            let attempt = transport_attempt;
            let status = response.status();
            emit_response_headers(prepared.request_id, attempt, status.as_u16(), on_transport)?;
            if !status.is_success() {
                let body = response.text().await?;
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: json!({ "error": truncate_observation_text(&body) }),
                })?;
                if provider_error_is_quota_exhausted(&body) {
                    return Err(ProviderAdapterError::QuotaExhausted { detail: body }.into());
                }
                return Err(ResponsesRequestError { status, body }.into());
            }

            let streamed = prepared
                .body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let atomic_response = prepared.response_commit == ProviderResponseCommitMode::Atomic;
            let atomic_stream = streamed && atomic_response;
            let mut telemetry = ProviderStreamTelemetry::new(prepared.request_id, attempt);
            let mut provisional_deltas = Vec::new();
            let mut tool_call_committed = false;
            let decoded = {
                let mut observed_delta = |mut delta| {
                    normalize_stream_tool_call(&mut delta, &prepared);
                    tool_call_committed |= delta.is_tool_call_done();
                    telemetry.observe(&delta, on_transport)?;
                    if atomic_response && delta.waits_for_atomic_commit() {
                        provisional_deltas.push(delta);
                        Ok(())
                    } else {
                        on_delta(delta)
                    }
                };
                decode_openai_responses_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    &mut observed_delta,
                    true,
                )
                .await
            };
            telemetry.finish_progress(on_transport)?;
            let (mut response, response_attempt, response_status, commit_deltas) = match decoded {
                Ok(response) => (response, attempt, status.as_u16(), provisional_deltas),
                Err(error) if tool_call_committed => {
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body: stream_decode_error_observation(&error),
                    })?;
                    return Err(error);
                }
                Err(error) if atomic_stream && provider_stream_rate_limit(&error).is_some() => {
                    let retry_after =
                        provider_stream_rate_limit(&error).and_then(|error| error.retry_after());
                    let Some(attempt) = schedule_rate_limited_stream_retry(
                        &prepared,
                        attempt,
                        status.as_u16(),
                        stream_rate_limit_retries,
                        retry_after,
                        &error,
                        on_transport,
                    )
                    .await?
                    else {
                        return Err(error);
                    };
                    stream_rate_limit_retries = stream_rate_limit_retries.saturating_add(1);
                    next_attempt = attempt;
                    continue;
                }
                Err(error) if provider_stream_stalled(&error) => {
                    let body = stream_decode_error_observation(&error);
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body,
                    })?;
                    return Err(error);
                }
                Err(error) if atomic_stream => {
                    let mut recovered_delta = |mut delta| {
                        normalize_stream_tool_call(&mut delta, &prepared);
                        on_delta(delta)
                    };
                    let RecoveredToolResponse {
                        response,
                        attempt,
                        status,
                    } = recover_streamed_tool_call_non_streaming(
                        OpenAiRecoveryProtocol::Responses,
                        &self.client,
                        self.auth,
                        &self.api_key,
                        &prepared,
                        attempt,
                        status.as_u16(),
                        &error,
                        &mut recovered_delta,
                        on_transport,
                    )
                    .await?;
                    (response, attempt, status, Vec::new())
                }
                Err(error) => {
                    let body = stream_decode_error_observation(&error);
                    on_transport(ProviderTransportEvent::Response {
                        attempt,
                        status: Some(status.as_u16()),
                        response_id: None,
                        body,
                    })?;
                    return Err(error);
                }
            };
            normalize_provider_tool_calls(&mut response.tool_calls, &prepared.tool_contracts);
            if response_attempt == attempt {
                telemetry.emit_commit_started(on_transport)?;
            }
            for delta in commit_deltas {
                on_delta(delta)?;
            }
            on_transport(ProviderTransportEvent::Response {
                attempt: response_attempt,
                status: Some(response_status),
                response_id: response.response_id.clone(),
                body: model_response_observation(&response),
            })?;
            attach_completed_responses_transcript(&mut response, &prepared);
            return Ok(response);
        }
    }
}

fn normalize_stream_tool_call(delta: &mut ModelStreamDelta, prepared: &PreparedProviderRequest) {
    let ModelStreamDelta::ToolCallDone { call, .. } = delta else {
        return;
    };
    normalize_provider_tool_calls(std::slice::from_mut(call), &prepared.tool_contracts);
}

fn attach_completed_responses_transcript(
    response: &mut ModelResponse,
    prepared: &PreparedProviderRequest,
) {
    let Some(mut transcript) = prepared.wire_transcript.clone() else {
        return;
    };
    transcript.format = OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT.to_string();
    transcript
        .items
        .extend(response.provider_items.iter().cloned());
    response
        .provider_items
        .push(provider_transcript_candidate_item(&transcript));
}

fn stream_decode_error_observation(error: &anyhow::Error) -> Value {
    if provider_stream_rate_limit(error).is_some() {
        json!({
            "error": truncate_observation_text(&error.to_string()),
            "classification": "rate_limit",
            "recovery": "unsafe_after_stream_commit"
        })
    } else {
        tool_call_protocol_error_observation(error, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{provider_wire_transcript, ModelRequest, ProviderWireTranscript};
    use uuid::Uuid;

    #[test]
    fn completed_chat_response_produces_a_transcript_cursor_candidate() {
        let transcript = ProviderWireTranscript {
            format: "openai_chat_native_messages_v1".to_string(),
            items: vec![json!({ "role": "user", "content": "question" })],
        };
        let prepared = PreparedProviderRequest {
            request_id: Uuid::nil(),
            adapter: "openai_chat_completions".to_string(),
            method: "POST".to_string(),
            endpoint: "https://example.test/chat/completions".to_string(),
            body: json!({}),
            observation_body: json!({}),
            cache_trace: None,
            logical_request: ModelRequest {
                instructions: Default::default(),
                input: Default::default(),
                tool_candidates: Vec::new(),
                previous_response_items: Vec::new(),
                provider_transcript: None,
                previous_response_id: None,
                prompt_cache_breakpoint_policy: Default::default(),
                final_output_json_schema: None,
            },
            wire_transcript: Some(transcript.clone()),
            tool_contracts: Vec::new(),
            response_commit: ProviderResponseCommitMode::Streaming,
        };
        let mut response = ModelResponse::text("answer");

        attach_completed_chat_transcript(&mut response, &prepared);

        let completed =
            provider_wire_transcript(&response.provider_items[0]).expect("transcript candidate");
        assert_eq!(completed.format, transcript.format);
        assert_eq!(completed.items[..transcript.items.len()], transcript.items);
        assert_eq!(completed.items.last().unwrap()["role"], "assistant");
        assert_eq!(completed.items.last().unwrap()["content"], "answer");
    }

    #[test]
    fn completed_responses_response_extends_the_exact_request_transcript() {
        let transcript = ProviderWireTranscript {
            format: "openai_responses_request_input_v1".to_string(),
            items: vec![json!({ "role": "user", "content": "question" })],
        };
        let prepared = PreparedProviderRequest {
            request_id: Uuid::nil(),
            adapter: "openai_responses".to_string(),
            method: "POST".to_string(),
            endpoint: "https://example.test/responses".to_string(),
            body: json!({}),
            observation_body: json!({}),
            cache_trace: None,
            logical_request: ModelRequest {
                instructions: Default::default(),
                input: Default::default(),
                tool_candidates: Vec::new(),
                previous_response_items: Vec::new(),
                provider_transcript: None,
                previous_response_id: None,
                prompt_cache_breakpoint_policy: Default::default(),
                final_output_json_schema: None,
            },
            wire_transcript: Some(transcript.clone()),
            tool_contracts: Vec::new(),
            response_commit: ProviderResponseCommitMode::Atomic,
        };
        let provider_item = json!({
            "type": "function_call",
            "call_id": "call_1",
            "name": "lookup",
            "arguments": "{}",
        });
        let mut response = ModelResponse::text("");
        response.provider_items.push(provider_item.clone());

        attach_completed_responses_transcript(&mut response, &prepared);

        assert_eq!(response.provider_items[0], provider_item);
        let completed = provider_wire_transcript(response.provider_items.last().unwrap())
            .expect("Responses transcript candidate");
        assert_eq!(
            completed.format,
            OPENAI_RESPONSES_COMPLETED_TRANSCRIPT_FORMAT
        );
        assert_eq!(completed.items[..transcript.items.len()], transcript.items);
        assert_eq!(completed.items.last(), Some(&provider_item));
    }
}
