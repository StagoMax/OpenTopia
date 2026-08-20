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

use crate::provider::transport::send_provider_request_with_network_retries;

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
    on_transport(ProviderTransportEvent::Retry {
        attempt: failed_attempt.saturating_add(1),
        retry_kind: ProviderRetryKind::StateRecovery,
        retry_index: Some(1),
        retry_limit: Some(1),
        reason: format!(
            "atomic tool-call stream was invalid; retrying the same logical request once without streaming: {}",
            truncate_observation_text(&stream_error.to_string())
        ),
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
