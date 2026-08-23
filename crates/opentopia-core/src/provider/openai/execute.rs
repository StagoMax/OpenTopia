use super::super::{
    apply_provider_auth, provider_rejected_image_input, rejected_chat_profile_capability,
    request_image_part_count, truncate_observation_text, ModelResponse, ModelStreamCallback,
    PreparedProviderRequest, ProviderAdapterError, ProviderResponseCommitMode,
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
    OpenAiResponsesProvider, ResponsesRequestError,
};
use crate::provider::transport::send_provider_request_with_network_retries;
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
            if !status.is_success() {
                let body = response.text().await?;
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: json!({ "error": truncate_observation_text(&body) }),
                })?;
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
            let atomic_stream =
                streamed && prepared.response_commit == ProviderResponseCommitMode::Atomic;
            let mut provisional_deltas = Vec::new();
            let decoded = if atomic_stream {
                let mut buffer_delta = |delta| {
                    provisional_deltas.push(delta);
                    Ok(())
                };
                decode_openai_chat_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    &mut buffer_delta,
                    true,
                )
                .await
            } else {
                decode_openai_chat_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    on_delta,
                    true,
                )
                .await
            };
            let (mut response, response_attempt, response_status, commit_deltas) = match decoded {
                Ok(response) => (response, attempt, status.as_u16(), provisional_deltas),
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
                Err(error) if atomic_stream => {
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
                        on_delta,
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
            for delta in commit_deltas {
                on_delta(delta)?;
            }
            on_transport(ProviderTransportEvent::Response {
                attempt: response_attempt,
                status: Some(response_status),
                response_id: response.response_id.clone(),
                body: model_response_observation(&response),
            })?;
            return Ok(response);
        }
    }
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
            if !status.is_success() {
                let body = response.text().await?;
                on_transport(ProviderTransportEvent::Response {
                    attempt,
                    status: Some(status.as_u16()),
                    response_id: None,
                    body: json!({ "error": truncate_observation_text(&body) }),
                })?;
                return Err(ResponsesRequestError { status, body }.into());
            }

            let streamed = prepared
                .body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let atomic_stream =
                streamed && prepared.response_commit == ProviderResponseCommitMode::Atomic;
            let mut provisional_deltas = Vec::new();
            let decoded = if atomic_stream {
                let mut buffer_delta = |delta| {
                    provisional_deltas.push(delta);
                    Ok(())
                };
                decode_openai_responses_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    &mut buffer_delta,
                    true,
                )
                .await
            } else {
                decode_openai_responses_response(
                    response,
                    streamed,
                    &prepared.logical_request.tool_candidates,
                    on_delta,
                    true,
                )
                .await
            };
            let (mut response, response_attempt, response_status, commit_deltas) = match decoded {
                Ok(response) => (response, attempt, status.as_u16(), provisional_deltas),
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
                Err(error) if atomic_stream => {
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
                        on_delta,
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
            for delta in commit_deltas {
                on_delta(delta)?;
            }
            on_transport(ProviderTransportEvent::Response {
                attempt: response_attempt,
                status: Some(response_status),
                response_id: response.response_id.clone(),
                body: model_response_observation(&response),
            })?;
            return Ok(response);
        }
    }
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
