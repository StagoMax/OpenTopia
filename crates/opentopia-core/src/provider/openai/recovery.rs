use super::super::{
    apply_provider_auth, redact_transport_value, truncate_observation_text, ModelResponse,
    ModelStreamCallback, PreparedProviderRequest, ProviderAuthKind, ProviderTransportCallback,
    ProviderTransportEvent,
};
use super::{
    decode_openai_chat_response, decode_openai_responses_response,
    tool_call_protocol_error_observation,
};
use crate::model::ProviderRetryKind;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};

use crate::provider::transport::{
    provider_rate_limit_retry_delay, send_provider_request_with_network_retries,
    PROVIDER_RATE_LIMIT_RETRY_LIMIT,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum OpenAiRecoveryProtocol {
    ChatCompletions,
    Responses,
}

pub(super) struct RecoveredToolResponse {
    pub(super) response: ModelResponse,
    pub(super) attempt: usize,
    pub(super) status: u16,
}

/// Provider-declared rate limits inside an HTTP-200 SSE stream are transport
/// failures, not malformed tool calls. Atomic tool streams have not committed
/// deltas or executed tools yet, so retrying the original streaming request is
/// safe and preserves the negotiated wire protocol.
pub(super) async fn schedule_rate_limited_stream_retry(
    prepared: &PreparedProviderRequest,
    failed_attempt: usize,
    failed_status: u16,
    retry_index: usize,
    retry_after: Option<std::time::Duration>,
    stream_error: &anyhow::Error,
    on_transport: &mut ProviderTransportCallback<'_>,
) -> anyhow::Result<Option<usize>> {
    let exhausted = retry_index >= PROVIDER_RATE_LIMIT_RETRY_LIMIT;
    on_transport(ProviderTransportEvent::Response {
        attempt: failed_attempt,
        status: Some(failed_status),
        response_id: None,
        body: json!({
            "error": truncate_observation_text(&stream_error.to_string()),
            "classification": "rate_limit",
            "recovery": if exhausted {
                "retry_streaming_rate_limit_exhausted"
            } else {
                "retry_streaming_rate_limit"
            }
        }),
    })?;
    if exhausted {
        return Ok(None);
    }

    let next_retry_index = retry_index.saturating_add(1);
    let next_attempt = failed_attempt.saturating_add(1);
    let delay = provider_rate_limit_retry_delay(next_retry_index, retry_after);
    on_transport(ProviderTransportEvent::Retry {
        attempt: next_attempt,
        retry_kind: ProviderRetryKind::Network,
        retry_index: Some(next_retry_index),
        retry_limit: Some(PROVIDER_RATE_LIMIT_RETRY_LIMIT),
        reason: format!(
            "provider rate limited the streamed response; retrying the original streaming request after {} second(s): {}",
            delay.as_secs_f64(),
            truncate_observation_text(&stream_error.to_string())
        ),
        cache_trace: prepared.cache_trace.clone(),
        body: prepared.observation_body.clone(),
    })?;
    tokio::time::sleep(delay).await;
    Ok(Some(next_attempt))
}

/// Tool-bearing streamed responses are atomic: no tool has run and no partial
/// response has been committed when its stream cannot be decoded or validated.
/// Retrying the same logical request once without streaming is therefore a safe
/// transport-level recovery and avoids asking the model to regenerate a
/// different action.
pub(super) async fn recover_streamed_tool_call_non_streaming(
    protocol: OpenAiRecoveryProtocol,
    client: &reqwest::Client,
    auth: ProviderAuthKind,
    api_key: &str,
    prepared: &PreparedProviderRequest,
    failed_attempt: usize,
    failed_status: u16,
    stream_error: &anyhow::Error,
    on_delta: &mut ModelStreamCallback<'_>,
    on_transport: &mut ProviderTransportCallback<'_>,
) -> anyhow::Result<RecoveredToolResponse> {
    on_transport(ProviderTransportEvent::Response {
        attempt: failed_attempt,
        status: Some(failed_status),
        response_id: None,
        body: tool_call_protocol_error_observation(stream_error, Some("retry_non_streaming_once")),
    })?;

    let mut body = prepared.body.clone();
    body["stream"] = Value::Bool(false);
    if let Some(object) = body.as_object_mut() {
        object.remove("stream_options");
    }
    let observation_body = redact_transport_value(&body);
    let cache_trace = crate::build_provider_cache_trace(&body, None, false);
    on_transport(ProviderTransportEvent::Retry {
        attempt: failed_attempt.saturating_add(1),
        retry_kind: ProviderRetryKind::StateRecovery,
        retry_index: Some(1),
        retry_limit: Some(1),
        reason: format!(
            "atomic tool-call stream was invalid; retrying the same logical request once without streaming: {}",
            truncate_observation_text(&stream_error.to_string())
        ),
        cache_trace: cache_trace.clone(),
        body: observation_body.clone(),
    })?;

    let (response, attempt) = send_provider_request_with_network_retries(
        || {
            apply_provider_auth(client.post(&prepared.endpoint), auth, api_key)
                .header(CONTENT_TYPE, "application/json")
                .json(&body)
        },
        failed_attempt.saturating_add(1),
        &observation_body,
        cache_trace.as_ref(),
        on_transport,
    )
    .await?;
    let status = response.status();
    if !status.is_success() {
        let response_body = response.text().await?;
        on_transport(ProviderTransportEvent::Response {
            attempt,
            status: Some(status.as_u16()),
            response_id: None,
            body: json!({
                "error": truncate_observation_text(&response_body),
                "recovery": "retry_non_streaming_once_failed"
            }),
        })?;
        anyhow::bail!(
            "provider non-streaming tool-call recovery failed ({status}) after streamed protocol error: {response_body}"
        );
    }

    let decoded = match protocol {
        OpenAiRecoveryProtocol::ChatCompletions => {
            decode_openai_chat_response(
                response,
                false,
                &prepared.logical_request.tool_candidates,
                on_delta,
                true,
            )
            .await
        }
        OpenAiRecoveryProtocol::Responses => {
            decode_openai_responses_response(
                response,
                false,
                &prepared.logical_request.tool_candidates,
                on_delta,
                true,
            )
            .await
        }
    };
    let response = match decoded {
        Ok(response) => response,
        Err(recovery_error) => {
            let error = anyhow::anyhow!(
                "provider tool-call protocol error: both streamed decoding and one non-streaming recovery failed; streamed error: {stream_error}; non-streaming error: {recovery_error}"
            );
            on_transport(ProviderTransportEvent::Response {
                attempt,
                status: Some(status.as_u16()),
                response_id: None,
                body: tool_call_protocol_error_observation(
                    &error,
                    Some("retry_non_streaming_once_failed"),
                ),
            })?;
            return Err(error);
        }
    };

    Ok(RecoveredToolResponse {
        response,
        attempt,
        status: status.as_u16(),
    })
}
