//! Provider-neutral model execution boundary used by the turn runtime.
//!
//! The loop depends on [`ModelGateway`] and does not own provider selection,
//! wire encoding, authentication, retries, or response normalization.

use crate::context_runtime::CanonicalModelRequest;
use crate::provider::{
    ModelProvider, ModelRequest, ModelResponse, ModelStreamCallback, ModelStreamDelta,
    PreparedProviderRequest, ProviderResponseCommitMode, ProviderTransportCallback,
};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::Mutex;
use uuid::Uuid;

#[async_trait]
pub trait ModelGateway: Send + Sync {
    fn prepare(
        &self,
        request_id: Uuid,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest>;

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
        on_metric: &mut ModelGatewayMetricCallback<'_>,
    ) -> anyhow::Result<ModelResponse>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelGatewayMetricEvent {
    /// Emitted once when the first normalized output-bearing delta arrives,
    /// before atomic response buffers are validated and committed.
    FirstOutputTokenReceived { request_id: Uuid },
}

pub type ModelGatewayMetricCallback<'a> =
    dyn FnMut(ModelGatewayMetricEvent) -> anyhow::Result<()> + Send + 'a;

/// Pure logical-request to provider-wire encoding boundary. New adapters
/// implement this without acquiring sockets or streaming responses.
pub trait ProviderCodec: Send + Sync {
    fn encode(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest>;
}

/// Provider transport boundary. It receives an already encoded request and is
/// solely responsible for authentication, retries, I/O, and delivering
/// provider-neutral stream events and terminal responses.
#[async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn send(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse>;
}

/// Adapts the built-in provider driver to the two gateway ports. Keeping this
/// private prevents the combined driver interface from leaking back into the
/// agent runtime while providers are free to split their internals further.
#[derive(Clone)]
struct ModelProviderPorts {
    provider: Arc<dyn ModelProvider>,
}

impl ProviderCodec for ModelProviderPorts {
    fn encode(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.provider.prepare(request_id, request)
    }
}

#[async_trait]
impl ProviderTransport for ModelProviderPorts {
    async fn send(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        self.provider
            .stream_prepared(prepared, on_delta, on_transport)
            .await
    }
}

#[derive(Clone)]
pub struct ProviderModelGateway {
    codec: Arc<dyn ProviderCodec>,
    transport: Arc<dyn ProviderTransport>,
}

impl ProviderModelGateway {
    pub fn from_provider(provider: Arc<dyn ModelProvider>) -> Self {
        let ports = Arc::new(ModelProviderPorts { provider });
        Self {
            codec: ports.clone(),
            transport: ports,
        }
    }

    pub fn from_parts(
        codec: Arc<dyn ProviderCodec>,
        transport: Arc<dyn ProviderTransport>,
    ) -> Self {
        Self { codec, transport }
    }
}

#[async_trait]
impl ModelGateway for ProviderModelGateway {
    fn prepare(
        &self,
        request_id: Uuid,
        request: CanonicalModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.codec.encode(request_id, request.into_logical())
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
        on_metric: &mut ModelGatewayMetricCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let request_id = prepared.request_id;
        if prepared.response_commit == ProviderResponseCommitMode::Streaming {
            let mut first_token_received = false;
            let mut observed_delta = |delta: ModelStreamDelta| {
                if !first_token_received && delta.contains_output_token() {
                    first_token_received = true;
                    on_metric(ModelGatewayMetricEvent::FirstOutputTokenReceived { request_id })?;
                }
                on_delta(delta)
            };
            return self
                .transport
                .send(prepared, &mut observed_delta, on_transport)
                .await;
        }

        // A tool-capable model turn is one semantic transaction. Transport
        // fragments are provisional until the adapter has assembled and
        // validated the terminal response; otherwise a malformed tool call can
        // leak its accompanying text into durable conversation events before
        // the turn is rejected. Retries clear the fragments from the abandoned
        // attempt, and only the successfully decoded attempt is committed.
        let pending_deltas = Arc::new(Mutex::new(Vec::<ModelStreamDelta>::new()));
        let delta_buffer = Arc::clone(&pending_deltas);
        let mut first_token_received = false;
        let mut buffered_delta = move |delta: ModelStreamDelta| {
            if !first_token_received && delta.contains_output_token() {
                first_token_received = true;
                on_metric(ModelGatewayMetricEvent::FirstOutputTokenReceived { request_id })?;
            }
            delta_buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(delta);
            Ok(())
        };
        let retry_buffer = Arc::clone(&pending_deltas);
        let mut observed_transport = |event| {
            if matches!(event, crate::provider::ProviderTransportEvent::Retry { .. }) {
                retry_buffer
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clear();
            }
            on_transport(event)
        };
        let response = self
            .transport
            .send(prepared, &mut buffered_delta, &mut observed_transport)
            .await?;
        let deltas = std::mem::take(
            &mut *pending_deltas
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for delta in deltas {
            on_delta(delta)?;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_runtime::{ContextAssembler, ContextAssemblyInput, DefaultContextAssembler};
    use crate::model::ProviderRetryKind;
    use crate::model_context::CompiledModelContext;
    use crate::provider::{MockProvider, ModelFinishReason, ProviderTransportEvent};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn accepts_object_safe_gateway(_gateway: &dyn ModelGateway) {}

    fn canonical_request() -> CanonicalModelRequest {
        DefaultContextAssembler
            .compile(ContextAssemblyInput {
                model_context: &CompiledModelContext::default(),
                context_summary: None,
                conversation: Vec::new(),
                user_message: "hello".to_string(),
                user_content: Vec::new(),
                tool_candidates: Vec::new(),
                previous_tool_calls: Vec::new(),
                tool_results: Vec::new(),
                previous_response_items: Vec::new(),
                previous_response_id: None,
                branch_developer_instructions: None,
                prompt_cache_breakpoint_policy:
                    crate::provider::PromptCacheBreakpointPolicy::AppendOnlyUsers,
                final_output_json_schema: None,
            })
            .expect("canonical request")
    }

    #[test]
    fn provider_adapter_implements_the_object_safe_gateway_port() {
        let gateway = ProviderModelGateway::from_provider(Arc::new(MockProvider));
        accepts_object_safe_gateway(&gateway);
    }

    struct RecordingCodec {
        called: Arc<AtomicBool>,
    }

    impl ProviderCodec for RecordingCodec {
        fn encode(
            &self,
            request_id: Uuid,
            request: ModelRequest,
        ) -> anyhow::Result<PreparedProviderRequest> {
            self.called.store(true, Ordering::SeqCst);
            Ok(PreparedProviderRequest {
                request_id,
                adapter: "recording-codec".to_string(),
                method: "POST".to_string(),
                endpoint: "provider://encoded".to_string(),
                body: serde_json::json!({"encoded": request.input.current_user.message}),
                observation_body: serde_json::json!({"encoded": true}),
                cache_trace: None,
                logical_request: request,
                tool_contracts: Vec::new(),
                response_commit: ProviderResponseCommitMode::Streaming,
            })
        }
    }

    struct RecordingTransport {
        called: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ProviderTransport for RecordingTransport {
        async fn send(
            &self,
            prepared: PreparedProviderRequest,
            on_delta: &mut ModelStreamCallback<'_>,
            on_transport: &mut ProviderTransportCallback<'_>,
        ) -> anyhow::Result<ModelResponse> {
            self.called.store(true, Ordering::SeqCst);
            anyhow::ensure!(prepared.adapter == "recording-codec");
            on_delta(crate::provider::ModelStreamDelta::Reasoning {
                text: "thinking".to_string(),
            })?;
            on_transport(crate::provider::ProviderTransportEvent::Response {
                attempt: 1,
                status: Some(200),
                response_id: Some("response-1".to_string()),
                body: serde_json::json!({"status": "incomplete"}),
            })?;
            Ok(ModelResponse {
                text: String::new(),
                tool_calls: Vec::new(),
                usage: None,
                response_id: Some("response-1".to_string()),
                provider_items: Vec::new(),
                finish_reason: ModelFinishReason::Incomplete("test boundary".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn gateway_composes_distinct_codec_and_transport_ports() {
        let codec_called = Arc::new(AtomicBool::new(false));
        let transport_called = Arc::new(AtomicBool::new(false));
        let gateway = ProviderModelGateway::from_parts(
            Arc::new(RecordingCodec {
                called: Arc::clone(&codec_called),
            }),
            Arc::new(RecordingTransport {
                called: Arc::clone(&transport_called),
            }),
        );
        let canonical = canonical_request();
        let prepared = gateway
            .prepare(Uuid::from_u128(1), canonical)
            .expect("codec encodes");
        let mut reasoning = String::new();
        let response = gateway
            .stream_prepared(
                prepared,
                &mut |delta| {
                    if let crate::provider::ModelStreamDelta::Reasoning { text } = delta {
                        reasoning.push_str(&text);
                    }
                    Ok(())
                },
                &mut |_| Ok(()),
                &mut |_| Ok(()),
            )
            .await
            .expect("transport sends");

        assert!(codec_called.load(Ordering::SeqCst));
        assert!(transport_called.load(Ordering::SeqCst));
        assert_eq!(reasoning, "thinking");
        assert_eq!(
            response.finish_reason,
            ModelFinishReason::Incomplete("test boundary".to_string())
        );
    }

    struct AtomicCodec;

    impl ProviderCodec for AtomicCodec {
        fn encode(
            &self,
            request_id: Uuid,
            request: ModelRequest,
        ) -> anyhow::Result<PreparedProviderRequest> {
            Ok(PreparedProviderRequest {
                request_id,
                adapter: "atomic-test".to_string(),
                method: "POST".to_string(),
                endpoint: "provider://atomic-test".to_string(),
                body: serde_json::json!({}),
                observation_body: serde_json::json!({}),
                cache_trace: None,
                logical_request: request,
                tool_contracts: Vec::new(),
                response_commit: ProviderResponseCommitMode::Atomic,
            })
        }
    }

    struct FailingAtomicTransport;

    #[async_trait]
    impl ProviderTransport for FailingAtomicTransport {
        async fn send(
            &self,
            _prepared: PreparedProviderRequest,
            on_delta: &mut ModelStreamCallback<'_>,
            _on_transport: &mut ProviderTransportCallback<'_>,
        ) -> anyhow::Result<ModelResponse> {
            on_delta(ModelStreamDelta::Text {
                text: "provisional".to_string(),
            })?;
            anyhow::bail!("terminal tool protocol validation failed")
        }
    }

    #[tokio::test]
    async fn atomic_response_discards_deltas_when_terminal_validation_fails() {
        let gateway = ProviderModelGateway::from_parts(
            Arc::new(AtomicCodec),
            Arc::new(FailingAtomicTransport),
        );
        let prepared = gateway
            .prepare(Uuid::from_u128(2), canonical_request())
            .unwrap();
        let mut committed = Vec::new();
        let mut metrics = Vec::new();

        gateway
            .stream_prepared(
                prepared,
                &mut |delta| {
                    committed.push(delta);
                    Ok(())
                },
                &mut |_| Ok(()),
                &mut |metric| {
                    metrics.push(metric);
                    Ok(())
                },
            )
            .await
            .expect_err("invalid atomic response must fail");

        assert!(committed.is_empty());
        assert_eq!(
            metrics,
            vec![ModelGatewayMetricEvent::FirstOutputTokenReceived {
                request_id: Uuid::from_u128(2)
            }]
        );
    }

    struct RetryingAtomicTransport;

    #[async_trait]
    impl ProviderTransport for RetryingAtomicTransport {
        async fn send(
            &self,
            _prepared: PreparedProviderRequest,
            on_delta: &mut ModelStreamCallback<'_>,
            on_transport: &mut ProviderTransportCallback<'_>,
        ) -> anyhow::Result<ModelResponse> {
            on_delta(ModelStreamDelta::Text {
                text: "discarded-attempt".to_string(),
            })?;
            on_transport(ProviderTransportEvent::Retry {
                attempt: 2,
                retry_kind: ProviderRetryKind::StateRecovery,
                retry_index: None,
                retry_limit: None,
                reason: "retry transaction".to_string(),
                cache_trace: None,
                body: serde_json::json!({}),
            })?;
            on_delta(ModelStreamDelta::Text {
                text: "committed-attempt".to_string(),
            })?;
            Ok(ModelResponse {
                text: "committed-attempt".to_string(),
                tool_calls: Vec::new(),
                usage: None,
                response_id: Some("response-2".to_string()),
                provider_items: Vec::new(),
                finish_reason: ModelFinishReason::Stop,
            })
        }
    }

    #[tokio::test]
    async fn atomic_response_clears_abandoned_attempt_before_commit() {
        let gateway = ProviderModelGateway::from_parts(
            Arc::new(AtomicCodec),
            Arc::new(RetryingAtomicTransport),
        );
        let prepared = gateway
            .prepare(Uuid::from_u128(3), canonical_request())
            .unwrap();
        let mut committed = Vec::new();

        gateway
            .stream_prepared(
                prepared,
                &mut |delta| {
                    committed.push(delta);
                    Ok(())
                },
                &mut |_| Ok(()),
                &mut |_| Ok(()),
            )
            .await
            .unwrap();

        assert_eq!(
            committed,
            vec![ModelStreamDelta::Text {
                text: "committed-attempt".to_string()
            }]
        );
    }
}
