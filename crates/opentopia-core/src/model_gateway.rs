//! Provider-neutral model execution boundary used by the turn runtime.
//!
//! Provider selection and construction remain compatibility concerns while the
//! legacy `AgentCore` is migrated. The loop itself depends on [`ModelGateway`]
//! and no longer owns transport selection directly.

use crate::context_runtime::CanonicalModelRequest;
use crate::provider::{
    ModelProvider, ModelRequest, ModelResponse, ModelStreamCallback, ModelStreamDelta,
    PreparedProviderRequest, ProviderTransportCallback,
};
use async_trait::async_trait;
use std::sync::Arc;
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
    ) -> anyhow::Result<ModelResponse>;
}

/// Pure logical-request to provider-wire encoding boundary. New adapters
/// implement this without acquiring sockets or streaming responses.
pub trait ProviderCodec: Send + Sync {
    fn encode(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest>;

    /// Normalize one provider stream event into the canonical delta protocol.
    /// Legacy codecs are identity adapters because their old driver already
    /// performed wire parsing before yielding the event.
    fn decode_delta(&self, delta: ModelStreamDelta) -> anyhow::Result<ModelStreamDelta> {
        Ok(delta)
    }

    /// Normalize finish reason, usage, tool calls, and provider state after the
    /// transport reaches a terminal response.
    fn decode_response(&self, response: ModelResponse) -> anyhow::Result<ModelResponse> {
        Ok(response)
    }
}

/// Provider transport boundary. It receives an already encoded request and is
/// solely responsible for retries, I/O, and delivering normalized stream
/// events. Legacy drivers retain their parsing internally behind this facade.
#[async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn send(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse>;
}

#[derive(Clone)]
pub struct LegacyProviderCodec {
    provider: Arc<dyn ModelProvider>,
}

impl LegacyProviderCodec {
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        Self { provider }
    }
}

impl ProviderCodec for LegacyProviderCodec {
    fn encode(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        self.provider.prepare(request_id, request)
    }
}

#[derive(Clone)]
pub struct LegacyProviderTransport {
    provider: Arc<dyn ModelProvider>,
}

impl LegacyProviderTransport {
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl ProviderTransport for LegacyProviderTransport {
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

/// Compatibility adapter that keeps provider construction outside the model
/// execution port while preserving every existing codec and transport behavior.
#[derive(Clone)]
pub struct ProviderModelGateway {
    codec: Arc<dyn ProviderCodec>,
    transport: Arc<dyn ProviderTransport>,
}

impl ProviderModelGateway {
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        Self {
            codec: Arc::new(LegacyProviderCodec::new(Arc::clone(&provider))),
            transport: Arc::new(LegacyProviderTransport::new(provider)),
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
    ) -> anyhow::Result<ModelResponse> {
        let codec = Arc::clone(&self.codec);
        let mut decoded_delta = |delta| on_delta(codec.decode_delta(delta)?);
        let response = self
            .transport
            .send(prepared, &mut decoded_delta, on_transport)
            .await?;
        self.codec.decode_response(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_runtime::{ContextAssembler, ContextAssemblyInput, DefaultContextAssembler};
    use crate::model_context::CompiledModelContext;
    use crate::provider::{MockProvider, ModelFinishReason};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn accepts_object_safe_gateway(_gateway: &dyn ModelGateway) {}

    #[test]
    fn provider_adapter_implements_the_object_safe_gateway_port() {
        let gateway = ProviderModelGateway::new(Arc::new(MockProvider));
        accepts_object_safe_gateway(&gateway);
    }

    struct RecordingCodec {
        called: Arc<AtomicBool>,
        decoded_delta: Arc<AtomicBool>,
        decoded_response: Arc<AtomicBool>,
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
                logical_request: request,
            })
        }

        fn decode_delta(&self, delta: ModelStreamDelta) -> anyhow::Result<ModelStreamDelta> {
            self.decoded_delta.store(true, Ordering::SeqCst);
            Ok(delta)
        }

        fn decode_response(&self, response: ModelResponse) -> anyhow::Result<ModelResponse> {
            self.decoded_response.store(true, Ordering::SeqCst);
            Ok(response)
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
        let delta_decoded = Arc::new(AtomicBool::new(false));
        let response_decoded = Arc::new(AtomicBool::new(false));
        let transport_called = Arc::new(AtomicBool::new(false));
        let gateway = ProviderModelGateway::from_parts(
            Arc::new(RecordingCodec {
                called: Arc::clone(&codec_called),
                decoded_delta: Arc::clone(&delta_decoded),
                decoded_response: Arc::clone(&response_decoded),
            }),
            Arc::new(RecordingTransport {
                called: Arc::clone(&transport_called),
            }),
        );
        let canonical = DefaultContextAssembler
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
            .expect("canonical request");
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
            )
            .await
            .expect("transport sends");

        assert!(codec_called.load(Ordering::SeqCst));
        assert!(delta_decoded.load(Ordering::SeqCst));
        assert!(response_decoded.load(Ordering::SeqCst));
        assert!(transport_called.load(Ordering::SeqCst));
        assert_eq!(reasoning, "thinking");
        assert_eq!(
            response.finish_reason,
            ModelFinishReason::Incomplete("test boundary".to_string())
        );
    }
}
