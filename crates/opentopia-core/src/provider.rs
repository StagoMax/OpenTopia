use crate::model::{ModelContentPart, ProviderRetryKind};
#[cfg(test)]
use crate::model_context::{estimate_tokens, ContextItemKind, ModelContextItem};
use crate::model_context::{CompiledModelContext, ContextCacheScope, ContextRole};
#[cfg(test)]
use crate::settings::ProviderKind;
use crate::settings::{
    OpenAiCompatibilityReport, ProviderAdapterKind, ProviderAdapterProfile, ProviderAuthKind,
    ProviderFeatureSupport, ProviderHealthCheck, ProviderInstructionEncoding,
    ProviderMessageProtocolCapabilities, ProviderOutputProtocolCapabilities,
    ProviderReasoningProtocol, ProviderSettings, ProviderToolProtocolCapabilities,
    ProviderTransportKind, PROVIDER_ADAPTER_PROFILE_VERSION,
};
use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::AUTHORIZATION;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

mod openai;
mod token_estimate;
mod transport;

pub use openai::{OpenAiCompatibleProvider, OpenAiResponsesProvider};
pub use token_estimate::estimate_provider_tool_surface_tokens;
#[cfg(test)]
use token_estimate::{estimate_provider_tool_results, estimate_serialized_slice};

use openai::{
    chat_finish_reason, compatibility_probe_candidate, emit_response_deltas,
    is_tool_call_protocol_error, legacy_tool_observation, model_response_observation,
    nonredundant_tool_result_content, parse_model_usage, parse_required_tool_arguments,
    provider_rejected_strict_function_tools, provider_rejected_tool_representation,
    provider_tool_result_content, request_uses_strict_function_tools,
    responses_request_uses_enhanced_tools, tool_call_protocol_error_observation,
    validate_provider_response_protocol, validate_tool_probe_response, OpenAiProbeOutcome,
    StreamingToolCall, NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION,
};
pub(crate) use openai::{
    invalid_tool_arguments_json_details, normalize_tool_argument_keys, tool_input_schema_error,
};

#[cfg(test)]
use openai::{
    compile_openai_tools, extract_provider_tool_calls, extract_response_text,
    normalize_provider_arguments, openai_messages, openai_messages_with_reasoning,
    openai_portable_messages, openai_portable_messages_with_reasoning,
    openai_strict_function_schema, openai_tools, parse_model_response_body,
    parse_model_response_body_with_tools, responses_input, responses_tool_result_output,
    responses_tools, OpenAiStreamAccumulator, ResponsesStreamAccumulator,
    INVALID_TOOL_ARGUMENTS_JSON_KEY, OPENAI_CHAT_ASSISTANT_STATE_TYPE,
    OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT, OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT,
};

use transport::{
    app_server_idle_timeout, next_stream_chunk, send_provider_request_with_network_retries,
    stream_idle_timeout, SseDecoder, PROVIDER_NETWORK_RETRY_LIMIT,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelConversationRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Typed input content shared by user/history messages and tool results.
///
/// This alias leaves the model-layer representation as the single source of
/// truth while making the provider-facing API discoverable.
pub type ModelInputContent = ModelContentPart;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelConversationMessage {
    pub role: ModelConversationRole,
    /// Legacy text content. Non-empty `content_parts` are appended and sent as
    /// native content parts where the selected provider supports them.
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_parts: Vec<ModelInputContent>,
    /// Provider-neutral structured assistant tool calls. Keeping these in the
    /// durable conversation prevents cross-turn replay from degrading an
    /// assistant/tool exchange into synthetic user text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ProviderToolCall>,
    /// Provider-neutral structured tool results paired by `call_id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ProviderToolResult>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheBreakpointPolicy {
    /// One-shot/auxiliary calls may cache only the immutable instruction prefix.
    #[default]
    StableOnly,
    /// Multi-turn agent calls also anchor every real user message as the
    /// append-only ledger grows.
    AppendOnlyUsers,
}

/// Current user input carried by the append-only model ledger.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUserInput {
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ModelInputContent>,
}

/// Provider-neutral, typed request ledger. Conversation, the current user
/// input, and the active turn's completed tool exchange as of this request
/// have exactly one owner. On round N, calls/results from earlier rounds in the
/// same turn are cumulative; they are not the entire thread history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInputLedger {
    #[serde(default)]
    pub conversation: Vec<ModelConversationMessage>,
    pub current_user: ModelUserInput,
    #[serde(default)]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default)]
    pub tool_results: Vec<ProviderToolResult>,
}

/// Exact provider-visible transcript retained at a successful turn boundary.
///
/// This is deliberately separate from the provider-neutral conversation
/// projection. The projection remains the durable recovery source, while this
/// value preserves byte ordering for adapters whose prompt cache requires the
/// next request to extend the previous wire transcript without rebuilding it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderWireTranscript {
    pub format: String,
    pub items: Vec<Value>,
}

pub(crate) const PROVIDER_TRANSCRIPT_STATE_TYPE: &str = "opentopia_provider_wire_transcript";
pub(crate) const PROVIDER_TRANSCRIPT_CANDIDATE_TYPE: &str =
    "opentopia_provider_wire_transcript_candidate";

pub fn provider_transcript_state_item(transcript: &ProviderWireTranscript) -> Value {
    json!({
        "type": PROVIDER_TRANSCRIPT_STATE_TYPE,
        "format": &transcript.format,
        "items": &transcript.items,
    })
}

pub(crate) fn provider_transcript_candidate_item(transcript: &ProviderWireTranscript) -> Value {
    json!({
        "type": PROVIDER_TRANSCRIPT_CANDIDATE_TYPE,
        "format": &transcript.format,
        "items": &transcript.items,
    })
}

pub(crate) fn provider_wire_transcript(item: &Value) -> Option<ProviderWireTranscript> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    if !matches!(
        item_type,
        PROVIDER_TRANSCRIPT_STATE_TYPE | PROVIDER_TRANSCRIPT_CANDIDATE_TYPE
    ) {
        return None;
    }
    Some(ProviderWireTranscript {
        format: item.get("format")?.as_str()?.to_string(),
        items: item.get("items")?.as_array()?.clone(),
    })
}

pub(crate) fn provider_item_is_internal_transcript(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some(PROVIDER_TRANSCRIPT_STATE_TYPE | PROVIDER_TRANSCRIPT_CANDIDATE_TYPE)
    )
}

pub fn split_provider_transcript_state(
    items: Vec<Value>,
) -> (Option<ProviderWireTranscript>, Vec<Value>) {
    let transcript = items
        .iter()
        .rev()
        .find(|item| {
            item.get("type").and_then(Value::as_str) == Some(PROVIDER_TRANSCRIPT_STATE_TYPE)
        })
        .and_then(provider_wire_transcript);
    let provider_items = items
        .into_iter()
        .filter(|item| !provider_item_is_internal_transcript(item))
        .collect();
    (transcript, provider_items)
}

/// Canonical logical shape consumed by provider codecs.
///
/// `instructions` contains only classified instruction/context modules. Typed
/// history lives once in `input`; providers are adapters over this shape and
/// may not assemble a second prompt representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRequest {
    #[serde(default)]
    pub instructions: CompiledModelContext,
    #[serde(default)]
    pub input: ModelInputLedger,
    /// Provider-native tool surface. Adapters map these candidates to `tools`
    /// or `dynamicTools`; they must not serialize schemas into prompt text.
    #[serde(default)]
    pub tool_candidates: Vec<ProviderToolCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_response_items: Vec<Value>,
    /// Internal wire-cache continuation extracted from provider state. It is
    /// intentionally omitted from request/event serialization: the exact
    /// transcript can be large and is already persisted once in the provider
    /// cursor. Provider codecs consume it directly in memory.
    #[serde(skip)]
    pub provider_transcript: Option<ProviderWireTranscript>,
    /// Continue a stored Responses API chain. The logical request still carries
    /// the complete replay context so the adapter can recover if this cursor is
    /// unknown or expired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub prompt_cache_breakpoint_policy: PromptCacheBreakpointPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_output_json_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelResponse {
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ProviderToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ModelUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_items: Vec<Value>,
    #[serde(default)]
    pub finish_reason: ModelFinishReason,
}

impl ModelResponse {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Vec::new(),
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::Stop,
        }
    }

    pub fn decision(&self) -> ModelDecision {
        if let Some(reason) = self.finish_reason.incomplete_reason() {
            return ModelDecision::Incomplete(reason);
        }
        if !self.tool_calls.is_empty() {
            return ModelDecision::Act(self.tool_calls.clone());
        }
        if self.finish_reason == ModelFinishReason::ToolCalls {
            return ModelDecision::Incomplete(IncompleteReason::ProviderProtocol(
                "provider reported tool_calls but returned no tool call".to_string(),
            ));
        }
        let text = self.text.trim();
        if text.is_empty() {
            ModelDecision::Incomplete(IncompleteReason::EmptyResponse)
        } else {
            ModelDecision::Final(text.to_string())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "reason", rename_all = "snake_case")]
pub enum ModelFinishReason {
    Stop,
    ToolCalls,
    Completed,
    Length,
    ContentFilter,
    Incomplete(String),
    StreamInterrupted,
}

impl Default for ModelFinishReason {
    fn default() -> Self {
        Self::StreamInterrupted
    }
}

impl ModelFinishReason {
    fn incomplete_reason(&self) -> Option<IncompleteReason> {
        match self {
            Self::Stop | Self::ToolCalls | Self::Completed => None,
            Self::Length => Some(IncompleteReason::OutputTokenLimit),
            Self::ContentFilter => Some(IncompleteReason::ContentFilter),
            Self::Incomplete(reason) => Some(IncompleteReason::Provider(reason.clone())),
            Self::StreamInterrupted => Some(IncompleteReason::StreamInterrupted),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum IncompleteReason {
    OutputTokenLimit,
    ContentFilter,
    EmptyResponse,
    StreamInterrupted,
    Provider(String),
    ProviderProtocol(String),
}

impl std::fmt::Display for IncompleteReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutputTokenLimit => formatter.write_str("output token limit reached"),
            Self::ContentFilter => formatter.write_str("response stopped by content filter"),
            Self::EmptyResponse => {
                formatter.write_str("provider returned an empty assistant response")
            }
            Self::StreamInterrupted => {
                formatter.write_str("provider stream ended before a terminal event")
            }
            Self::Provider(reason) => write!(
                formatter,
                "provider reported an incomplete response: {reason}"
            ),
            Self::ProviderProtocol(reason) => {
                write!(formatter, "provider completion protocol error: {reason}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "decision", content = "value", rename_all = "snake_case")]
pub enum ModelDecision {
    Act(Vec<ProviderToolCall>),
    Final(String),
    Incomplete(IncompleteReason),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ModelStreamDelta {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolCall {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Usage {
        usage: ModelUsage,
    },
}

impl ModelStreamDelta {
    pub fn contains_output_token(&self) -> bool {
        match self {
            Self::Text { text } | Self::Reasoning { text } => !text.is_empty(),
            Self::ToolCall {
                id,
                name,
                arguments_delta,
                ..
            } => {
                id.as_deref().is_some_and(|value| !value.is_empty())
                    || name.as_deref().is_some_and(|value| !value.is_empty())
                    || !arguments_delta.is_empty()
            }
            Self::Usage { .. } => false,
        }
    }
}

/// Responses assistant-message phase values. The stream adapter uses these to
/// prevent commentary preambles from being promoted into the final answer,
/// while retaining the original provider item (including `phase`) for replay.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantOutputPhase {
    Commentary,
    FinalAnswer,
}

impl AssistantOutputPhase {
    fn from_wire(value: &str) -> Option<Self> {
        match value {
            "commentary" => Some(Self::Commentary),
            "final_answer" => Some(Self::FinalAnswer),
            _ => None,
        }
    }
}

pub type ModelStreamCallback<'a> = dyn FnMut(ModelStreamDelta) -> anyhow::Result<()> + Send + 'a;

/// Controls when normalized response deltas become externally observable.
///
/// Tool-bearing responses are semantic transactions: their streamed fragments
/// remain provisional until the provider adapter has assembled and validated
/// the terminal response. This is declared by the adapter because hosted tools
/// may exist on the wire without a corresponding logical tool candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProviderResponseCommitMode {
    #[default]
    Streaming,
    Atomic,
}
#[derive(Debug, Clone)]
pub struct PreparedProviderRequest {
    pub request_id: Uuid,
    pub adapter: String,
    pub method: String,
    pub endpoint: String,
    pub body: Value,
    pub observation_body: Value,
    /// Content-free fingerprints computed from the exact, unredacted wire
    /// request before it crosses the provider boundary.
    pub cache_trace: Option<crate::model::ProviderCacheTrace>,
    pub logical_request: ModelRequest,
    /// Exact ordered input emitted by an adapter that supports transcript
    /// continuation. The transport attaches the successful assistant output
    /// before handing the resulting cursor candidate back to the runtime.
    pub wire_transcript: Option<ProviderWireTranscript>,
    /// Exact function-tool contracts compiled for this provider request. The
    /// response decoder uses the same artifacts that produced the advertised
    /// schemas to restore provider wire arguments to the canonical tool shape.
    pub tool_contracts: Vec<CompiledToolContract>,
    pub response_commit: ProviderResponseCommitMode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledToolContract {
    pub name: String,
    /// Schema before provider-specific strict lowering. This is normally the
    /// tool's canonical schema; representation adapters such as portable
    /// apply_patch may intentionally expose a canonical subset.
    pub logical_input_schema: Value,
    /// The exact schema serialized into this request.
    pub wire_input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderAdapterError {
    #[error(
        "provider adapter capability profile is stale for {capability}; test this connection/model again: {detail}"
    )]
    CapabilityProfileStale {
        capability: &'static str,
        detail: String,
    },
    #[error("provider adapter does not support required capability {capability}: {detail}")]
    CapabilityUnavailable {
        capability: &'static str,
        detail: String,
    },
}

fn require_function_tools(
    adapter: &'static str,
    capabilities: ProviderToolProtocolCapabilities,
) -> anyhow::Result<()> {
    if capabilities.function_tools == ProviderFeatureSupport::Supported {
        return Ok(());
    }
    Err(ProviderAdapterError::CapabilityUnavailable {
        capability: "function_tools",
        detail: format!(
            "{adapter} capability is {:?}; run connection negotiation or choose an adapter that passed the function-tool round trip",
            capabilities.function_tools
        ),
    }
    .into())
}

#[derive(Debug, Clone)]
pub enum ProviderTransportEvent {
    Retry {
        attempt: usize,
        retry_kind: ProviderRetryKind,
        retry_index: Option<usize>,
        retry_limit: Option<usize>,
        reason: String,
        cache_trace: Option<crate::model::ProviderCacheTrace>,
        body: Value,
    },
    Response {
        attempt: usize,
        status: Option<u16>,
        response_id: Option<String>,
        body: Value,
    },
}

pub type ProviderTransportCallback<'a> =
    dyn FnMut(ProviderTransportEvent) -> anyhow::Result<()> + Send + 'a;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolCandidate {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub disclosure: ProviderToolDisclosure,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<ProviderToolNamespace>,
}

impl ProviderToolCandidate {
    pub fn direct(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            disclosure: ProviderToolDisclosure::Direct,
            namespace: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolDisclosure {
    #[default]
    Direct,
    DeferredIndividual,
    DeferredNamespace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolNamespace {
    pub name: String,
    pub description: String,
}

/// Provider-facing representations are selected from negotiated protocol
/// capabilities, not vendor or model-name tables. Function is the portable
/// fallback; Freeform and Hosted are optional fast paths.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolRepresentation {
    Function,
    Freeform,
    Hosted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "representation", rename_all = "snake_case")]
pub enum ProviderToolDefinition {
    Function {
        name: String,
        description: String,
        input_schema: Value,
        strict: bool,
    },
    Freeform {
        name: String,
        description: String,
    },
    Hosted {
        kind: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolCall {
    pub id: String,
    pub name: String,
    /// Canonical tool arguments. Provider adapters must decode any
    /// provider-specific strict-schema representation before returning a call.
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderToolResult {
    pub call_id: String,
    pub name: String,
    /// Legacy text output. `content` preserves structured and multimodal tool
    /// output for provider adapters and persisted events.
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ModelInputContent>,
    pub is_error: bool,
    pub metadata: Value,
}

impl ProviderToolResult {
    pub fn content_or_legacy_text(&self) -> Vec<ModelInputContent> {
        if self.content.is_empty() {
            vec![ModelInputContent::text(self.output.clone())]
        } else {
            self.content.clone()
        }
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse>;

    fn prepare(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        let response_commit = if request.tool_candidates.is_empty() {
            ProviderResponseCommitMode::Streaming
        } else {
            ProviderResponseCommitMode::Atomic
        };
        let body = serde_json::to_value(&request)?;
        Ok(PreparedProviderRequest {
            request_id,
            adapter: "logical".to_string(),
            method: "MODEL".to_string(),
            endpoint: "provider://logical".to_string(),
            observation_body: redact_transport_value(&body),
            cache_trace: crate::build_provider_cache_trace(&body, None, false),
            body,
            logical_request: request,
            wire_transcript: None,
            tool_contracts: Vec::new(),
            response_commit,
        })
    }

    async fn stream_prepared(
        &self,
        prepared: PreparedProviderRequest,
        on_delta: &mut ModelStreamCallback<'_>,
        on_transport: &mut ProviderTransportCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self.stream(prepared.logical_request, on_delta).await?;
        on_transport(ProviderTransportEvent::Response {
            attempt: 1,
            status: None,
            response_id: response.response_id.clone(),
            body: model_response_observation(&response),
        })?;
        Ok(response)
    }

    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut ModelStreamCallback<'_>,
    ) -> anyhow::Result<ModelResponse> {
        let response = self.complete(request).await?;
        if !response.text.is_empty() {
            on_delta(ModelStreamDelta::Text {
                text: response.text.clone(),
            })?;
        }
        for (index, call) in response.tool_calls.iter().enumerate() {
            on_delta(ModelStreamDelta::ToolCall {
                index,
                id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                arguments_delta: call.arguments.to_string(),
            })?;
        }
        if let Some(usage) = &response.usage {
            on_delta(ModelStreamDelta::Usage {
                usage: usage.clone(),
            })?;
        }
        Ok(response)
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDriverTrust {
    BuiltIn,
    Signed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDriverDescriptor {
    pub id: String,
    pub adapter: ProviderAdapterKind,
    pub transport: ProviderTransportKind,
    pub display_name: String,
    pub trust: ProviderDriverTrust,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderDriverUse {
    Model,
    Guardian,
}

type ProviderDriverFactory =
    fn(&ProviderSettings, ProviderDriverUse) -> Option<Arc<dyn ModelProvider>>;

#[async_trait]
trait ProviderCapabilityNegotiator: Send + Sync {
    async fn negotiate(
        &self,
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderNegotiationResult>;
}

#[derive(Debug, Clone, Copy)]
struct OpenAiCapabilityNegotiator;

#[async_trait]
impl ProviderCapabilityNegotiator for OpenAiCapabilityNegotiator {
    async fn negotiate(
        &self,
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderNegotiationResult> {
        let health = OpenAiCompatibleProvider::probe_settings(settings).await?;
        let adapter_profiles = health
            .openai_compatibility
            .as_ref()
            .filter(|_| health.reachable && health.model_available)
            .map(OpenAiCompatibilityReport::adapter_profiles)
            .unwrap_or_default();
        Ok(ProviderNegotiationResult {
            health,
            adapter_profiles,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct AnthropicCapabilityNegotiator;

#[async_trait]
impl ProviderCapabilityNegotiator for AnthropicCapabilityNegotiator {
    async fn negotiate(
        &self,
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderNegotiationResult> {
        let provider = AnthropicMessagesProvider::from_settings(settings)
            .context("Anthropic Messages provider is not configured")?;
        let (portable_tools, streaming_tools) = provider.probe_tool_capabilities().await;
        let reachable = portable_tools.support == ProviderFeatureSupport::Supported
            || streaming_tools.support == ProviderFeatureSupport::Supported;
        let model_available = portable_tools.support == ProviderFeatureSupport::Supported;
        let error = (!model_available).then(|| {
            portable_tools
                .detail
                .clone()
                .unwrap_or_else(|| "Anthropic function-tool round trip failed".to_string())
        });
        let health = ProviderHealthCheck {
            reachable,
            latency_ms: None,
            model_available,
            error,
            openai_compatibility: None,
        };
        let adapter_profile = model_available.then(|| ProviderAdapterProfile {
            profile_version: PROVIDER_ADAPTER_PROFILE_VERSION,
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            adapter: ProviderAdapterKind::AnthropicMessages,
            instruction_encoding: ProviderInstructionEncoding::FoldDeveloperIntoSystem,
            reasoning_protocol: ProviderReasoningProtocol::Omit,
            message_protocol: ProviderMessageProtocolCapabilities::default(),
            output_protocol: ProviderOutputProtocolCapabilities::default(),
            tool_protocol: ProviderToolProtocolCapabilities {
                function_tools: portable_tools.support,
                streaming_tools: streaming_tools.support,
                ..ProviderToolProtocolCapabilities::default()
            },
            checked_at: Utc::now(),
        });
        Ok(ProviderNegotiationResult {
            health,
            adapter_profiles: adapter_profile.into_iter().collect(),
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct StaticCapabilityNegotiator {
    adapter: ProviderAdapterKind,
    instruction_encoding: ProviderInstructionEncoding,
    tool_protocol: ProviderToolProtocolCapabilities,
}

#[async_trait]
impl ProviderCapabilityNegotiator for StaticCapabilityNegotiator {
    async fn negotiate(
        &self,
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderNegotiationResult> {
        let provider = configured_provider_from_settings_with_adapter(settings, self.adapter)
            .context("provider is not configured")?;
        let health = provider.check_health().await?;
        let adapter_profile =
            (health.reachable && health.model_available).then(|| ProviderAdapterProfile {
                profile_version: PROVIDER_ADAPTER_PROFILE_VERSION,
                base_url: settings.base_url.clone(),
                model: settings.model.clone(),
                adapter: self.adapter,
                instruction_encoding: self.instruction_encoding,
                reasoning_protocol: ProviderReasoningProtocol::Omit,
                message_protocol: ProviderMessageProtocolCapabilities::default(),
                output_protocol: ProviderOutputProtocolCapabilities::default(),
                tool_protocol: self.tool_protocol,
                checked_at: Utc::now(),
            });
        Ok(ProviderNegotiationResult {
            health,
            adapter_profiles: adapter_profile.into_iter().collect(),
        })
    }
}

#[derive(Clone)]
struct ProviderDriverRegistration {
    descriptor: ProviderDriverDescriptor,
    factory: ProviderDriverFactory,
    negotiator: Arc<dyn ProviderCapabilityNegotiator>,
}

/// Host-owned registry for model transports. Registrations are intentionally
/// private to this crate: ordinary plugin discovery cannot add provider drivers
/// or promote a manifest into a trusted transport implementation.
#[derive(Clone)]
pub struct ProviderDriverRegistry {
    drivers: BTreeMap<String, ProviderDriverRegistration>,
}

impl Default for ProviderDriverRegistry {
    fn default() -> Self {
        let mut registry = Self {
            drivers: BTreeMap::new(),
        };
        registry.register_builtin(
            ProviderAdapterKind::Mock,
            ProviderTransportKind::Mock,
            "Mock",
            |_, _| Some(Arc::new(MockProvider)),
            Arc::new(StaticCapabilityNegotiator {
                adapter: ProviderAdapterKind::Mock,
                instruction_encoding: ProviderInstructionEncoding::NativeRoles,
                tool_protocol: ProviderToolProtocolCapabilities::default(),
            }),
        );
        registry.register_builtin(
            ProviderAdapterKind::OpenAiChat,
            ProviderTransportKind::Http,
            "OpenAI-compatible",
            |settings, usage| {
                OpenAiCompatibleProvider::from_settings(settings).map(|provider| match usage {
                    ProviderDriverUse::Model => Arc::new(provider) as Arc<dyn ModelProvider>,
                    ProviderDriverUse::Guardian => {
                        Arc::new(provider.for_guardian()) as Arc<dyn ModelProvider>
                    }
                })
            },
            Arc::new(OpenAiCapabilityNegotiator),
        );
        registry.register_builtin(
            ProviderAdapterKind::OpenAiResponses,
            ProviderTransportKind::Http,
            "OpenAI Responses",
            |settings, usage| {
                OpenAiResponsesProvider::from_settings(settings).map(|provider| match usage {
                    ProviderDriverUse::Model => Arc::new(provider) as Arc<dyn ModelProvider>,
                    ProviderDriverUse::Guardian => {
                        Arc::new(provider.for_guardian()) as Arc<dyn ModelProvider>
                    }
                })
            },
            Arc::new(OpenAiCapabilityNegotiator),
        );
        registry.register_builtin(
            ProviderAdapterKind::AnthropicMessages,
            ProviderTransportKind::Http,
            "Anthropic Messages",
            |settings, usage| {
                AnthropicMessagesProvider::from_settings(settings).map(|provider| match usage {
                    ProviderDriverUse::Model => Arc::new(provider) as Arc<dyn ModelProvider>,
                    ProviderDriverUse::Guardian => {
                        Arc::new(provider.for_guardian()) as Arc<dyn ModelProvider>
                    }
                })
            },
            Arc::new(AnthropicCapabilityNegotiator),
        );
        registry.register_builtin(
            ProviderAdapterKind::CodexAppServer,
            ProviderTransportKind::CodexAppServer,
            "Codex App Server",
            |settings, usage| {
                CodexAppServerProvider::from_settings(settings).map(|provider| match usage {
                    ProviderDriverUse::Model => Arc::new(provider) as Arc<dyn ModelProvider>,
                    ProviderDriverUse::Guardian => {
                        Arc::new(provider.for_guardian()) as Arc<dyn ModelProvider>
                    }
                })
            },
            Arc::new(StaticCapabilityNegotiator {
                adapter: ProviderAdapterKind::CodexAppServer,
                instruction_encoding: ProviderInstructionEncoding::NativeRoles,
                tool_protocol: ProviderToolProtocolCapabilities::default(),
            }),
        );
        registry
    }
}

impl ProviderDriverRegistry {
    pub fn built_in() -> &'static Self {
        static REGISTRY: OnceLock<ProviderDriverRegistry> = OnceLock::new();
        REGISTRY.get_or_init(Self::default)
    }

    pub fn descriptors(&self) -> Vec<ProviderDriverDescriptor> {
        self.drivers
            .values()
            .map(|registration| registration.descriptor.clone())
            .collect()
    }

    pub fn descriptor(&self, adapter: ProviderAdapterKind) -> Option<&ProviderDriverDescriptor> {
        self.drivers
            .get(adapter.as_str())
            .map(|registration| &registration.descriptor)
    }

    pub fn create(&self, settings: &ProviderSettings) -> Option<Arc<dyn ModelProvider>> {
        self.create_for(settings, ProviderDriverUse::Model)
    }

    pub fn create_guardian(&self, settings: &ProviderSettings) -> Option<Arc<dyn ModelProvider>> {
        self.create_for(settings, ProviderDriverUse::Guardian)
    }

    fn register_builtin(
        &mut self,
        adapter: ProviderAdapterKind,
        transport: ProviderTransportKind,
        display_name: &str,
        factory: ProviderDriverFactory,
        negotiator: Arc<dyn ProviderCapabilityNegotiator>,
    ) {
        let id = adapter.as_str().to_string();
        let replaced = self.drivers.insert(
            id.clone(),
            ProviderDriverRegistration {
                descriptor: ProviderDriverDescriptor {
                    id,
                    adapter,
                    transport,
                    display_name: display_name.to_string(),
                    trust: ProviderDriverTrust::BuiltIn,
                },
                factory,
                negotiator,
            },
        );
        debug_assert!(replaced.is_none(), "duplicate built-in provider driver");
    }

    fn create_for(
        &self,
        settings: &ProviderSettings,
        usage: ProviderDriverUse,
    ) -> Option<Arc<dyn ModelProvider>> {
        self.create_for_adapter(
            settings,
            settings.resolved_adapter_for_model(&settings.model),
            usage,
        )
    }

    fn create_for_adapter(
        &self,
        settings: &ProviderSettings,
        adapter: ProviderAdapterKind,
        usage: ProviderDriverUse,
    ) -> Option<Arc<dyn ModelProvider>> {
        self.drivers.get(adapter.as_str()).and_then(|registration| {
            (registration.descriptor.transport == settings.effective_transport())
                .then(|| (registration.factory)(settings, usage))
                .flatten()
        })
    }

    async fn negotiate(
        &self,
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderNegotiationResult> {
        let registration = self
            .drivers
            .get(
                settings
                    .resolved_adapter_for_model(&settings.model)
                    .as_str(),
            )
            .context("provider driver is not registered")?;
        anyhow::ensure!(
            registration.descriptor.transport == settings.effective_transport(),
            "provider adapter is not supported by the selected transport"
        );
        registration.negotiator.negotiate(settings).await
    }
}

/// Builds the transport for a connection. Callers pass settings that already
/// carry any per-thread model override. An unconfigured connection degrades to
/// [`MockProvider`] to preserve the desktop first-run behaviour.
pub fn provider_from_settings(settings: &ProviderSettings) -> Arc<dyn ModelProvider> {
    configured_provider_from_settings(settings).unwrap_or_else(|| Arc::new(MockProvider))
}

pub fn configured_provider_from_settings(
    settings: &ProviderSettings,
) -> Option<Arc<dyn ModelProvider>> {
    ProviderDriverRegistry::built_in().create(settings)
}

pub fn configured_provider_from_settings_with_adapter(
    settings: &ProviderSettings,
    adapter: ProviderAdapterKind,
) -> Option<Arc<dyn ModelProvider>> {
    ProviderDriverRegistry::built_in().create_for_adapter(
        settings,
        adapter,
        ProviderDriverUse::Model,
    )
}

#[derive(Debug, Clone)]
pub struct ProviderNegotiationResult {
    pub health: ProviderHealthCheck,
    pub adapter_profiles: Vec<ProviderAdapterProfile>,
}

/// Runs the connection's protocol-specific readiness negotiation and returns a
/// normalized adapter contract. Settings/UI callers use this once; request
/// codecs consume the persisted profile and never probe on the send path.
pub async fn negotiate_provider_settings(
    settings: &ProviderSettings,
) -> anyhow::Result<ProviderNegotiationResult> {
    ProviderDriverRegistry::built_in().negotiate(settings).await
}

/// Same connection, constrained for guardian review calls.
pub fn guardian_provider_from_settings(settings: &ProviderSettings) -> Arc<dyn ModelProvider> {
    ProviderDriverRegistry::built_in()
        .create_guardian(settings)
        .unwrap_or_else(|| Arc::new(MockProvider))
}

mod anthropic;

pub use anthropic::AnthropicMessagesProvider;

#[cfg(test)]
use anthropic::{anthropic_system_instructions, anthropic_tool_result, anthropic_tools};

#[derive(Debug, Default)]
struct ProviderEnv {
    values: HashMap<String, String>,
}

fn provider_api_key(settings: &ProviderSettings) -> Option<String> {
    if matches!(
        settings.effective_auth(),
        ProviderAuthKind::None | ProviderAuthKind::CodexSession
    ) {
        return Some(String::new());
    }
    std::env::var(&settings.api_key_source)
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            if settings.api_key_source != "OPENTOPIA_API_KEY" {
                return None;
            }
            ProviderEnv::load().first([
                "OPENTOPIA_API_KEY",
                "AUDIT_COPILOT_LLM_API_KEY",
                "CREDIT_REVIEW_LLM_API_KEY",
                "OPENAI_API_KEY",
            ])
        })
}

fn apply_provider_auth(
    request: reqwest::RequestBuilder,
    auth: ProviderAuthKind,
    secret: &str,
) -> reqwest::RequestBuilder {
    match auth {
        ProviderAuthKind::Bearer => request.header(AUTHORIZATION, format!("Bearer {secret}")),
        ProviderAuthKind::XApiKey => request.header("x-api-key", secret),
        ProviderAuthKind::CodexSession | ProviderAuthKind::None => request,
    }
}

fn ensure_visual_input_supported(
    request: &ModelRequest,
    supports_vision: bool,
) -> anyhow::Result<()> {
    if !supports_vision && request_has_image_input(request) {
        anyhow::bail!(
            "this provider is marked as not supporting visual input; enable supportsVision before attaching images"
        );
    }
    Ok(())
}

fn request_image_part_count(request: &ModelRequest) -> usize {
    request
        .input
        .conversation
        .iter()
        .flat_map(|message| message.content_parts.iter())
        .chain(request.input.current_user.content.iter())
        .chain(
            request
                .input
                .tool_results
                .iter()
                .flat_map(|result| result.content.iter()),
        )
        .filter(|part| matches!(part, ModelContentPart::Image { .. }))
        .count()
}

fn request_has_image_input(request: &ModelRequest) -> bool {
    request_image_part_count(request) > 0
}

fn provider_rejected_image_input(body: &str) -> bool {
    let body_lower = body.to_ascii_lowercase();
    body.contains("图片")
        || body.contains("图像")
        || body.contains("多模态")
        || body_lower.contains("image")
        || body_lower.contains("vision")
        || body_lower.contains("multimodal")
}

fn provider_rejected_parallel_tool_calls(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    (body.contains("parallel_tool_calls") || body.contains("parallel tool calls"))
        && [
            "unsupported",
            "not supported",
            "unknown",
            "unrecognized",
            "invalid",
            "unexpected",
            "not allowed",
        ]
        .iter()
        .any(|marker| body.contains(marker))
}

fn provider_rejected_json_schema_output(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    (body.contains("response_format")
        || body.contains("json_schema")
        || body.contains("json schema")
        || body.contains("text.format")
        || body.contains("structured output"))
        && [
            "unsupported",
            "not supported",
            "unknown",
            "unrecognized",
            "invalid",
            "unexpected",
            "not allowed",
        ]
        .iter()
        .any(|marker| body.contains(marker))
}

fn rejected_chat_profile_capability(body: &Value, error: &str) -> Option<&'static str> {
    let uses_native_developer_role =
        body.get("messages")
            .and_then(Value::as_array)
            .is_some_and(|messages| {
                messages
                    .iter()
                    .any(|message| message.get("role") == Some(&json!("developer")))
            });
    if uses_native_developer_role && provider_rejected_developer_messages(error) {
        Some("developer_role")
    } else if body.get("parallel_tool_calls").is_some()
        && provider_rejected_parallel_tool_calls(error)
    {
        Some("parallel_tool_calls")
    } else if request_uses_strict_function_tools(body)
        && provider_rejected_strict_function_tools(error)
    {
        Some("strict_function_tools")
    } else if body.get("response_format").is_some() && provider_rejected_json_schema_output(error) {
        Some("json_schema_output")
    } else {
        None
    }
}

fn rejected_responses_profile_capability(body: &Value, error: &str) -> Option<&'static str> {
    if body.get("parallel_tool_calls").is_some() && provider_rejected_parallel_tool_calls(error) {
        Some("parallel_tool_calls")
    } else if request_uses_strict_function_tools(body)
        && provider_rejected_strict_function_tools(error)
    {
        Some("strict_function_tools")
    } else if body.get("text").is_some() && provider_rejected_json_schema_output(error) {
        Some("json_schema_output")
    } else if responses_request_uses_enhanced_tools(body)
        && provider_rejected_tool_representation(error)
    {
        Some("enhanced_tool_protocol")
    } else {
        None
    }
}

fn provider_rejected_developer_messages(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("developer")
        && [
            "unsupported",
            "not supported",
            "unknown variant",
            "unknown role",
            "invalid role",
            "unrecognized",
            "not allowed",
            "expected one of",
        ]
        .iter()
        .any(|marker| body.contains(marker))
}

impl ProviderEnv {
    fn load() -> Self {
        let mut values = std::env::vars().collect::<HashMap<_, _>>();
        for path in candidate_env_files() {
            merge_dotenv_file(&mut values, &path);
        }
        Self { values }
    }

    fn first<const N: usize>(&self, keys: [&str; N]) -> Option<String> {
        keys.into_iter().find_map(|key| {
            self.values
                .get(key)
                .filter(|value| !value.is_empty())
                .cloned()
        })
    }
}

fn candidate_env_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("OPENTOPIA_ENV_FILE") {
        paths.push(PathBuf::from(path));
    }
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join(".env"));
        if let Some(parent) = cwd.parent() {
            paths.push(parent.join(credit_review_project_name()).join(".env"));
            paths.extend(find_sibling_credit_review_env_files(parent));
        }
    }
    paths
}

fn credit_review_project_name() -> String {
    [0x4FE1, 0x8D37, 0x5BA1, 0x6838, 0x52A9, 0x624B]
        .into_iter()
        .filter_map(char::from_u32)
        .collect()
}

fn find_sibling_credit_review_env_files(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(".env"))
        .filter(|path| {
            std::fs::read_to_string(path).is_ok_and(|content| {
                content.contains("CREDIT_REVIEW_LLM_API_KEY")
                    || content.contains("AUDIT_COPILOT_LLM_API_KEY")
            })
        })
        .collect()
}

fn merge_dotenv_file(values: &mut HashMap<String, String>, path: &Path) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };

    for line in content.lines() {
        let Some((key, value)) = parse_dotenv_line(line) else {
            continue;
        };
        values.entry(key).or_insert(value);
    }
}

fn parse_dotenv_line(line: &str) -> Option<(String, String)> {
    let mut line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(rest) = line.strip_prefix("export ") {
        line = rest.trim();
    }

    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    let value = strip_env_quotes(value.trim());
    Some((key.to_string(), value.to_string()))
}

fn strip_env_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn instruction_messages(request: &ModelRequest) -> Vec<(ContextRole, String)> {
    scoped_instruction_messages(request, true)
        .into_iter()
        .chain(scoped_instruction_messages(request, false))
        .collect()
}

fn scoped_instruction_messages(
    request: &ModelRequest,
    lineage_prefix: bool,
) -> Vec<(ContextRole, String)> {
    request
        .instructions
        .instruction_messages_with_scope()
        .into_iter()
        .filter_map(|(role, scope, content)| {
            let belongs_to_prefix =
                matches!(scope, ContextCacheScope::Stable | ContextCacheScope::Thread);
            (belongs_to_prefix == lineage_prefix).then_some((role, content))
        })
        .collect()
}
fn resource_fallback_text(uri: &str, content_type: Option<&str>, name: Option<&str>) -> String {
    let mut fields = vec![format!("uri={uri}")];
    if let Some(name) = name {
        fields.push(format!("name={name}"));
    }
    if let Some(content_type) = content_type {
        fields.push(format!("contentType={content_type}"));
    }
    format!("[Attached resource: {}]", fields.join(", "))
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0b0000_0011) << 4 | second >> 4) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((second & 0b0000_1111) << 2 | third >> 6) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(third & 0b0011_1111) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn redact_transport_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let normalized = key.to_ascii_lowercase();
                    let is_image_bytes = normalized == "data"
                        && fields.get("type").and_then(Value::as_str) == Some("image")
                        && value.is_array();
                    let value = if is_image_bytes {
                        Value::String(format!(
                            "[binary image omitted: {} bytes]",
                            value.as_array().map(Vec::len).unwrap_or_default()
                        ))
                    } else if matches!(
                        normalized.as_str(),
                        "authorization"
                            | "api_key"
                            | "apikey"
                            | "password"
                            | "secret"
                            | "access_token"
                            | "refresh_token"
                    ) {
                        Value::String("[REDACTED]".to_string())
                    } else {
                        redact_transport_value(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_transport_value).collect()),
        Value::String(text) if text.starts_with("data:") && text.contains(";base64,") => {
            Value::String(format!("[data URL omitted: {} chars]", text.len()))
        }
        Value::String(text) if text.len() > 256_000 => Value::String(format!(
            "{}\n[observation truncated: {} chars total]",
            text.chars().take(256_000).collect::<String>(),
            text.len()
        )),
        value => value.clone(),
    }
}

pub fn redact_model_observation(value: &Value) -> Value {
    redact_transport_value(value)
}

fn truncate_observation_text(text: &str) -> String {
    const LIMIT: usize = 16_000;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    format!(
        "{}\n[observation truncated: {} chars total]",
        text.chars().take(LIMIT).collect::<String>(),
        text.len()
    )
}
mod codex_app_server;

pub use codex_app_server::{
    CodexAccountManager, CodexAccountStatus, CodexAppServerProvider, CodexLoginStart,
};

#[cfg(test)]
use codex_app_server::{
    codex_developer_instructions, codex_dynamic_tool_call, codex_dynamic_tools, codex_item_text,
    codex_turn_input, is_codex_builtin_action, is_isolated_codex_host_profile,
};

#[derive(Debug, Default)]
pub struct MockProvider;

#[async_trait]
impl ModelProvider for MockProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        Ok(ModelResponse::text(format!(
            "OpenTopia MVP mock provider received: {}",
            request.input.current_user.message
        )))
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        Ok(ProviderHealthCheck {
            reachable: true,
            latency_ms: None,
            model_available: false,
            error: None,
            openai_compatibility: None,
        })
    }
}

#[cfg(test)]
mod tests;
