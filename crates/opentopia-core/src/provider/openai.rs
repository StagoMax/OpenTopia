use super::transport::{next_stream_chunk, stream_idle_timeout, SseDecoder};
use super::{
    apply_provider_auth, ensure_visual_input_supported, provider_api_key, redact_transport_value,
    rejected_responses_profile_capability, require_function_tools, truncate_observation_text,
    ModelFinishReason, ModelProvider, ModelRequest, ModelResponse, ModelStreamCallback,
    ModelStreamDelta, PreparedProviderRequest, ProviderAdapterError, ProviderEnv,
    ProviderResponseCommitMode, ProviderToolCandidate, ProviderTransportCallback,
    ProviderTransportEvent, ProviderWireTranscript,
};
use crate::model::ProviderRetryKind;
use crate::settings::{
    is_official_openai_endpoint, official_openai_explicit_prompt_cache_support,
    official_openai_tool_search_support, trusted_chat_message_protocol_contract,
    OpenAiCompatibilityReport, OpenAiProtocol, PromptCachePolicy, ProviderAdapterKind,
    ProviderAuthKind, ProviderFeatureSupport, ProviderHealthCheck, ProviderInstructionEncoding,
    ProviderMessageProtocolCapabilities, ProviderOutputProtocolCapabilities,
    ProviderReasoningProtocol, ProviderSettings, ProviderToolProtocolCapabilities,
    ProviderTransportKind,
};
use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::header::CONTENT_TYPE;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

mod codec;
mod decode;
mod execute;
mod probe;
mod reasoning;
mod recovery;
mod stream;

use codec::{
    add_responses_prompt_cache_breakpoint, compile_responses_tools, normalize_provider_tool_calls,
    responses_system_instructions,
};
pub(in crate::provider) use codec::{
    compile_openai_tools, legacy_tool_observation, nonredundant_tool_result_content,
    openai_messages_with_reasoning, openai_portable_messages_with_reasoning,
    provider_tool_result_content, responses_input, OPENAI_CHAT_ASSISTANT_STATE_TYPE,
};
#[cfg(test)]
pub(in crate::provider) use codec::{
    normalize_provider_arguments, openai_messages, openai_portable_messages,
    openai_strict_function_schema, openai_tools, responses_tool_result_output, responses_tools,
};
pub(in crate::provider) use codec::{
    OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT, OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT,
};
pub(in crate::provider) use decode::parse_model_response_body_with_tools;
#[cfg(test)]
pub(in crate::provider) use decode::{
    extract_provider_tool_calls, extract_response_text, parse_model_response_body,
    INVALID_TOOL_ARGUMENTS_JSON_KEY,
};
pub(crate) use decode::{
    invalid_tool_arguments_json_details, normalize_tool_argument_keys, tool_input_schema_error,
};
pub(in crate::provider) use decode::{
    model_response_observation, parse_model_usage, parse_required_tool_arguments,
    tool_call_protocol_error_observation,
};
pub(in crate::provider) use probe::OpenAiProbeClient;
use reasoning::{
    apply_reasoning_protocol, default_reasoning_protocol, reasoning_probe_candidates,
    reasoning_protocol_label, AppliedReasoning,
};
pub(in crate::provider) use stream::{
    chat_finish_reason, OpenAiStreamAccumulator, ResponsesStreamAccumulator, StreamingToolCall,
};

// Request encoding and response decoding live in dedicated protocol modules.

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    pub(in crate::provider) client: reqwest::Client,
    pub(in crate::provider) base_url: String,
    pub(in crate::provider) api_key: String,
    pub(in crate::provider) auth: ProviderAuthKind,
    pub(in crate::provider) model: String,
    pub(in crate::provider) temperature: Option<f64>,
    pub(in crate::provider) max_output_tokens: Option<u32>,
    pub(in crate::provider) reasoning_effort: Option<String>,
    pub(in crate::provider) parallel_tool_calls: bool,
    pub(in crate::provider) prompt_cache_key: Option<String>,
    pub(in crate::provider) supports_vision: bool,
    pub(in crate::provider) chat_codec: OpenAiChatCodec,
    pub(in crate::provider) reasoning_protocol: ProviderReasoningProtocol,
    pub(in crate::provider) output_protocol: ProviderOutputProtocolCapabilities,
    pub(in crate::provider) tool_protocol: ProviderToolProtocolCapabilities,
}

/// Deterministic Canonical Message -> OpenAI Chat wire codec. Capability
/// negotiation owns the profile; this codec only applies the selected mapping
/// once and has no network or capability-discovery access.
#[derive(Debug, Clone, Copy)]
pub(super) struct OpenAiChatCodec {
    pub(in crate::provider) instruction_encoding: ProviderInstructionEncoding,
    pub(in crate::provider) message_protocol: ProviderMessageProtocolCapabilities,
}

impl OpenAiChatCodec {
    fn encode_messages(&self, request: &ModelRequest, thinking_enabled: bool) -> Vec<Value> {
        let replay_reasoning = thinking_enabled
            && self
                .message_protocol
                .requires_reasoning_content_for_tool_calls;
        match self.instruction_encoding {
            ProviderInstructionEncoding::NativeRoles => {
                openai_messages_with_reasoning(request, replay_reasoning)
            }
            ProviderInstructionEncoding::PortableChatEnvelope
            | ProviderInstructionEncoding::FoldDeveloperIntoSystem => {
                openai_portable_messages_with_reasoning(request, replay_reasoning)
            }
        }
    }

    fn transcript_format(&self) -> &'static str {
        match self.instruction_encoding {
            ProviderInstructionEncoding::NativeRoles => OPENAI_CHAT_NATIVE_TRANSCRIPT_FORMAT,
            ProviderInstructionEncoding::PortableChatEnvelope
            | ProviderInstructionEncoding::FoldDeveloperIntoSystem => {
                OPENAI_CHAT_PORTABLE_TRANSCRIPT_FORMAT
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct OpenAiProbeOutcome {
    pub(super) support: ProviderFeatureSupport,
    pub(super) detail: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct OpenAiMessageProtocolProbeOutcome {
    capabilities: ProviderMessageProtocolCapabilities,
    detail: Option<String>,
}

#[derive(Debug, Clone)]
struct OpenAiReasoningProbeOutcome {
    protocol: Option<ProviderReasoningProtocol>,
    tool_support: OpenAiProbeOutcome,
}

pub(super) fn compatibility_probe_candidate(expected_token: &str) -> ProviderToolCandidate {
    ProviderToolCandidate::direct(
        "compatibility_probe",
        "Returns the exact compatibility token supplied by the user.",
        json!({
            "type": "object",
            "properties": {
                "token": { "type": "string", "const": expected_token }
            },
            "required": ["token"],
            "additionalProperties": false
        }),
    )
}

pub(super) fn validate_tool_probe_response(
    response: &ModelResponse,
    expected_token: &str,
) -> Result<(), String> {
    if response.tool_calls.len() != 1 {
        return Err(format!(
            "HTTP 200 returned {} tool calls; expected exactly one compatibility_probe call",
            response.tool_calls.len()
        ));
    }
    let call = &response.tool_calls[0];
    if call.name != "compatibility_probe" {
        return Err(format!(
            "HTTP 200 returned tool '{}'; expected compatibility_probe",
            call.name
        ));
    }
    if call.arguments.get("token").and_then(Value::as_str) != Some(expected_token) {
        return Err(
            "HTTP 200 returned a compatibility_probe call without the required token argument"
                .to_string(),
        );
    }
    Ok(())
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into();
        let model = model.into();
        // An OpenAI-compatible endpoint is not proof that it implements every
        // OpenAI message role. Start third-party/relay connections on the
        // portable system/user shape and promote them to native `developer`
        // messages only after an explicit capability probe reports support.
        let instruction_encoding = if is_official_openai_endpoint(&base_url) {
            ProviderInstructionEncoding::NativeRoles
        } else {
            // Direct constructors have no negotiated settings contract. Keep
            // the portable deterministic default; configured providers replace
            // it with the persisted adapter profile below.
            ProviderInstructionEncoding::PortableChatEnvelope
        };
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.clone(),
            api_key: api_key.into(),
            auth: ProviderAuthKind::Bearer,
            model: model.clone(),
            temperature: None,
            max_output_tokens: None,
            reasoning_effort: None,
            parallel_tool_calls: true,
            prompt_cache_key: None,
            supports_vision: true,
            chat_codec: OpenAiChatCodec {
                instruction_encoding,
                message_protocol: trusted_chat_message_protocol_contract(&base_url, &model)
                    .unwrap_or_default(),
            },
            reasoning_protocol: default_reasoning_protocol(ProviderAdapterKind::OpenAiChat, &model),
            output_protocol: ProviderOutputProtocolCapabilities::default(),
            tool_protocol: ProviderToolProtocolCapabilities {
                function_tools: ProviderFeatureSupport::Supported,
                ..ProviderToolProtocolCapabilities::default()
            },
        }
    }

    pub fn from_env() -> Option<Self> {
        let env = ProviderEnv::load();
        let api_key = env.first([
            "OPENTOPIA_API_KEY",
            "AUDIT_COPILOT_LLM_API_KEY",
            "CREDIT_REVIEW_LLM_API_KEY",
            "OPENAI_API_KEY",
        ])?;
        let base_url = env
            .first([
                "OPENTOPIA_OPENAI_BASE_URL",
                "AUDIT_COPILOT_LLM_BASE_URL",
                "CREDIT_REVIEW_LLM_BASE_URL",
                "OPENAI_BASE_URL",
            ])
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let model = env
            .first([
                "OPENTOPIA_MODEL",
                "AUDIT_COPILOT_LLM_MODEL",
                "CREDIT_REVIEW_LLM_MODEL",
                "CREDIT_REVIEW_LLM_CHEAP_MODEL",
                "CREDIT_REVIEW_LLM_STRONG_MODEL",
            ])
            .unwrap_or_else(|| "gpt-4.1-mini".to_string());
        Some(Self::new(base_url, api_key, model))
    }

    pub fn from_settings(settings: &ProviderSettings) -> Option<Self> {
        Self::from_openai_settings(settings)
    }

    fn from_openai_settings(settings: &ProviderSettings) -> Option<Self> {
        let api_key = provider_api_key(settings)?;
        let mut provider = Self::new(settings.base_url.clone(), api_key, settings.model.clone());
        provider.auth = settings.effective_auth();
        Some(provider.with_generation_settings(settings))
    }

    pub(in crate::provider) fn with_generation_settings(
        mut self,
        settings: &ProviderSettings,
    ) -> Self {
        if let Some(temperature) = settings.temperature_for_model() {
            self.temperature = Some(temperature);
        }
        self.max_output_tokens = settings.max_output_tokens_for_model();
        self.reasoning_effort = settings.reasoning_effort_for_model();
        self.parallel_tool_calls = settings.parallel_tool_calls;
        self.prompt_cache_key = settings.prompt_cache_key.clone();
        self.supports_vision = settings.supports_vision_for_model();
        self.tool_protocol = settings
            .capabilities_for_adapter(ProviderAdapterKind::OpenAiChat)
            .tool_protocol;
        if let Some(profile) = settings
            .adapter_profile_for_model_and_adapter(&settings.model, ProviderAdapterKind::OpenAiChat)
        {
            self.chat_codec.instruction_encoding = profile.instruction_encoding;
            self.chat_codec.message_protocol = self
                .chat_codec
                .message_protocol
                .union(profile.message_protocol);
            self.reasoning_protocol = profile.reasoning_protocol;
            self.output_protocol = profile.output_protocol;
        }
        self
    }

    fn apply_chat_reasoning_options(&self, payload: &mut Value) -> AppliedReasoning {
        apply_reasoning_protocol(
            self.reasoning_protocol,
            self.reasoning_effort.as_deref(),
            None,
            payload,
        )
    }

    fn apply_probe_reasoning_options(
        &self,
        protocol: ProviderReasoningProtocol,
        payload: &mut Value,
    ) {
        let effort = self
            .reasoning_effort
            .as_deref()
            .filter(|value| !matches!(*value, "none" | "minimal"))
            .unwrap_or("high");
        let applied = apply_reasoning_protocol(protocol, Some(effort), None, payload);
        if applied.omit_tool_choice {
            payload
                .as_object_mut()
                .expect("OpenAI compatibility probe payload")
                .remove("tool_choice");
        }
        if applied.omit_temperature {
            payload
                .as_object_mut()
                .expect("OpenAI compatibility probe payload")
                .remove("temperature");
        }
    }

    pub async fn probe_settings(
        settings: &ProviderSettings,
    ) -> anyhow::Result<ProviderHealthCheck> {
        if settings.effective_transport() != ProviderTransportKind::Http
            || !settings.effective_allowed_adapters().iter().any(|adapter| {
                matches!(
                    adapter,
                    ProviderAdapterKind::OpenAiChat | ProviderAdapterKind::OpenAiResponses
                )
            })
        {
            anyhow::bail!("compatibility probing requires an OpenAI-compatible provider");
        }
        let provider = Self::from_openai_settings(settings)
            .context("OpenAI-compatible provider is not configured")?;
        provider
            .probe_compatibility(settings.resolved_adapter_for_model(&settings.model))
            .await
    }

    pub(super) async fn probe_compatibility(
        &self,
        preferred_adapter: ProviderAdapterKind,
    ) -> anyhow::Result<ProviderHealthCheck> {
        const TOOL_PROBE_TOKEN: &str = "opentopia-tool-probe-v1";
        let start = std::time::Instant::now();
        let probe_client = OpenAiProbeClient::new(self);
        let probe_enhanced_features = is_official_openai_endpoint(&self.base_url);
        let mut chat_payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "System compatibility probe."},
                {"role": "user", "content": "Reply with OK."}
            ],
            "max_tokens": 16,
            "stream": false
        });
        let mut responses_payload = json!({
            "model": self.model,
            "input": "Reply with OK.",
            "max_output_tokens": 16,
            "stream": false,
            "store": false
        });
        let mut chat_json_schema_output_payload = json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": "Return a JSON object whose ok property is true."
            }],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "compatibility_probe",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {"ok": {"type": "boolean"}},
                        "required": ["ok"],
                        "additionalProperties": false
                    }
                }
            },
            "max_tokens": 64,
            "stream": false
        });
        let mut responses_json_schema_output_payload = json!({
            "model": self.model,
            "input": "Return a JSON object whose ok property is true.",
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "compatibility_probe",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": {"ok": {"type": "boolean"}},
                        "required": ["ok"],
                        "additionalProperties": false
                    }
                }
            },
            "max_output_tokens": 64,
            "stream": false,
            "store": false
        });
        let mut chat_strict_tools_payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "System compatibility probe."},
                {"role": "user", "content": format!("Call compatibility_probe exactly once with token {TOOL_PROBE_TOKEN}.")}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "compatibility_probe",
                    "description": "Returns the exact compatibility token supplied by the user.",
                    "parameters": {
                        "type": "object",
                        "properties": {"token": {"type": "string", "const": TOOL_PROBE_TOKEN}},
                        "required": ["token"],
                        "additionalProperties": false
                    },
                    "strict": true
                }
            }],
            "tool_choice": {"type": "function", "function": {"name": "compatibility_probe"}},
            // Thinking models spend output tokens before emitting the tool
            // call. A tiny budget creates a false "tools unsupported" result.
            "max_tokens": 1024,
            "stream": false
        });
        let mut responses_native_tools_payload = json!({
            "model": self.model,
            "input": "Reply with OK.",
            "tools": [{"type": "web_search"}],
            "tool_choice": "none",
            "max_output_tokens": 16,
            "stream": false,
            "store": false
        });
        let mut responses_strict_function_tools_payload = json!({
            "model": self.model,
            "input": format!("Call compatibility_probe exactly once with token {TOOL_PROBE_TOKEN}."),
            "tools": [{
                "type": "function",
                "name": "compatibility_probe",
                "description": "Returns the exact compatibility token supplied by the user.",
                "parameters": {
                    "type": "object",
                    "properties": {"token": {"type": "string", "const": TOOL_PROBE_TOKEN}},
                    "required": ["token"],
                    "additionalProperties": false
                },
                "strict": true
            }],
            "tool_choice": {"type": "function", "name": "compatibility_probe"},
            "max_output_tokens": 1024,
            "stream": false,
            "store": false
        });
        let mut responses_custom_tools_payload = json!({
            "model": self.model,
            "input": "Reply with OK.",
            "tools": [{
                "type": "custom",
                "name": "compatibility_probe",
                "description": "Validates Responses freeform-tool support without invoking a tool."
            }],
            "tool_choice": "none",
            "max_output_tokens": 16,
            "stream": false,
            "store": false
        });
        let mut responses_apply_patch_payload = json!({
            "model": self.model,
            "input": "Reply with OK.",
            "tools": [{"type": "apply_patch"}],
            "tool_choice": "none",
            "max_output_tokens": 16,
            "stream": false,
            "store": false
        });
        let mut chat_portable_tools_payload = chat_strict_tools_payload.clone();
        chat_portable_tools_payload["tools"][0]["function"]
            .as_object_mut()
            .expect("chat function probe")
            .remove("strict");
        let known_chat_message_protocol =
            trusted_chat_message_protocol_contract(&self.base_url, &self.model).unwrap_or_default();
        let mut responses_portable_function_tools_payload =
            responses_strict_function_tools_payload.clone();
        responses_portable_function_tools_payload["tools"][0]
            .as_object_mut()
            .expect("responses function probe")
            .remove("strict");
        // Reasoning negotiation is serialized per adapter because the first
        // successful function-tool round trip becomes the persisted runtime
        // contract. Model names only order candidates; they never decide the
        // result. Chat and Responses remain independent and can negotiate in
        // parallel behind the shared rate-limit-aware probe client.
        let (chat_reasoning, responses_reasoning) = tokio::join!(
            self.probe_reasoning_protocol(
                &probe_client,
                ProviderAdapterKind::OpenAiChat,
                "/chat/completions",
                chat_portable_tools_payload.clone(),
                TOOL_PROBE_TOKEN,
            ),
            self.probe_reasoning_protocol(
                &probe_client,
                ProviderAdapterKind::OpenAiResponses,
                "/responses",
                responses_portable_function_tools_payload.clone(),
                TOOL_PROBE_TOKEN,
            ),
        );
        let chat_reasoning_protocol = chat_reasoning.protocol.unwrap_or(self.reasoning_protocol);
        let responses_reasoning_protocol = responses_reasoning
            .protocol
            .unwrap_or(ProviderReasoningProtocol::ResponsesReasoning);
        for payload in [
            &mut chat_payload,
            &mut chat_json_schema_output_payload,
            &mut chat_strict_tools_payload,
            &mut chat_portable_tools_payload,
        ] {
            self.apply_probe_reasoning_options(chat_reasoning_protocol, payload);
        }
        for payload in [
            &mut responses_payload,
            &mut responses_json_schema_output_payload,
            &mut responses_native_tools_payload,
            &mut responses_strict_function_tools_payload,
            &mut responses_custom_tools_payload,
            &mut responses_apply_patch_payload,
            &mut responses_portable_function_tools_payload,
        ] {
            self.apply_probe_reasoning_options(responses_reasoning_protocol, payload);
        }

        let chat_function_tools = chat_reasoning.tool_support;
        let responses_function_tools = responses_reasoning.tool_support;
        let (chat_strict_function_tools, responses_strict_function_tools) = tokio::join!(
            async {
                if chat_function_tools.support == ProviderFeatureSupport::Supported {
                    self.probe_openai_function_tool_roundtrip(
                        &probe_client,
                        "/chat/completions",
                        chat_strict_tools_payload.clone(),
                        TOOL_PROBE_TOKEN,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(
                            "strict tools were not probed because the portable Chat function contract failed"
                                .to_string(),
                        ),
                    }
                }
            },
            async {
                if responses_function_tools.support == ProviderFeatureSupport::Supported {
                    self.probe_openai_function_tool_roundtrip(
                        &probe_client,
                        "/responses",
                        responses_strict_function_tools_payload.clone(),
                        TOOL_PROBE_TOKEN,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(
                            "strict tools were not probed because the portable Responses function contract failed"
                                .to_string(),
                        ),
                    }
                }
            },
        );

        // Streaming uses the strongest non-streaming tool shape already
        // proven. A failed strict probe therefore degrades only strictness, not
        // the portable streaming contract.
        let mut chat_streaming_tools_payload =
            if chat_strict_function_tools.support == ProviderFeatureSupport::Supported {
                chat_strict_tools_payload.clone()
            } else {
                chat_portable_tools_payload.clone()
            };
        chat_streaming_tools_payload["stream"] = json!(true);
        chat_streaming_tools_payload["stream_options"] = json!({ "include_usage": true });
        chat_streaming_tools_payload["parallel_tool_calls"] = json!(true);
        let chat_message_protocol_payload = chat_portable_tools_payload.clone();
        let mut responses_streaming_tools_payload =
            if responses_strict_function_tools.support == ProviderFeatureSupport::Supported {
                responses_strict_function_tools_payload.clone()
            } else {
                responses_portable_function_tools_payload.clone()
            };
        responses_streaming_tools_payload["stream"] = json!(true);
        responses_streaming_tools_payload["parallel_tool_calls"] = json!(true);
        let mut developer_payload = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "System compatibility probe."},
                {"role": "developer", "content": "Developer compatibility probe."},
                {"role": "user", "content": "Reply with OK."}
            ],
            "max_tokens": 16,
            "stream": false
        });
        self.apply_probe_reasoning_options(chat_reasoning_protocol, &mut developer_payload);
        let probe_developer_in_main_batch =
            chat_function_tools.support == ProviderFeatureSupport::Supported;

        let (
            mut chat,
            mut responses,
            chat_streaming_tools,
            chat_json_schema_output,
            chat_message_protocol,
            responses_native_tools,
            responses_streaming_tools,
            responses_json_schema_output,
            responses_custom_tools,
            responses_apply_patch,
            early_developer,
        ) = tokio::join!(
            async {
                if chat_function_tools.support == ProviderFeatureSupport::Supported {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Supported,
                        detail: None,
                    }
                } else {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/chat/completions",
                        chat_payload,
                        false,
                    )
                    .await
                }
            },
            async {
                if responses_function_tools.support == ProviderFeatureSupport::Supported {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Supported,
                        detail: None,
                    }
                } else {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/responses",
                        responses_payload,
                        false,
                    )
                    .await
                }
            },
            async {
                if chat_function_tools.support == ProviderFeatureSupport::Supported {
                    self.probe_openai_function_tool_roundtrip(
                        &probe_client,
                        "/chat/completions",
                        chat_streaming_tools_payload,
                        TOOL_PROBE_TOKEN,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(
                            "streaming tools were not probed because the portable Chat function contract failed"
                                .to_string(),
                        ),
                    }
                }
            },
            async {
                if probe_enhanced_features {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/chat/completions",
                        chat_json_schema_output_payload,
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: None,
                    }
                }
            },
            async {
                if chat_function_tools.support == ProviderFeatureSupport::Supported {
                    self.probe_openai_chat_message_protocol(
                        &probe_client,
                        chat_message_protocol_payload,
                    )
                    .await
                } else {
                    OpenAiMessageProtocolProbeOutcome {
                        capabilities: known_chat_message_protocol,
                        detail: Some(
                            "assistant-message reasoning replay was not probed because the portable Chat function contract failed"
                                .to_string(),
                        ),
                    }
                }
            },
            async {
                if probe_enhanced_features {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/responses",
                        responses_native_tools_payload,
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(
                            "hosted Responses tools are not assumed for third-party relays"
                                .to_string(),
                        ),
                    }
                }
            },
            async {
                if responses_function_tools.support == ProviderFeatureSupport::Supported {
                    self.probe_openai_function_tool_roundtrip(
                        &probe_client,
                        "/responses",
                        responses_streaming_tools_payload,
                        TOOL_PROBE_TOKEN,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(
                            "streaming tools were not probed because the portable Responses function contract failed"
                                .to_string(),
                        ),
                    }
                }
            },
            async {
                if probe_enhanced_features {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/responses",
                        responses_json_schema_output_payload,
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: None,
                    }
                }
            },
            async {
                if probe_enhanced_features {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/responses",
                        responses_custom_tools_payload,
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: None,
                    }
                }
            },
            async {
                if probe_enhanced_features {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/responses",
                        responses_apply_patch_payload,
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: None,
                    }
                }
            },
            async {
                if probe_developer_in_main_batch {
                    self.probe_openai_endpoint(
                        &probe_client,
                        "/chat/completions",
                        developer_payload.clone(),
                        true,
                    )
                    .await
                } else {
                    OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: None,
                    }
                }
            },
        );
        if chat_function_tools.support == ProviderFeatureSupport::Supported {
            chat.support = ProviderFeatureSupport::Supported;
        }
        if responses_function_tools.support == ProviderFeatureSupport::Supported {
            responses.support = ProviderFeatureSupport::Supported;
        }
        // The streaming probe already carries the production parallel hint.
        // Reusing its outcome avoids two extra RPM-consuming requests and is
        // conservative if the combined envelope fails.
        let chat_parallel_tool_calls = chat_streaming_tools.clone();
        let responses_parallel_tool_calls = responses_streaming_tools.clone();

        let developer = if probe_developer_in_main_batch {
            early_developer
        } else if chat.support == ProviderFeatureSupport::Supported {
            self.probe_openai_endpoint(&probe_client, "/chat/completions", developer_payload, true)
                .await
        } else {
            OpenAiProbeOutcome {
                support: ProviderFeatureSupport::Unknown,
                detail: Some(
                    "developer messages were not probed because Chat Completions was unavailable"
                        .to_string(),
                ),
            }
        };

        // Selecting `/responses` from a bare text request is unsafe for relay
        // endpoints: some gateways accept the request and later translate it to
        // Chat Completions, but lose fields during a real agent turn. Route on
        // completed function-tool contracts and keep hosted web search as an
        // independent optional capability.
        let chat_agent_compatible = chat.support == ProviderFeatureSupport::Supported
            && chat_function_tools.support == ProviderFeatureSupport::Supported;
        let responses_agent_compatible = responses.support == ProviderFeatureSupport::Supported
            && responses_function_tools.support == ProviderFeatureSupport::Supported;
        let fallback_protocol = if chat_agent_compatible {
            OpenAiProtocol::ChatCompletions
        } else if responses_agent_compatible {
            OpenAiProtocol::Responses
        } else if chat.support == ProviderFeatureSupport::Supported {
            OpenAiProtocol::ChatCompletions
        } else if responses.support == ProviderFeatureSupport::Supported {
            OpenAiProtocol::Responses
        } else if preferred_adapter == ProviderAdapterKind::OpenAiResponses {
            OpenAiProtocol::Responses
        } else {
            OpenAiProtocol::ChatCompletions
        };
        let message_compatibility = developer.support != ProviderFeatureSupport::Supported;
        let mut notes = Vec::new();
        if let Some(detail) = chat.detail {
            notes.push(format!("Chat Completions: {detail}"));
        }
        if let Some(detail) = chat_function_tools.detail {
            notes.push(format!("Chat function tools: {detail}"));
        }
        if let Some(detail) = chat_strict_function_tools.detail {
            notes.push(format!("Chat strict function tools: {detail}"));
        }
        if let Some(detail) = chat_streaming_tools.detail {
            notes.push(format!("Chat streaming tools: {detail}"));
        }
        if let Some(detail) = chat_parallel_tool_calls.detail {
            notes.push(format!("Chat parallel tool calls: {detail}"));
        }
        if let Some(detail) = chat_json_schema_output.detail {
            notes.push(format!("Chat JSON Schema output: {detail}"));
        }
        if let Some(detail) = chat_message_protocol.detail {
            notes.push(format!("Chat assistant-message protocol: {detail}"));
        }
        if let Some(detail) = responses.detail {
            notes.push(format!("Responses: {detail}"));
        }
        if let Some(detail) = responses_native_tools.detail {
            notes.push(format!("Responses native tools: {detail}"));
        }
        if let Some(detail) = responses_function_tools.detail {
            notes.push(format!("Responses function tools: {detail}"));
        }
        if let Some(detail) = responses_strict_function_tools.detail {
            notes.push(format!("Responses strict function tools: {detail}"));
        }
        if let Some(detail) = responses_streaming_tools.detail {
            notes.push(format!("Responses streaming tools: {detail}"));
        }
        if let Some(detail) = responses_parallel_tool_calls.detail {
            notes.push(format!("Responses parallel tool calls: {detail}"));
        }
        if let Some(detail) = responses_json_schema_output.detail {
            notes.push(format!("Responses JSON Schema output: {detail}"));
        }
        if let Some(detail) = responses_custom_tools.detail {
            notes.push(format!("Responses custom tools: {detail}"));
        }
        if let Some(detail) = responses_apply_patch.detail {
            notes.push(format!("Responses apply_patch: {detail}"));
        }
        if let Some(detail) = developer.detail {
            notes.push(format!("developer messages: {detail}"));
        }
        if message_compatibility {
            notes.push(
                "Compatibility mode enabled: developer instructions and structured tool history will be flattened before sending."
                    .to_string(),
            );
        }

        let tool_probe_failure_detail = Self::summarize_function_tool_probe_failures(&notes);
        let mut report = OpenAiCompatibilityReport {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            selected_protocol: fallback_protocol,
            chat_completions: chat.support,
            chat_function_tools: chat_function_tools.support,
            chat_strict_function_tools: chat_strict_function_tools.support,
            chat_streaming_tools: chat_streaming_tools.support,
            chat_parallel_tool_calls: chat_parallel_tool_calls.support,
            chat_json_schema_output: chat_json_schema_output.support,
            chat_message_protocol: known_chat_message_protocol
                .union(chat_message_protocol.capabilities),
            chat_reasoning_protocol: chat_reasoning.protocol,
            responses: responses.support,
            responses_native_tools: responses_native_tools.support,
            responses_function_tools: responses_function_tools.support,
            responses_strict_function_tools: responses_strict_function_tools.support,
            responses_streaming_tools: responses_streaming_tools.support,
            responses_parallel_tool_calls: responses_parallel_tool_calls.support,
            responses_json_schema_output: responses_json_schema_output.support,
            responses_custom_tools: responses_custom_tools.support,
            responses_apply_patch: responses_apply_patch.support,
            responses_reasoning_protocol: responses_reasoning.protocol,
            developer_messages: developer.support,
            message_compatibility,
            checked_at: Utc::now(),
            notes,
        };
        let selected_protocol = report.recommended_protocol().unwrap_or(fallback_protocol);
        report.selected_protocol = selected_protocol;
        let reachable = chat.support == ProviderFeatureSupport::Supported
            || responses.support == ProviderFeatureSupport::Supported;
        let model_available = match selected_protocol {
            OpenAiProtocol::ChatCompletions => chat_agent_compatible,
            OpenAiProtocol::Responses => responses_agent_compatible,
        };
        Ok(ProviderHealthCheck {
            reachable,
            latency_ms: Some(start.elapsed().as_millis() as u64),
            model_available,
            error: (!model_available).then(|| {
                if reachable {
                    let summary = format!(
                        "model '{}' reached the endpoint, but no adapter completed the required function-tool capability round trip",
                        self.model.trim()
                    );
                    tool_probe_failure_detail
                        .as_deref()
                        .map(|detail| format!("{summary}: {detail}"))
                        .unwrap_or(summary)
                } else {
                    let detail = report
                        .notes
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "no supported OpenAI endpoint was detected".to_string());
                    format!("model '{}': {detail}", self.model.trim())
                }
            }),
            openai_compatibility: Some(report),
        })
    }

    /// Returns the bounded, user-actionable portion of a failed adapter
    /// negotiation. A connection test used to retain these notes in the
    /// compatibility report but discard them from the error shown during
    /// model discovery, leaving a generic failure with no way to diagnose a
    /// relay's tool-call incompatibility.
    fn summarize_function_tool_probe_failures(notes: &[String]) -> Option<String> {
        const TOOL_PROBE_PREFIXES: [&str; 2] =
            ["Chat function tools:", "Responses function tools:"];
        const MAX_DETAILS: usize = 2;
        const MAX_CHARS: usize = 1_500;

        let details = notes
            .iter()
            .filter(|note| {
                TOOL_PROBE_PREFIXES
                    .iter()
                    .any(|prefix| note.starts_with(prefix))
            })
            .take(MAX_DETAILS)
            .map(|note| note.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>();
        if details.is_empty() {
            return None;
        }

        let summary = details.join("; ");
        if summary.chars().count() <= MAX_CHARS {
            return Some(summary);
        }
        Some(format!(
            "{}…",
            summary.chars().take(MAX_CHARS - 1).collect::<String>()
        ))
    }

    async fn probe_openai_chat_message_protocol(
        &self,
        probe_client: &OpenAiProbeClient,
        payload: Value,
    ) -> OpenAiMessageProtocolProbeOutcome {
        let fallback =
            trusted_chat_message_protocol_contract(&self.base_url, &self.model).unwrap_or_default();
        let response = match probe_client.send("/chat/completions", &payload).await {
            Ok(response) => response,
            Err(error) => {
                return OpenAiMessageProtocolProbeOutcome {
                    capabilities: fallback,
                    detail: Some(error),
                }
            }
        };
        let (response, _probe_permit) = response.into_parts();
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return OpenAiMessageProtocolProbeOutcome {
                capabilities: fallback,
                detail: Some(format!(
                    "HTTP {}: {}",
                    status.as_u16(),
                    truncate_observation_text(body.trim())
                )),
            };
        }
        let body = match response.json::<Value>().await {
            Ok(body) => body,
            Err(error) => {
                return OpenAiMessageProtocolProbeOutcome {
                    capabilities: fallback,
                    detail: Some(format!("invalid JSON response: {error}")),
                }
            }
        };
        let has_tool_calls = body
            .pointer("/choices/0/message/tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|calls| !calls.is_empty());
        let exposes_reasoning_content = body
            .pointer("/choices/0/message/reasoning_content")
            .and_then(Value::as_str)
            .is_some();
        let observed = ProviderMessageProtocolCapabilities {
            // Replaying a provider-owned extension field is safe even when the
            // endpoint merely accepts rather than strictly requires it. This
            // conservative contract also covers relays with opaque model ids.
            requires_reasoning_content_for_tool_calls: has_tool_calls && exposes_reasoning_content,
        };
        OpenAiMessageProtocolProbeOutcome {
            capabilities: fallback.union(observed),
            detail: (!has_tool_calls).then(|| {
                "HTTP 200 did not return a tool call; reasoning replay was not inferred".to_string()
            }),
        }
    }

    async fn probe_openai_endpoint(
        &self,
        probe_client: &OpenAiProbeClient,
        path: &str,
        payload: Value,
        role_probe: bool,
    ) -> OpenAiProbeOutcome {
        let response = match probe_client.send(path, &payload).await {
            Ok(response) => response,
            Err(error) => {
                return OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Unknown,
                    detail: Some(error),
                }
            }
        };
        let (response, _probe_permit) = response.into_parts();
        let status = response.status();
        if status.is_success() {
            return OpenAiProbeOutcome {
                support: ProviderFeatureSupport::Supported,
                detail: None,
            };
        }
        let body = response.text().await.unwrap_or_default();
        let support = if role_probe && matches!(status.as_u16(), 400 | 422) {
            ProviderFeatureSupport::Unsupported
        } else if matches!(status.as_u16(), 404 | 405 | 501) {
            ProviderFeatureSupport::Unsupported
        } else {
            ProviderFeatureSupport::Unknown
        };
        OpenAiProbeOutcome {
            support,
            detail: Some(format!(
                "HTTP {}: {}",
                status.as_u16(),
                truncate_observation_text(body.trim())
            )),
        }
    }

    async fn probe_openai_function_tool_roundtrip(
        &self,
        probe_client: &OpenAiProbeClient,
        path: &str,
        payload: Value,
        expected_token: &str,
    ) -> OpenAiProbeOutcome {
        let mut embedded_rate_limit_retry = 0;
        loop {
            let response = match probe_client.send(path, &payload).await {
                Ok(response) => response,
                Err(error) => {
                    return OpenAiProbeOutcome {
                        support: ProviderFeatureSupport::Unknown,
                        detail: Some(error),
                    }
                }
            };
            let (response, _probe_permit) = response.into_parts();
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
            let streamed = payload
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let candidate = compatibility_probe_candidate(expected_token);
            let candidates = [candidate];
            let mut ignore_delta = |_| Ok(());
            let decoded = tokio::time::timeout(Duration::from_secs(20), async {
                if path == "/responses" {
                    decode_openai_responses_response(
                        response,
                        streamed,
                        &candidates,
                        &mut ignore_delta,
                        false,
                    )
                    .await
                } else {
                    decode_openai_chat_response(
                        response,
                        streamed,
                        &candidates,
                        &mut ignore_delta,
                        false,
                    )
                    .await
                }
            })
            .await;
            let parsed = match decoded {
                Ok(Ok(response)) => validate_tool_probe_response(&response, expected_token),
                Ok(Err(error)) => {
                    if let Some(rate_limit) = stream::provider_stream_rate_limit(&error) {
                        embedded_rate_limit_retry += 1;
                        if probe_client
                            .schedule_embedded_rate_limit_retry(
                                embedded_rate_limit_retry,
                                rate_limit.retry_after(),
                            )
                            .await
                        {
                            continue;
                        }
                    }
                    Err(error.to_string())
                }
                Err(_) => {
                    Err("HTTP 200 tool response did not complete within 20 seconds".to_string())
                }
            };
            return match parsed {
                Ok(()) => OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Supported,
                    detail: None,
                },
                Err(detail) => OpenAiProbeOutcome {
                    support: ProviderFeatureSupport::Unsupported,
                    detail: Some(detail),
                },
            };
        }
    }

    async fn probe_reasoning_protocol(
        &self,
        probe_client: &OpenAiProbeClient,
        adapter: ProviderAdapterKind,
        path: &str,
        portable_tool_payload: Value,
        expected_token: &str,
    ) -> OpenAiReasoningProbeOutcome {
        let mut attempts = Vec::new();
        let mut final_outcome = OpenAiProbeOutcome {
            support: ProviderFeatureSupport::Unknown,
            detail: Some("no reasoning protocol candidates were available".to_string()),
        };
        for protocol in reasoning_probe_candidates(adapter, &self.model) {
            let mut payload = portable_tool_payload.clone();
            self.apply_probe_reasoning_options(protocol, &mut payload);
            let outcome = self
                .probe_openai_function_tool_roundtrip(probe_client, path, payload, expected_token)
                .await;
            if outcome.support == ProviderFeatureSupport::Supported {
                return OpenAiReasoningProbeOutcome {
                    protocol: Some(protocol),
                    tool_support: outcome,
                };
            }
            if let Some(detail) = outcome.detail.as_deref() {
                attempts.push(format!("{}: {detail}", reasoning_protocol_label(protocol)));
            }
            let should_stop = outcome.support == ProviderFeatureSupport::Unknown;
            final_outcome = outcome;
            // Unknown normally means transport/rate-limit/server failure. More
            // candidate requests cannot establish a protocol and would amplify
            // load on an already unhealthy endpoint.
            if should_stop {
                break;
            }
        }
        if !attempts.is_empty() {
            final_outcome.detail = Some(attempts.join("; "));
        }
        OpenAiReasoningProbeOutcome {
            protocol: None,
            tool_support: final_outcome,
        }
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.temperature = Some(0.0);
        self.max_output_tokens = Some(self.max_output_tokens.unwrap_or(1_024).min(1_024));
        if self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("low".to_string());
        }
        self.parallel_tool_calls = true;
        self
    }

    fn prepare_chat_request(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        ensure_visual_input_supported(&request, self.supports_vision)?;
        if !request.tool_candidates.is_empty() {
            require_function_tools("OpenAI Chat Completions", self.tool_protocol)?;
        }
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let thinking_enabled = self
            .reasoning_effort
            .as_deref()
            .is_none_or(|effort| effort != "none");
        let mut messages = self.chat_codec.encode_messages(&request, thinking_enabled);
        if self.output_protocol.json_schema != ProviderFeatureSupport::Supported {
            if let Some(schema) = request.final_output_json_schema.as_ref() {
                messages.push(json!({
                    "role": "system",
                    "content": textual_json_schema_output_instruction(schema),
                }));
            }
        }
        let wire_transcript = ProviderWireTranscript {
            format: self.chat_codec.transcript_format().to_string(),
            items: messages.clone(),
        };
        let tool_capable = !request.tool_candidates.is_empty();
        let compiled_tools = compile_openai_tools(&request.tool_candidates, self.tool_protocol);
        let stream = !tool_capable
            || self.tool_protocol.streaming_tools == ProviderFeatureSupport::Supported;
        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": stream
        });
        if stream {
            payload["stream_options"] = json!({ "include_usage": true });
        }
        // Reasoning models reject any explicit temperature with a 400, so the
        // field is omitted rather than clamped.
        let applied_reasoning = self.apply_chat_reasoning_options(&mut payload);
        if !applied_reasoning.omit_temperature {
            if let Some(temperature) = self.temperature {
                payload["temperature"] = json!(temperature);
            }
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            payload["max_tokens"] = json!(max_output_tokens);
        }
        if !request.tool_candidates.is_empty() {
            payload["tools"] = json!(compiled_tools.tools);
            if !applied_reasoning.omit_tool_choice {
                payload["tool_choice"] = json!("auto");
            }
            if self.tool_protocol.parallel_tool_calls == ProviderFeatureSupport::Supported {
                payload["parallel_tool_calls"] = json!(self.parallel_tool_calls);
            }
        }
        if is_official_openai_endpoint(&self.base_url) {
            if let Some(prompt_cache_key) = request
                .instructions
                .prompt_cache_key
                .as_deref()
                .or(self.prompt_cache_key.as_deref())
                .filter(|value| !value.is_empty())
            {
                payload["prompt_cache_key"] = json!(prompt_cache_key);
            }
        }
        if self.output_protocol.json_schema == ProviderFeatureSupport::Supported {
            if let Some(schema) = request.final_output_json_schema.as_ref() {
                payload["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "guardian_assessment",
                        "strict": true,
                        "schema": schema,
                    }
                });
            }
        }

        Ok(PreparedProviderRequest {
            request_id,
            adapter: "openai_chat_completions".to_string(),
            method: "POST".to_string(),
            endpoint: url,
            observation_body: redact_transport_value(&payload),
            cache_trace: crate::build_provider_cache_trace(&payload, None, false),
            body: payload,
            logical_request: request,
            wire_transcript: Some(wire_transcript),
            tool_contracts: compiled_tools.contracts,
            response_commit: if tool_capable {
                ProviderResponseCommitMode::Atomic
            } else {
                ProviderResponseCommitMode::Streaming
            },
        })
    }
}

pub(super) fn is_tool_call_protocol_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .to_string()
            .contains("provider tool-call protocol error")
    })
}

pub(super) async fn decode_openai_chat_response(
    mut response: reqwest::Response,
    streamed: bool,
    tool_candidates: &[ProviderToolCandidate],
    on_delta: &mut ModelStreamCallback<'_>,
    emit_nonstream_deltas: bool,
) -> anyhow::Result<ModelResponse> {
    if !streamed {
        let body: Value = response.json().await?;
        let response = validate_provider_response_protocol(parse_model_response_body_with_tools(
            &body,
            tool_candidates,
        )?)?;
        if emit_nonstream_deltas {
            for reasoning in response.provider_items.iter().filter_map(|item| {
                (item.get("type").and_then(Value::as_str) == Some(OPENAI_CHAT_ASSISTANT_STATE_TYPE))
                    .then(|| item.get("reasoning_content").and_then(Value::as_str))
                    .flatten()
            }) {
                on_delta(ModelStreamDelta::Reasoning {
                    text: reasoning.to_string(),
                })?;
            }
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
            if let Some(usage) = response.usage.clone() {
                on_delta(ModelStreamDelta::Usage { usage })?;
            }
        }
        return Ok(response);
    }

    let mut decoder = SseDecoder::default();
    let mut accumulator = OpenAiStreamAccumulator::default();
    let idle_timeout = stream_idle_timeout();
    loop {
        let Some(chunk) = next_stream_chunk(&mut response, idle_timeout).await? else {
            break;
        };
        for data in decoder.push(&chunk)? {
            if data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(&data)
                .map_err(|err| anyhow::anyhow!("invalid provider SSE data: {err}: {data}"))?;
            accumulator.apply(&event, on_delta)?;
        }
    }
    for data in decoder.finish()? {
        if data != "[DONE]" {
            let event: Value = serde_json::from_str(&data)
                .map_err(|err| anyhow::anyhow!("invalid provider SSE data: {err}: {data}"))?;
            accumulator.apply(&event, on_delta)?;
        }
    }
    validate_provider_response_protocol(accumulator.finish_with_tools(tool_candidates)?)
}

pub(super) fn validate_provider_response_protocol(
    response: ModelResponse,
) -> anyhow::Result<ModelResponse> {
    if response.finish_reason == ModelFinishReason::ToolCalls && response.tool_calls.is_empty() {
        anyhow::bail!(
            "provider tool-call protocol error: provider reported a tool-call finish but returned no structured tool call"
        );
    }
    Ok(response)
}

pub(super) fn emit_response_deltas(
    response: &ModelResponse,
    on_delta: &mut ModelStreamCallback<'_>,
) -> anyhow::Result<()> {
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
    if let Some(usage) = response.usage.clone() {
        on_delta(ModelStreamDelta::Usage { usage })?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct OpenAiResponsesProvider {
    pub(in crate::provider) client: reqwest::Client,
    pub(in crate::provider) base_url: String,
    pub(in crate::provider) api_key: String,
    pub(in crate::provider) auth: ProviderAuthKind,
    pub(in crate::provider) model: String,
    pub(in crate::provider) temperature: Option<f64>,
    pub(in crate::provider) max_output_tokens: Option<u32>,
    pub(in crate::provider) reasoning_effort: Option<String>,
    pub(in crate::provider) reasoning_protocol: ProviderReasoningProtocol,
    pub(in crate::provider) store_responses: bool,
    pub(in crate::provider) parallel_tool_calls: bool,
    pub(in crate::provider) prompt_cache_key: Option<String>,
    pub(in crate::provider) prompt_cache_policy: Option<PromptCachePolicy>,
    pub(in crate::provider) compaction_threshold_tokens: Option<u32>,
    pub(in crate::provider) native_web_search: bool,
    pub(in crate::provider) supports_vision: bool,
    pub(in crate::provider) output_protocol: ProviderOutputProtocolCapabilities,
    pub(in crate::provider) tool_protocol: ProviderToolProtocolCapabilities,
}

#[derive(Debug, thiserror::Error)]
#[error("provider request failed ({status}): {body}")]
pub(super) struct ResponsesRequestError {
    status: reqwest::StatusCode,
    body: String,
}

pub(super) const NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION: &str = "When web search is needed, prefer the provider's built-in web search tool. Use a supplied search tool only if built-in search is unavailable, fails, or the required source exists only through that tool. Do not run both for the same query unless fallback is necessary.";

impl ResponsesRequestError {
    fn invalid_previous_response(&self, response_id: &str) -> bool {
        if !matches!(self.status.as_u16(), 400 | 404) {
            return false;
        }
        let body = self.body.to_ascii_lowercase();
        body.contains("previous_response_id")
            || body.contains("previous response")
            || body.contains(&response_id.to_ascii_lowercase())
    }
}

pub(super) fn responses_request_uses_enhanced_tools(body: &Value) -> bool {
    body.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                matches!(
                    tool.get("type").and_then(Value::as_str),
                    Some("custom") | Some("apply_patch") | Some("namespace") | Some("tool_search")
                ) || tool.get("defer_loading").and_then(Value::as_bool) == Some(true)
            })
        })
}

pub(super) fn request_uses_strict_function_tools(body: &Value) -> bool {
    body.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("strict").and_then(Value::as_bool) == Some(true)
                    || tool
                        .get("function")
                        .and_then(|function| function.get("strict"))
                        .and_then(Value::as_bool)
                        == Some(true)
            })
        })
}

pub(super) fn provider_rejected_strict_function_tools(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("strict")
        && [
            "unsupported",
            "not supported",
            "unknown field",
            "unknown parameter",
            "unrecognized",
            "invalid schema",
            "invalid tool",
            "invalid value",
        ]
        .iter()
        .any(|marker| body.contains(marker))
}

pub(super) fn provider_rejected_tool_representation(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    [
        "unsupported",
        "not supported",
        "unknown tool",
        "unknown field",
        "unknown parameter",
        "unrecognized",
        "invalid tool",
        "invalid value",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}

impl OpenAiResponsesProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        let base_url = base_url.into();
        let model = model.into();
        let native_tool_search = official_openai_tool_search_support(&base_url, &model);
        let native_web_search = is_official_openai_endpoint(&base_url);
        let reasoning_protocol =
            default_reasoning_protocol(ProviderAdapterKind::OpenAiResponses, &model);
        Self {
            client: reqwest::Client::new(),
            base_url,
            api_key: api_key.into(),
            auth: ProviderAuthKind::Bearer,
            model,
            temperature: None,
            max_output_tokens: None,
            reasoning_effort: None,
            reasoning_protocol,
            store_responses: false,
            parallel_tool_calls: true,
            prompt_cache_key: None,
            prompt_cache_policy: None,
            compaction_threshold_tokens: None,
            native_web_search,
            supports_vision: true,
            output_protocol: ProviderOutputProtocolCapabilities::default(),
            tool_protocol: ProviderToolProtocolCapabilities {
                function_tools: ProviderFeatureSupport::Supported,
                deferred_tool_loading: native_tool_search,
                namespace_tools: native_tool_search,
                hosted_tool_search: native_tool_search,
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
        provider.store_responses = settings.store_responses;
        provider.parallel_tool_calls = settings.parallel_tool_calls;
        provider.prompt_cache_key = settings.prompt_cache_key.clone();
        provider.prompt_cache_policy = settings.prompt_cache_policy;
        provider.compaction_threshold_tokens = settings.responses_compaction_threshold_tokens;
        provider.supports_vision = settings.supports_vision_for_model();
        provider.tool_protocol = settings
            .capabilities_for_adapter(ProviderAdapterKind::OpenAiResponses)
            .tool_protocol;
        provider.native_web_search = is_official_openai_endpoint(&provider.base_url)
            || provider.tool_protocol.hosted_web_search == ProviderFeatureSupport::Supported;
        if let Some(profile) = settings.adapter_profile_for_model_and_adapter(
            &settings.model,
            ProviderAdapterKind::OpenAiResponses,
        ) {
            provider.reasoning_protocol = profile.reasoning_protocol;
            provider.output_protocol = profile.output_protocol;
        }
        Some(provider)
    }

    pub(crate) fn for_guardian(mut self) -> Self {
        self.temperature = Some(0.0);
        self.max_output_tokens = Some(self.max_output_tokens.unwrap_or(1_024).min(1_024));
        if self.reasoning_effort.is_some() {
            self.reasoning_effort = Some("low".to_string());
        }
        self.parallel_tool_calls = true;
        self.native_web_search = false;
        self
    }

    fn prepare_responses_request(
        &self,
        request_id: Uuid,
        request: ModelRequest,
    ) -> anyhow::Result<PreparedProviderRequest> {
        ensure_visual_input_supported(&request, self.supports_vision)?;
        let compiled_tools = compile_responses_tools(&request.tool_candidates, self.tool_protocol);
        let requires_function_tools = compiled_tools
            .tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some("function"));
        if requires_function_tools {
            require_function_tools("OpenAI Responses", self.tool_protocol)?;
        }
        let endpoint = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let tool_capable = self.native_web_search || !request.tool_candidates.is_empty();
        let stream = !tool_capable
            || self.tool_protocol.streaming_tools == ProviderFeatureSupport::Supported;
        let mut payload = json!({
            "model": self.model,
            "input": responses_input(&request),
            "stream": stream,
            "store": self.store_responses,
        });
        if self.tool_protocol.parallel_tool_calls == ProviderFeatureSupport::Supported {
            payload["parallel_tool_calls"] = json!(self.parallel_tool_calls);
        }
        // Reasoning models reject any explicit temperature with a 400, so the
        // field is omitted rather than clamped. `None` also omits it — letting
        // the model use its vendor default.
        let applied_reasoning = apply_reasoning_protocol(
            self.reasoning_protocol,
            self.reasoning_effort.as_deref(),
            None,
            &mut payload,
        );
        if !applied_reasoning.omit_temperature {
            if let Some(temperature) = self.temperature {
                payload["temperature"] = json!(temperature);
            }
        }
        let mut system_instructions = responses_system_instructions(&request);
        if self.output_protocol.json_schema != ProviderFeatureSupport::Supported {
            if let Some(schema) = request.final_output_json_schema.as_ref() {
                if !system_instructions.trim().is_empty() {
                    system_instructions.push_str("\n\n");
                }
                system_instructions.push_str(&textual_json_schema_output_instruction(schema));
            }
        }
        if self.native_web_search {
            if !system_instructions.trim().is_empty() {
                system_instructions.push_str("\n\n");
            }
            system_instructions.push_str(NATIVE_WEB_SEARCH_PRIORITY_INSTRUCTION);
        }
        if !system_instructions.trim().is_empty() {
            payload["instructions"] = json!(system_instructions);
        }
        if let Some(previous_response_id) = request
            .previous_response_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            payload["previous_response_id"] = json!(previous_response_id);
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            payload["max_output_tokens"] = json!(max_output_tokens);
        }
        if applied_reasoning.enabled
            && !self.store_responses
            && is_official_openai_endpoint(&self.base_url)
        {
            payload["include"] = json!(["reasoning.encrypted_content"]);
        }
        let mut tools = Vec::new();
        if self.native_web_search {
            tools.push(json!({ "type": "web_search" }));
        }
        tools.extend(compiled_tools.tools.iter().cloned());
        if !tools.is_empty() {
            payload["tools"] = json!(tools);
            payload["tool_choice"] = json!("auto");
        }
        if is_official_openai_endpoint(&self.base_url) {
            if let Some(prompt_cache_key) = request
                .instructions
                .prompt_cache_key
                .as_deref()
                .or(self.prompt_cache_key.as_deref())
                .filter(|value| !value.is_empty())
            {
                payload["prompt_cache_key"] = json!(prompt_cache_key);
            }
        }
        match self.prompt_cache_policy {
            Some(PromptCachePolicy::Explicit30m)
                if official_openai_explicit_prompt_cache_support(&self.base_url, &self.model)
                    == ProviderFeatureSupport::Supported =>
            {
                payload["prompt_cache_options"] = json!({
                    "mode": "explicit",
                    "ttl": "30m",
                });
                add_responses_prompt_cache_breakpoint(&mut payload["input"], &request);
            }
            Some(PromptCachePolicy::Explicit30m) => {
                // Older official models and unprobed compatible relays may
                // reject explicit-only fields. Omitting them preserves the
                // provider's implicit cache behavior without a failed call.
            }
            Some(PromptCachePolicy::LegacyInMemory) => {
                payload["prompt_cache_retention"] = json!("in_memory");
            }
            Some(PromptCachePolicy::Legacy24h) => {
                payload["prompt_cache_retention"] = json!("24h");
            }
            None => {}
        }
        if let Some(threshold) = self.compaction_threshold_tokens.filter(|value| *value > 0) {
            payload["context_management"] = json!([{
                "type": "compaction",
                "compact_threshold": threshold,
            }]);
        }
        if self.output_protocol.json_schema == ProviderFeatureSupport::Supported {
            if let Some(schema) = request.final_output_json_schema.as_ref() {
                payload["text"] = json!({
                    "format": {
                        "type": "json_schema",
                        "name": "guardian_assessment",
                        "strict": true,
                        "schema": schema,
                    }
                });
            }
        }

        Ok(PreparedProviderRequest {
            request_id,
            adapter: "openai_responses".to_string(),
            method: "POST".to_string(),
            endpoint,
            observation_body: redact_transport_value(&payload),
            cache_trace: crate::build_provider_cache_trace(&payload, None, false),
            body: payload,
            logical_request: request,
            wire_transcript: None,
            tool_contracts: compiled_tools.contracts,
            response_commit: if tool_capable {
                ProviderResponseCommitMode::Atomic
            } else {
                ProviderResponseCommitMode::Streaming
            },
        })
    }
}

fn textual_json_schema_output_instruction(schema: &Value) -> String {
    format!(
        "Return only one JSON value that conforms exactly to this JSON Schema. Do not wrap it in Markdown or add explanatory text.\n<output_json_schema>\n{}\n</output_json_schema>",
        schema
    )
}

pub(super) async fn decode_openai_responses_response(
    mut response: reqwest::Response,
    streamed: bool,
    tool_candidates: &[ProviderToolCandidate],
    on_delta: &mut ModelStreamCallback<'_>,
    emit_nonstream_deltas: bool,
) -> anyhow::Result<ModelResponse> {
    if !streamed {
        let body: Value = response.json().await?;
        let response = validate_provider_response_protocol(parse_model_response_body_with_tools(
            &body,
            tool_candidates,
        )?)?;
        if emit_nonstream_deltas {
            emit_response_deltas(&response, on_delta)?;
        }
        return Ok(response);
    }

    let mut decoder = SseDecoder::default();
    let mut accumulator = ResponsesStreamAccumulator::default();
    let idle_timeout = stream_idle_timeout();
    loop {
        let Some(chunk) = next_stream_chunk(&mut response, idle_timeout).await? else {
            break;
        };
        for data in decoder.push(&chunk)? {
            if data == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(&data)
                .map_err(|err| anyhow::anyhow!("invalid Responses SSE data: {err}: {data}"))?;
            accumulator.apply(&event, on_delta)?;
        }
    }
    for data in decoder.finish()? {
        if data != "[DONE]" {
            let event: Value = serde_json::from_str(&data)
                .map_err(|err| anyhow::anyhow!("invalid Responses SSE data: {err}: {data}"))?;
            accumulator.apply(&event, on_delta)?;
        }
    }
    validate_provider_response_protocol(accumulator.finish_with_tools(tool_candidates)?)
}

// Protocol codec implementation is defined in the child modules above.
#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
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
        self.prepare_chat_request(request_id, request)
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
        self.execute_chat_request(prepared, on_delta, on_transport)
            .await
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();

        let models_url = format!("{}/models", self.base_url.trim_end_matches('/'));
        match tokio::time::timeout(
            Duration::from_secs(5),
            apply_provider_auth(self.client.get(&models_url), self.auth, &self.api_key).send(),
        )
        .await
        {
            Ok(Ok(response)) => {
                let latency = start.elapsed().as_millis() as u64;
                let reachable = response.status().is_success();
                Ok(ProviderHealthCheck {
                    reachable,
                    latency_ms: Some(latency),
                    model_available: reachable,
                    error: if reachable {
                        None
                    } else {
                        Some(format!("HTTP {}", response.status()))
                    },
                    openai_compatibility: None,
                })
            }
            Ok(Err(_)) => {
                let chat_url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    apply_provider_auth(self.client.post(&chat_url), self.auth, &self.api_key)
                        .header(CONTENT_TYPE, "application/json")
                        .json(&json!({
                            "model": self.model,
                            "messages": [{"role": "user", "content": "hi"}],
                            "max_tokens": 1
                        }))
                        .send(),
                )
                .await
                {
                    Ok(Ok(resp)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        let reachable = resp.status().is_success();
                        Ok(ProviderHealthCheck {
                            reachable,
                            latency_ms: Some(latency),
                            model_available: reachable,
                            error: if reachable {
                                None
                            } else {
                                Some(format!("HTTP {}", resp.status()))
                            },
                            openai_compatibility: None,
                        })
                    }
                    Ok(Err(err)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        Ok(ProviderHealthCheck {
                            reachable: false,
                            latency_ms: Some(latency),
                            model_available: false,
                            error: Some(err.to_string()),
                            openai_compatibility: None,
                        })
                    }
                    Err(_) => Ok(ProviderHealthCheck {
                        reachable: false,
                        latency_ms: None,
                        model_available: false,
                        error: Some("timeout".to_string()),
                        openai_compatibility: None,
                    }),
                }
            }
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

#[async_trait]
impl ModelProvider for OpenAiResponsesProvider {
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
        self.prepare_responses_request(request_id, request)
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
        let previous_response_id = prepared.logical_request.previous_response_id.clone();
        match self
            .execute_responses_request(prepared.clone(), 1, on_delta, on_transport)
            .await
        {
            Err(error)
                if previous_response_id.as_deref().is_some_and(|response_id| {
                    error
                        .downcast_ref::<ResponsesRequestError>()
                        .is_some_and(|error| error.invalid_previous_response(response_id))
                }) =>
            {
                let mut replay = prepared.logical_request;
                replay.previous_response_id = None;
                let replay = self.prepare_responses_request(prepared.request_id, replay)?;
                on_transport(ProviderTransportEvent::Retry {
                    attempt: 2,
                    retry_kind: ProviderRetryKind::StateRecovery,
                    retry_index: None,
                    retry_limit: None,
                    reason: "stored response cursor unavailable; replaying canonical local context"
                        .to_string(),
                    cache_trace: replay.cache_trace.clone(),
                    body: replay.observation_body.clone(),
                })?;
                self.execute_responses_request(replay, 2, on_delta, on_transport)
                    .await
            }
            Err(error) => {
                if let Some(response_error) = error.downcast_ref::<ResponsesRequestError>() {
                    if matches!(response_error.status.as_u16(), 400 | 404 | 422) {
                        if let Some(capability) = rejected_responses_profile_capability(
                            &prepared.body,
                            &response_error.body,
                        ) {
                            return Err(ProviderAdapterError::CapabilityProfileStale {
                                capability,
                                detail: response_error.body.clone(),
                            }
                            .into());
                        }
                    }
                }
                Err(error)
            }
            result => result,
        }
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        let start = std::time::Instant::now();
        let models_url = format!("{}/models", self.base_url.trim_end_matches('/'));
        match tokio::time::timeout(
            Duration::from_secs(5),
            apply_provider_auth(self.client.get(&models_url), self.auth, &self.api_key).send(),
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
