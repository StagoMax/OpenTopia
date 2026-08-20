use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Legacy provider preset identity. New runtime code resolves transport,
/// authentication, and adapter independently; this enum remains serialized for
/// one compatibility window so older desktop builds can still read settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Mock,
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
    #[serde(rename = "openai_responses", alias = "open_ai_responses")]
    OpenAiResponses,
    /// Delegate model execution and local file attachments to an installed
    /// Codex App Server instance.
    #[serde(rename = "codex_app_server")]
    CodexAppServer,
    /// Anthropic Messages API provider.
    #[serde(rename = "anthropic")]
    Anthropic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTransportKind {
    Http,
    CodexAppServer,
    Mock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthKind {
    Bearer,
    XApiKey,
    CodexSession,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiProtocol {
    ChatCompletions,
    Responses,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFeatureSupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

/// Wire protocol selected for one concrete endpoint/model pair. Connections
/// are credentials and routing; adapters are protocol codecs, so one relay may
/// legitimately select different adapters for different models.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAdapterKind {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    CodexAppServer,
    Mock,
}

impl ProviderAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "open_ai_chat",
            Self::OpenAiResponses => "open_ai_responses",
            Self::AnthropicMessages => "anthropic_messages",
            Self::CodexAppServer => "codex_app_server",
            Self::Mock => "mock",
        }
    }
}

/// Deterministic instruction lowering selected during capability negotiation.
/// The adapter reads this value while encoding; it never probes or changes it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderInstructionEncoding {
    NativeRoles,
    FoldDeveloperIntoSystem,
    PortableChatEnvelope,
}

/// Structural request envelope used to control model reasoning. Variant names
/// describe wire behavior rather than vendors or model families. Capability
/// negotiation owns this choice; request codecs never infer it from a model id.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReasoningProtocol {
    #[default]
    Omit,
    #[serde(alias = "reasoning_effort")]
    ChatReasoningEffort,
    #[serde(alias = "glm_thinking")]
    ChatThinkingReasoningEffort,
    #[serde(alias = "deep_seek_thinking", alias = "deepseek_thinking")]
    ChatThinkingHighMaxNoToolChoice,
    ResponsesReasoning,
}

pub const PROVIDER_ADAPTER_PROFILE_VERSION: u32 = 7;
// v6 is the first profile that persisted a reasoning envelope. Older profiles
// cannot be upgraded without guessing from a model name, so they deliberately
// expire and are renegotiated.
const MIN_MIGRATABLE_PROVIDER_ADAPTER_PROFILE_VERSION: u32 = 6;
#[cfg(test)]
pub(super) const PREVIOUS_PROVIDER_ADAPTER_PROFILE_VERSION: u32 =
    PROVIDER_ADAPTER_PROFILE_VERSION - 1;

/// Assistant-message constraints imposed by one concrete wire protocol. These
/// are negotiated or supplied by a trusted built-in endpoint contract; request
/// codecs consume the result without inspecting vendor or model names.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderMessageProtocolCapabilities {
    /// Every assistant message that contains tool calls must preserve the
    /// provider-issued `reasoning_content` field in subsequent requests.
    pub requires_reasoning_content_for_tool_calls: bool,
}

/// Structured final-output features exposed by one concrete wire protocol.
/// These are negotiated during provider setup and never inferred by retrying a
/// modified request after a live turn has already started.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderOutputProtocolCapabilities {
    pub json_schema: ProviderFeatureSupport,
}

impl ProviderMessageProtocolCapabilities {
    pub fn union(self, other: Self) -> Self {
        Self {
            requires_reasoning_content_for_tool_calls: self
                .requires_reasoning_content_for_tool_calls
                || other.requires_reasoning_content_for_tool_calls,
        }
    }
}

/// Normalized output of provider capability negotiation. Probe diagnostics may
/// remain provider-specific, but the runtime consumes only this stable adapter
/// contract and therefore never needs to reinterpret a probe response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAdapterProfile {
    pub profile_version: u32,
    pub base_url: String,
    pub model: String,
    pub adapter: ProviderAdapterKind,
    pub instruction_encoding: ProviderInstructionEncoding,
    #[serde(default)]
    pub reasoning_protocol: ProviderReasoningProtocol,
    #[serde(default)]
    pub message_protocol: ProviderMessageProtocolCapabilities,
    #[serde(default)]
    pub output_protocol: ProviderOutputProtocolCapabilities,
    #[serde(default)]
    pub tool_protocol: ProviderToolProtocolCapabilities,
    pub checked_at: DateTime<Utc>,
}

impl ProviderAdapterProfile {
    pub fn applies_to(&self, base_url: &str, model: &str) -> bool {
        self.profile_version == PROVIDER_ADAPTER_PROFILE_VERSION
            && self.matches_connection(base_url, model)
    }

    fn matches_connection(&self, base_url: &str, model: &str) -> bool {
        self.base_url.trim_end_matches('/') == base_url.trim_end_matches('/')
            && self.model.trim() == model.trim()
    }

    fn normalized_for(mut self, base_url: &str, model: &str) -> Option<Self> {
        if !self.matches_connection(base_url, model) {
            return None;
        }
        match self.profile_version {
            PROVIDER_ADAPTER_PROFILE_VERSION => Some(self),
            MIN_MIGRATABLE_PROVIDER_ADAPTER_PROFILE_VERSION..PROVIDER_ADAPTER_PROFILE_VERSION => {
                self.profile_version = PROVIDER_ADAPTER_PROFILE_VERSION;
                if self.adapter == ProviderAdapterKind::OpenAiResponses {
                    self.reasoning_protocol = ProviderReasoningProtocol::ResponsesReasoning;
                }
                if self.adapter == ProviderAdapterKind::OpenAiChat {
                    self.message_protocol = self
                        .message_protocol
                        .union(trusted_chat_message_protocol_contract(base_url, model)?);
                }
                Some(self)
            }
            _ => None,
        }
    }

    /// Scores only capabilities that were proven for this endpoint/model
    /// profile. The required function-tool contract is a gate, not a bonus.
    /// Optional protocol features then decide between otherwise viable
    /// adapters without coupling routing to a provider hostname.
    pub(crate) fn agent_capability_score(&self) -> Option<u16> {
        if self.tool_protocol.function_tools != ProviderFeatureSupport::Supported {
            return None;
        }
        let supported = |feature| u16::from(feature == ProviderFeatureSupport::Supported);
        Some(
            1_000
                + 100 * supported(self.tool_protocol.streaming_tools)
                + 20 * supported(self.tool_protocol.strict_function_tools)
                + 10 * supported(self.tool_protocol.parallel_tool_calls)
                + 8 * supported(self.output_protocol.json_schema)
                + 6 * supported(self.tool_protocol.freeform_tools)
                + 5 * supported(self.tool_protocol.hosted_apply_patch)
                + 4 * supported(self.tool_protocol.hosted_web_search)
                + 3 * supported(self.tool_protocol.hosted_tool_search)
                + 2 * supported(self.tool_protocol.deferred_tool_loading)
                + supported(self.tool_protocol.namespace_tools),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatibilityReport {
    pub base_url: String,
    pub model: String,
    pub selected_protocol: OpenAiProtocol,
    pub chat_completions: ProviderFeatureSupport,
    #[serde(default)]
    pub chat_function_tools: ProviderFeatureSupport,
    /// Chat Completions function tools with provider-enforced strict JSON
    /// Schema output. This is negotiated independently from ordinary function
    /// tools because many compatible relays accept `tools` but reject `strict`.
    #[serde(default)]
    pub chat_strict_function_tools: ProviderFeatureSupport,
    /// Chat Completions function calls remain structurally valid when tool
    /// arguments are delivered as a stream. This is negotiated separately from
    /// non-streaming function support because compatible relays frequently use
    /// different translation paths for the two transports.
    #[serde(default)]
    pub chat_streaming_tools: ProviderFeatureSupport,
    #[serde(default)]
    pub chat_parallel_tool_calls: ProviderFeatureSupport,
    #[serde(default)]
    pub chat_json_schema_output: ProviderFeatureSupport,
    /// Assistant-message replay requirements discovered for Chat Completions.
    #[serde(default)]
    pub chat_message_protocol: ProviderMessageProtocolCapabilities,
    /// Reasoning request envelope proven together with the Chat function-tool
    /// round trip. `None` is reserved for reports written before v7.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_reasoning_protocol: Option<ProviderReasoningProtocol>,
    pub responses: ProviderFeatureSupport,
    #[serde(default)]
    pub responses_native_tools: ProviderFeatureSupport,
    /// Function tools using the Responses wire shape. Kept separate from
    /// `responses_native_tools`: a relay may accept hosted web search while
    /// rejecting application-defined functions, or vice versa.
    #[serde(default)]
    pub responses_function_tools: ProviderFeatureSupport,
    /// Responses function tools with provider-enforced strict JSON Schema
    /// output. Kept separate from the portable function-tool capability.
    #[serde(default)]
    pub responses_strict_function_tools: ProviderFeatureSupport,
    /// Responses tool calls remain structurally valid over the streaming event
    /// protocol used by the runtime.
    #[serde(default)]
    pub responses_streaming_tools: ProviderFeatureSupport,
    #[serde(default)]
    pub responses_parallel_tool_calls: ProviderFeatureSupport,
    #[serde(default)]
    pub responses_json_schema_output: ProviderFeatureSupport,
    /// Freeform/custom tool definitions and `custom_tool_call` output items.
    #[serde(default)]
    pub responses_custom_tools: ProviderFeatureSupport,
    /// The named Responses `apply_patch` tool and its structured call/output
    /// item pair. This is negotiated per endpoint/model, never inferred from a
    /// vendor or model-name table.
    #[serde(default)]
    pub responses_apply_patch: ProviderFeatureSupport,
    /// Reasoning request envelope proven together with the Responses
    /// function-tool round trip. `None` is reserved for legacy reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responses_reasoning_protocol: Option<ProviderReasoningProtocol>,
    pub developer_messages: ProviderFeatureSupport,
    pub message_compatibility: bool,
    pub checked_at: DateTime<Utc>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl OpenAiCompatibilityReport {
    pub fn applies_to(&self, base_url: &str, model: &str) -> bool {
        self.base_url.trim_end_matches('/') == base_url.trim_end_matches('/')
            && self.model.trim() == model.trim()
    }

    fn profile_for_protocol(&self, protocol: OpenAiProtocol) -> ProviderAdapterProfile {
        let (adapter, instruction_encoding, output_protocol, tool_protocol) = match protocol {
            OpenAiProtocol::ChatCompletions => (
                ProviderAdapterKind::OpenAiChat,
                if self.developer_messages == ProviderFeatureSupport::Supported
                    && !self.message_compatibility
                {
                    ProviderInstructionEncoding::NativeRoles
                } else {
                    ProviderInstructionEncoding::PortableChatEnvelope
                },
                ProviderOutputProtocolCapabilities {
                    json_schema: self.chat_json_schema_output,
                },
                ProviderToolProtocolCapabilities {
                    function_tools: self.chat_function_tools,
                    strict_function_tools: self.chat_strict_function_tools,
                    streaming_tools: self.chat_streaming_tools,
                    parallel_tool_calls: self.chat_parallel_tool_calls,
                    ..ProviderToolProtocolCapabilities::default()
                },
            ),
            OpenAiProtocol::Responses => (
                ProviderAdapterKind::OpenAiResponses,
                ProviderInstructionEncoding::NativeRoles,
                ProviderOutputProtocolCapabilities {
                    json_schema: self.responses_json_schema_output,
                },
                ProviderToolProtocolCapabilities {
                    function_tools: self.responses_function_tools,
                    strict_function_tools: self.responses_strict_function_tools,
                    streaming_tools: self.responses_streaming_tools,
                    parallel_tool_calls: self.responses_parallel_tool_calls,
                    freeform_tools: self.responses_custom_tools,
                    hosted_apply_patch: self.responses_apply_patch,
                    hosted_web_search: self.responses_native_tools,
                    deferred_tool_loading: official_openai_tool_search_support(
                        &self.base_url,
                        &self.model,
                    ),
                    namespace_tools: official_openai_tool_search_support(
                        &self.base_url,
                        &self.model,
                    ),
                    hosted_tool_search: official_openai_tool_search_support(
                        &self.base_url,
                        &self.model,
                    ),
                    ..ProviderToolProtocolCapabilities::default()
                },
            ),
        };
        ProviderAdapterProfile {
            profile_version: PROVIDER_ADAPTER_PROFILE_VERSION,
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            adapter,
            instruction_encoding,
            reasoning_protocol: match protocol {
                OpenAiProtocol::ChatCompletions => self
                    .chat_reasoning_protocol
                    .unwrap_or(ProviderReasoningProtocol::ChatReasoningEffort),
                OpenAiProtocol::Responses => self
                    .responses_reasoning_protocol
                    .unwrap_or(ProviderReasoningProtocol::ResponsesReasoning),
            },
            message_protocol: if protocol == OpenAiProtocol::ChatCompletions {
                self.chat_message_protocol.union(
                    trusted_chat_message_protocol_contract(&self.base_url, &self.model)
                        .unwrap_or_default(),
                )
            } else {
                ProviderMessageProtocolCapabilities::default()
            },
            output_protocol,
            tool_protocol,
            checked_at: self.checked_at,
        }
    }

    /// Returns every wire contract proven by the probe. A connection can keep
    /// both Chat Completions and Responses profiles for the same model. A bare
    /// text HTTP success is not an agent adapter contract: only protocols that
    /// completed the required function-tool round trip are persisted.
    pub fn adapter_profiles(&self) -> Vec<ProviderAdapterProfile> {
        let mut profiles = Vec::new();
        if self.chat_completions == ProviderFeatureSupport::Supported
            && self.chat_function_tools == ProviderFeatureSupport::Supported
        {
            profiles.push(self.profile_for_protocol(OpenAiProtocol::ChatCompletions));
        }
        if self.responses == ProviderFeatureSupport::Supported
            && self.responses_function_tools == ProviderFeatureSupport::Supported
        {
            profiles.push(self.profile_for_protocol(OpenAiProtocol::Responses));
        }
        profiles
    }

    /// Recommended profile retained for legacy readers. Persisting a report
    /// must still store every successful profile returned by
    /// [`Self::adapter_profiles`].
    pub fn adapter_profile(&self) -> ProviderAdapterProfile {
        self.profile_for_protocol(self.selected_protocol)
    }

    /// Selects the strongest proven agent contract without consulting endpoint
    /// identity. Stable ties keep Chat first because it is the smaller portable
    /// contract; Responses wins when its independently proven capabilities are
    /// strictly richer.
    pub(crate) fn recommended_protocol(&self) -> Option<OpenAiProtocol> {
        let mut best = None;
        for protocol in [OpenAiProtocol::ChatCompletions, OpenAiProtocol::Responses] {
            let profile = self.profile_for_protocol(protocol);
            let Some(score) = profile.agent_capability_score() else {
                continue;
            };
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((protocol, score));
            }
        }
        best.map(|(protocol, _)| protocol)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCachePolicy {
    /// GPT-5.6 and later: cache only prefixes ending at explicit breakpoints.
    Explicit30m,
    /// Earlier models: keep the prompt cache in volatile memory.
    LegacyInMemory,
    /// Earlier models that support extended retention.
    Legacy24h,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeCompactionProtocol {
    OpenAiResponsesCompact,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub supports_native_compaction: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_compaction_protocol: Option<NativeCompactionProtocol>,
    pub supports_response_state: bool,
    pub supports_previous_response_id: bool,
    pub supports_provider_items: bool,
    pub supports_prompt_cache: bool,
    #[serde(default)]
    pub tool_protocol: ProviderToolProtocolCapabilities,
}

/// Capabilities of the selected API protocol as actually exposed by a
/// connection. `Unknown` intentionally behaves like unsupported at selection
/// time so compatible relays always retain the portable function-tool path.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderToolProtocolCapabilities {
    pub function_tools: ProviderFeatureSupport,
    pub strict_function_tools: ProviderFeatureSupport,
    /// The selected adapter/model/endpoint tuple has passed a production-codec
    /// round trip with tools enabled and streaming transport selected. Unknown
    /// is deliberately treated as unsupported when preparing a tool-capable
    /// request.
    pub streaming_tools: ProviderFeatureSupport,
    /// The protocol accepts the optional `parallel_tool_calls` request hint.
    pub parallel_tool_calls: ProviderFeatureSupport,
    pub freeform_tools: ProviderFeatureSupport,
    pub hosted_apply_patch: ProviderFeatureSupport,
    /// The endpoint/model can execute the Responses hosted `web_search` tool.
    /// Compatible relays must prove this independently from function tools.
    pub hosted_web_search: ProviderFeatureSupport,
    pub assistant_phase: ProviderFeatureSupport,
    /// Function definitions may be advertised with `defer_loading`.
    pub deferred_tool_loading: ProviderFeatureSupport,
    /// Deferred functions may be grouped under native namespaces.
    pub namespace_tools: ProviderFeatureSupport,
    /// The provider can execute the hosted `tool_search` tool.
    pub hosted_tool_search: ProviderFeatureSupport,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::OpenAiCompatible => "openai_compatible",
            Self::OpenAiResponses => "openai_responses",
            Self::CodexAppServer => "codex_app_server",
            Self::Anthropic => "anthropic",
        }
    }

    fn legacy_transport(self) -> ProviderTransportKind {
        match self {
            Self::Mock => ProviderTransportKind::Mock,
            Self::CodexAppServer => ProviderTransportKind::CodexAppServer,
            Self::OpenAiCompatible | Self::OpenAiResponses | Self::Anthropic => {
                ProviderTransportKind::Http
            }
        }
    }

    fn legacy_auth(self) -> ProviderAuthKind {
        match self {
            Self::Mock => ProviderAuthKind::None,
            Self::CodexAppServer => ProviderAuthKind::CodexSession,
            Self::Anthropic => ProviderAuthKind::XApiKey,
            Self::OpenAiCompatible | Self::OpenAiResponses => ProviderAuthKind::Bearer,
        }
    }

    fn legacy_adapters(self) -> Vec<ProviderAdapterKind> {
        match self {
            Self::Mock => vec![ProviderAdapterKind::Mock],
            Self::CodexAppServer => vec![ProviderAdapterKind::CodexAppServer],
            Self::Anthropic => vec![ProviderAdapterKind::AnthropicMessages],
            Self::OpenAiCompatible | Self::OpenAiResponses => vec![
                ProviderAdapterKind::OpenAiChat,
                ProviderAdapterKind::OpenAiResponses,
            ],
        }
    }

    fn legacy_preferred_adapter(self) -> ProviderAdapterKind {
        match self {
            Self::Mock => ProviderAdapterKind::Mock,
            Self::CodexAppServer => ProviderAdapterKind::CodexAppServer,
            Self::Anthropic => ProviderAdapterKind::AnthropicMessages,
            Self::OpenAiResponses => ProviderAdapterKind::OpenAiResponses,
            Self::OpenAiCompatible => ProviderAdapterKind::OpenAiChat,
        }
    }
}

pub(crate) fn is_official_openai_endpoint(base_url: &str) -> bool {
    base_url
        .trim_end_matches('/')
        .eq_ignore_ascii_case("https://api.openai.com/v1")
}

/// Trusted built-in contracts cover direct vendor endpoints before the first
/// connection probe. Relays and opaque model aliases acquire the same flag from
/// observed probe output and persist it in their adapter profile.
pub(crate) fn trusted_chat_message_protocol_contract(
    base_url: &str,
    model: &str,
) -> Option<ProviderMessageProtocolCapabilities> {
    let model = model.trim().to_ascii_lowercase();
    let model = model.rsplit('/').next().unwrap_or(&model);
    let model = model.split(':').next().unwrap_or(model);
    let official_deepseek = reqwest::Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"));
    let deepseek_thinking_model = model.starts_with("deepseek-v4-flash")
        || model.starts_with("deepseek-v4-pro")
        || model.starts_with("deepseek-reasoner");

    if official_deepseek && deepseek_thinking_model {
        return Some(ProviderMessageProtocolCapabilities {
            requires_reasoning_content_for_tool_calls: true,
        });
    }
    is_official_openai_endpoint(base_url).then_some(ProviderMessageProtocolCapabilities::default())
}

pub(crate) fn official_openai_tool_search_support(
    base_url: &str,
    model: &str,
) -> ProviderFeatureSupport {
    let official_endpoint = is_official_openai_endpoint(base_url);
    let version = model.strip_prefix("gpt-").and_then(|suffix| {
        let version = suffix
            .split(|character: char| !character.is_ascii_digit() && character != '.')
            .next()?;
        let mut parts = version.split('.');
        let major = parts.next()?.parse::<u32>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
        Some((major, minor))
    });
    if official_endpoint
        && version.is_some_and(|(major, minor)| major > 5 || (major == 5 && minor >= 4))
    {
        ProviderFeatureSupport::Supported
    } else {
        ProviderFeatureSupport::Unknown
    }
}

/// Explicit prompt-cache breakpoints and `prompt_cache_options` are an
/// official OpenAI Responses capability starting with the GPT-5.6 family.
/// Unknown relays stay on implicit caching until they expose a negotiated
/// capability instead of receiving fields they may reject.
pub(crate) fn official_openai_explicit_prompt_cache_support(
    base_url: &str,
    model: &str,
) -> ProviderFeatureSupport {
    let official_endpoint = is_official_openai_endpoint(base_url);
    let version = model.strip_prefix("gpt-").and_then(|suffix| {
        let version = suffix
            .split(|character: char| !character.is_ascii_digit() && character != '.')
            .next()?;
        let mut parts = version.split('.');
        let major = parts.next()?.parse::<u32>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
        Some((major, minor))
    });
    if official_endpoint
        && version.is_some_and(|(major, minor)| major > 5 || (major == 5 && minor >= 6))
    {
        ProviderFeatureSupport::Supported
    } else {
        ProviderFeatureSupport::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilities {
    /// Image-input support reported by the provider's model catalog. `None`
    /// means the endpoint did not publish modality metadata for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelSettings {
    /// An explicit user choice that takes precedence over catalog detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
    /// Outer `None` inherits the legacy connection setting. Inner `None`
    /// explicitly omits the request parameter for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<Option<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<Option<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<Option<String>>,
    /// Optional model-level protocol preference. This is independent from the
    /// connection preset and may be overridden again by a thread selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_adapter: Option<ProviderAdapterKind>,
}

type ProviderAdapterProfiles =
    BTreeMap<String, BTreeMap<ProviderAdapterKind, ProviderAdapterProfile>>;

fn deserialize_provider_adapter_profiles<'de, D>(
    deserializer: D,
) -> Result<ProviderAdapterProfiles, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StoredProfiles {
        Multiple(ProviderAdapterProfiles),
        Legacy(BTreeMap<String, ProviderAdapterProfile>),
    }

    match StoredProfiles::deserialize(deserializer)? {
        StoredProfiles::Multiple(profiles) => Ok(profiles),
        StoredProfiles::Legacy(profiles) => Ok(profiles
            .into_iter()
            .map(|(model, profile)| {
                let adapter = profile.adapter;
                (model, BTreeMap::from([(adapter, profile)]))
            })
            .collect()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub id: String,
    /// User-facing label. Empty values from legacy settings fall back to `id`.
    #[serde(default)]
    pub name: String,
    /// Deprecated preset identity retained only for serialized compatibility.
    /// Runtime dispatch must use `effective_transport`, `effective_auth`, and
    /// `resolved_adapter_for_model` instead.
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<ProviderTransportKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<ProviderAuthKind>,
    /// Protocols this connection is permitted to use. Empty is a legacy value
    /// and is interpreted from `kind` until settings are next saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_adapters: Vec<ProviderAdapterKind>,
    /// Connection-wide preference. `None` means use model preference, then the
    /// latest probe recommendation, then the legacy preset fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_adapter: Option<ProviderAdapterKind>,
    pub base_url: String,
    /// Default model for this connection. Threads may override it per
    /// conversation; this value is the fallback for new threads and for
    /// internal utility calls such as title generation.
    pub model: String,
    /// Model families the user allowed for this connection. Empty means "not
    /// narrowed yet", which shows every synced family rather than none.
    #[serde(default)]
    pub enabled_families: Vec<String>,
    /// Model ids last returned by the connection's `/v1/models` endpoint.
    /// Cached so the picker works offline; refreshed on explicit sync.
    #[serde(default)]
    pub synced_models: Vec<String>,
    /// Context windows the connection reported for its own models. Populated on
    /// sync when the endpoint publishes them, which is the only real capability
    /// detection available; it outranks the built-in table.
    #[serde(default)]
    pub model_context_windows: BTreeMap<String, usize>,
    /// Capabilities reported for each model by the connection's catalog.
    #[serde(default)]
    pub model_capabilities: BTreeMap<String, ProviderModelCapabilities>,
    /// Negotiated wire contract per model. This is the sole runtime source for
    /// adapter selection and message lowering; probe reports are diagnostics.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "deserialize_provider_adapter_profiles"
    )]
    pub adapter_profiles: ProviderAdapterProfiles,
    /// Per-model user overrides. These are intentionally separate from the
    /// catalog so a subsequent sync never discards an explicit choice.
    #[serde(default)]
    pub model_settings: BTreeMap<String, ProviderModelSettings>,
    #[serde(default)]
    pub models_synced_at: Option<DateTime<Utc>>,
    /// `None` means "don't send temperature — let the model use its default."
    /// This is important for reasoning models (o-series, GPT-5.x) that reject
    /// explicit temperature, and for users who want the vendor default.
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Optional user override. When omitted, the server resolves a known model
    /// capability and falls back to a conservative default for custom models.
    #[serde(default)]
    pub context_window_tokens: Option<usize>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub store_responses: bool,
    #[serde(default = "default_parallel_tool_calls")]
    pub parallel_tool_calls: bool,
    #[serde(default)]
    pub prompt_cache_key: Option<String>,
    #[serde(default)]
    pub prompt_cache_policy: Option<PromptCachePolicy>,
    #[serde(default)]
    pub responses_compaction_threshold_tokens: Option<u32>,
    #[serde(default)]
    pub rollout_budget: Option<RolloutBudgetSettings>,
    /// Last explicit compatibility probe for an OpenAI-compatible `/v1`
    /// connection. The endpoint and model are included so stale results are
    /// ignored after either setting changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_compatibility: Option<OpenAiCompatibilityReport>,
    pub api_key_source: String,
    pub api_key_configured: bool,
    pub health_status: Option<String>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "default".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            // Keep these empty until AppSettings::touch materializes the
            // independent axes. This preserves programmatic legacy callers
            // that construct a default and then only replace `kind`.
            transport: None,
            auth: None,
            allowed_adapters: Vec::new(),
            preferred_adapter: None,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4.1-mini".to_string(),
            enabled_families: Vec::new(),
            synced_models: Vec::new(),
            model_context_windows: BTreeMap::new(),
            model_capabilities: BTreeMap::new(),
            adapter_profiles: BTreeMap::new(),
            model_settings: BTreeMap::new(),
            models_synced_at: None,
            temperature: None,
            max_output_tokens: None,
            context_window_tokens: None,
            reasoning_effort: None,
            store_responses: false,
            parallel_tool_calls: default_parallel_tool_calls(),
            prompt_cache_key: None,
            prompt_cache_policy: None,
            responses_compaction_threshold_tokens: None,
            rollout_budget: None,
            openai_compatibility: None,
            api_key_source: "OPENTOPIA_API_KEY".to_string(),
            api_key_configured: false,
            health_status: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedProviderRoute {
    pub connection_id: String,
    pub model: String,
    pub transport: ProviderTransportKind,
    pub transport_driver_id: String,
    pub adapter: ProviderAdapterKind,
    pub adapter_profile_version: Option<u32>,
}

impl ResolvedProviderRoute {
    pub fn adapter_identity(&self) -> String {
        match self.adapter_profile_version {
            Some(version) => format!("{}:v{version}", self.adapter.as_str()),
            None => self.adapter.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RolloutBudgetSettings {
    pub limit_tokens: u64,
    #[serde(default = "default_rollout_sampling_token_weight")]
    pub sampling_token_weight: f64,
    #[serde(default = "default_rollout_prefill_token_weight")]
    pub prefill_token_weight: f64,
}

impl RolloutBudgetSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.limit_tokens == 0 {
            return Err("rollout budget limitTokens must be greater than zero");
        }
        if !self.sampling_token_weight.is_finite() || self.sampling_token_weight < 0.0 {
            return Err("rollout budget samplingTokenWeight must be finite and non-negative");
        }
        if !self.prefill_token_weight.is_finite() || self.prefill_token_weight < 0.0 {
            return Err("rollout budget prefillTokenWeight must be finite and non-negative");
        }
        if self.sampling_token_weight == 0.0 && self.prefill_token_weight == 0.0 {
            return Err("rollout budget requires at least one positive token weight");
        }
        Ok(())
    }
}

impl ProviderSettings {
    /// Applies a deprecated `kind` update as a complete compatibility preset.
    /// New callers should update the independent axes directly.
    pub fn apply_legacy_kind_preset(&mut self, kind: ProviderKind) {
        self.kind = kind;
        self.transport = Some(kind.legacy_transport());
        self.auth = Some(kind.legacy_auth());
        self.allowed_adapters = kind.legacy_adapters();
        self.preferred_adapter = Some(kind.legacy_preferred_adapter());
    }

    pub fn effective_transport(&self) -> ProviderTransportKind {
        self.transport
            .unwrap_or_else(|| self.kind.legacy_transport())
    }

    pub fn effective_auth(&self) -> ProviderAuthKind {
        self.auth.unwrap_or_else(|| self.kind.legacy_auth())
    }

    pub fn effective_allowed_adapters(&self) -> Vec<ProviderAdapterKind> {
        let mut adapters = if self.allowed_adapters.is_empty() {
            self.kind.legacy_adapters()
        } else {
            self.allowed_adapters.clone()
        };
        adapters.sort();
        adapters.dedup();
        adapters
    }

    pub fn allows_adapter(&self, adapter: ProviderAdapterKind) -> bool {
        self.effective_allowed_adapters().contains(&adapter)
    }

    /// Materializes the legacy compatibility fields into the independent
    /// connection axes. Called on load/save so new serialized settings no
    /// longer need `kind` to resolve runtime behavior.
    pub fn migrate_legacy_connection_axes(&mut self) {
        self.transport
            .get_or_insert_with(|| self.kind.legacy_transport());
        self.auth.get_or_insert_with(|| self.kind.legacy_auth());
        if self.allowed_adapters.is_empty() {
            self.allowed_adapters = self.kind.legacy_adapters();
        }
    }

    /// One-way load migration for settings written before normalized adapter
    /// profiles became authoritative. The compatibility report remains useful
    /// diagnostics, but no request-path decision reads it after this step.
    pub fn migrate_legacy_openai_compatibility_report(&mut self) {
        let Some(report) = self
            .openai_compatibility
            .clone()
            .filter(|report| report.applies_to(&self.base_url, &report.model))
        else {
            return;
        };
        let model = report.model.trim().to_string();
        for profile in report.adapter_profiles() {
            self.adapter_profiles
                .entry(model.clone())
                .or_default()
                .entry(profile.adapter)
                .or_insert(profile);
        }
        if self.preferred_adapter.is_none() {
            let settings = self.model_settings.entry(model).or_default();
            settings
                .preferred_adapter
                .get_or_insert(match report.selected_protocol {
                    OpenAiProtocol::ChatCompletions => ProviderAdapterKind::OpenAiChat,
                    OpenAiProtocol::Responses => ProviderAdapterKind::OpenAiResponses,
                });
        }
    }

    /// Persists deterministic schema upgrades once during settings load/save,
    /// so provider construction consumes only complete current contracts.
    /// Profiles older than the migratable floor are removed instead of being
    /// reconstructed from model-name guesses.
    pub fn migrate_adapter_profiles(&mut self) {
        for (model, profiles) in &mut self.adapter_profiles {
            profiles.retain(|_, profile| {
                let Some(normalized) = profile.clone().normalized_for(&self.base_url, model.trim())
                else {
                    return false;
                };
                *profile = normalized;
                true
            });
        }
        self.adapter_profiles
            .retain(|_, profiles| !profiles.is_empty());
    }

    pub fn adapter_profile_for_model_and_adapter(
        &self,
        model: &str,
        adapter: ProviderAdapterKind,
    ) -> Option<ProviderAdapterProfile> {
        let model = model.trim();
        self.adapter_profiles
            .get(model)
            .and_then(|profiles| profiles.get(&adapter))
            .cloned()
            .and_then(|profile| profile.normalized_for(&self.base_url, model))
    }

    pub fn resolved_adapter_for_model(&self, model: &str) -> ProviderAdapterKind {
        match self.effective_transport() {
            ProviderTransportKind::Mock => return ProviderAdapterKind::Mock,
            ProviderTransportKind::CodexAppServer => {
                return ProviderAdapterKind::CodexAppServer;
            }
            ProviderTransportKind::Http => {}
        }

        let model = model.trim();
        let allowed = self.effective_allowed_adapters();
        let allowed_contains = |adapter: ProviderAdapterKind| allowed.contains(&adapter);
        let has_negotiated_profiles = self
            .adapter_profiles
            .get(model)
            .is_some_and(|profiles| !profiles.is_empty());
        let preference_is_usable = |adapter: ProviderAdapterKind| {
            allowed_contains(adapter)
                && (!has_negotiated_profiles
                    || self
                        .adapter_profile_for_model_and_adapter(model, adapter)
                        .is_some())
        };
        if let Some(adapter) = self
            .model_settings
            .get(model)
            .and_then(|settings| settings.preferred_adapter)
            .filter(|adapter| preference_is_usable(*adapter))
        {
            return adapter;
        }
        if let Some(adapter) = self
            .preferred_adapter
            .filter(|adapter| preference_is_usable(*adapter))
        {
            return adapter;
        }
        let mut best_profile = None;
        for adapter in allowed.iter().copied() {
            let Some(profile) = self.adapter_profile_for_model_and_adapter(model, adapter) else {
                continue;
            };
            let Some(score) = profile.agent_capability_score() else {
                continue;
            };
            // `allowed` is sorted, so strict improvement preserves the stable
            // Chat-first tie break for equally capable OpenAI wire contracts.
            if best_profile.is_none_or(|(_, best_score)| score > best_score) {
                best_profile = Some((adapter, score));
            }
        }
        if let Some((adapter, _)) = best_profile {
            return adapter;
        }
        let legacy = self.kind.legacy_preferred_adapter();
        if allowed_contains(legacy) {
            legacy
        } else {
            allowed.first().copied().unwrap_or(legacy)
        }
    }

    pub fn adapter_profile_for_model(&self, model: &str) -> Option<ProviderAdapterProfile> {
        self.adapter_profile_for_model_and_adapter(model, self.resolved_adapter_for_model(model))
    }

    pub fn active_adapter_profile(&self) -> Option<ProviderAdapterProfile> {
        self.adapter_profile_for_model(&self.model)
    }

    pub fn apply_openai_compatibility_report(&mut self, report: OpenAiCompatibilityReport) {
        let model = report.model.trim().to_string();
        let selected_adapter = match report.selected_protocol {
            OpenAiProtocol::ChatCompletions => ProviderAdapterKind::OpenAiChat,
            OpenAiProtocol::Responses => ProviderAdapterKind::OpenAiResponses,
        };
        let previous_recommendation = self
            .openai_compatibility
            .as_ref()
            .filter(|previous| previous.applies_to(&self.base_url, &model))
            .map(|previous| match previous.selected_protocol {
                OpenAiProtocol::ChatCompletions => ProviderAdapterKind::OpenAiChat,
                OpenAiProtocol::Responses => ProviderAdapterKind::OpenAiResponses,
            });
        if let Some(profiles) = self.adapter_profiles.get_mut(report.model.trim()) {
            profiles.remove(&ProviderAdapterKind::OpenAiChat);
            profiles.remove(&ProviderAdapterKind::OpenAiResponses);
        }
        for profile in report.adapter_profiles() {
            self.apply_adapter_profile(profile);
        }
        if self.preferred_adapter.is_none() {
            let preferred_adapter = &mut self
                .model_settings
                .entry(model)
                .or_default()
                .preferred_adapter;
            if preferred_adapter.is_none() || *preferred_adapter == previous_recommendation {
                *preferred_adapter = Some(selected_adapter);
            }
        }
        self.openai_compatibility = Some(report);
    }

    pub fn apply_adapter_profile(&mut self, profile: ProviderAdapterProfile) {
        self.adapter_profiles
            .entry(profile.model.trim().to_string())
            .or_default()
            .insert(profile.adapter, profile);
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities_for_adapter(self.resolved_adapter_for_model(&self.model))
    }

    pub fn capabilities_for_adapter(&self, adapter: ProviderAdapterKind) -> ProviderCapabilities {
        let negotiated = self.adapter_profile_for_model_and_adapter(&self.model, adapter);
        match adapter {
            ProviderAdapterKind::OpenAiResponses => {
                ProviderCapabilities {
                    supports_native_compaction: self
                        .responses_compaction_threshold_tokens
                        .is_some(),
                    native_compaction_protocol: Some(
                        NativeCompactionProtocol::OpenAiResponsesCompact,
                    ),
                    supports_response_state: self.store_responses,
                    supports_previous_response_id: self.store_responses,
                    supports_provider_items: true,
                    supports_prompt_cache: true,
                    tool_protocol: ProviderToolProtocolCapabilities {
                        // Function tools are the portable Responses baseline.
                        // An explicit failed probe overrides that baseline.
                        function_tools: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.function_tools)
                            .unwrap_or(ProviderFeatureSupport::Supported),
                        strict_function_tools: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.strict_function_tools)
                            .unwrap_or_default(),
                        streaming_tools: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.streaming_tools)
                            .unwrap_or_default(),
                        parallel_tool_calls: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.parallel_tool_calls)
                            .unwrap_or_default(),
                        freeform_tools: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.freeform_tools)
                            .unwrap_or_default(),
                        hosted_apply_patch: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.hosted_apply_patch)
                            .unwrap_or_default(),
                        hosted_web_search: negotiated
                            .as_ref()
                            .map(|profile| profile.tool_protocol.hosted_web_search)
                            .unwrap_or_else(|| {
                                if is_official_openai_endpoint(&self.base_url) {
                                    ProviderFeatureSupport::Supported
                                } else {
                                    ProviderFeatureSupport::Unknown
                                }
                            }),
                        // Phase is optional on the wire and cannot be proven
                        // without observing an assistant message. Parsing is
                        // always tolerant; replayed items are authoritative.
                        assistant_phase: ProviderFeatureSupport::Unknown,
                        deferred_tool_loading: official_openai_tool_search_support(
                            &self.base_url,
                            &self.model,
                        ),
                        namespace_tools: official_openai_tool_search_support(
                            &self.base_url,
                            &self.model,
                        ),
                        hosted_tool_search: official_openai_tool_search_support(
                            &self.base_url,
                            &self.model,
                        ),
                    },
                }
            }
            ProviderAdapterKind::OpenAiChat => ProviderCapabilities {
                supports_prompt_cache: true,
                tool_protocol: negotiated
                    .map(|profile| profile.tool_protocol)
                    .unwrap_or_default(),
                ..ProviderCapabilities::default()
            },
            ProviderAdapterKind::Mock
            | ProviderAdapterKind::CodexAppServer
            | ProviderAdapterKind::AnthropicMessages => ProviderCapabilities::default(),
        }
    }

    pub fn resolved_route(&self) -> ResolvedProviderRoute {
        let adapter = self.resolved_adapter_for_model(&self.model);
        let profile = self.adapter_profile_for_model_and_adapter(&self.model, adapter);
        let transport = self.effective_transport();
        ResolvedProviderRoute {
            connection_id: self.id.clone(),
            model: self.model.clone(),
            transport,
            transport_driver_id: match transport {
                ProviderTransportKind::Http => "http".to_string(),
                ProviderTransportKind::CodexAppServer => "codex_app_server".to_string(),
                ProviderTransportKind::Mock => "mock".to_string(),
            },
            adapter,
            adapter_profile_version: profile.map(|profile| profile.profile_version),
        }
    }

    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            &self.id
        } else {
            name
        }
    }

    /// Resolution order, most to least trusted: the user's own override, then
    /// what the endpoint reported about this model, then the built-in table,
    /// then a conservative default. The endpoint outranks the table because the
    /// table cannot know about models released after this build.
    pub fn resolved_context_window_tokens(&self) -> usize {
        self.current_model_settings()
            .and_then(|settings| settings.context_window_tokens)
            .flatten()
            .or(self.context_window_tokens)
            .filter(|tokens| *tokens >= MIN_PROVIDER_CONTEXT_WINDOW_TOKENS)
            .or_else(|| self.reported_context_window_tokens())
            .or_else(|| known_model_context_window_tokens(&self.model))
            .unwrap_or(DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS)
    }

    /// Context window this connection published for its current model.
    fn reported_context_window_tokens(&self) -> Option<usize> {
        self.model_context_windows
            .get(self.model.trim())
            .copied()
            .filter(|tokens| *tokens >= MIN_PROVIDER_CONTEXT_WINDOW_TOKENS)
    }

    /// Resolves image-input support for the selected model. Explicit model
    /// settings win over catalog metadata, which wins over the checked-in
    /// model registry. Models absent from every source fail closed.
    pub fn supports_vision_for_model(&self) -> bool {
        self.current_model_settings()
            .and_then(|settings| settings.supports_vision)
            .or_else(|| {
                self.model_capabilities
                    .get(self.model.trim())
                    .and_then(|capabilities| capabilities.supports_vision)
            })
            .or_else(|| known_model_supports_vision(&self.model))
            .unwrap_or(false)
    }

    pub fn temperature_for_model(&self) -> Option<f64> {
        self.current_model_settings()
            .and_then(|settings| settings.temperature)
            .unwrap_or(self.temperature)
    }

    pub fn max_output_tokens_for_model(&self) -> Option<u32> {
        self.current_model_settings()
            .and_then(|settings| settings.max_output_tokens)
            .unwrap_or(self.max_output_tokens)
    }

    pub fn reasoning_effort_for_model(&self) -> Option<String> {
        self.current_model_settings()
            .and_then(|settings| settings.reasoning_effort.clone())
            .unwrap_or_else(|| self.reasoning_effort.clone())
    }

    fn current_model_settings(&self) -> Option<&ProviderModelSettings> {
        self.model_settings.get(self.model.trim())
    }

    /// Applies a per-thread model override on top of the connection defaults.
    /// The connection keeps owning transport concerns (endpoint, key, limits);
    /// only the model and its reasoning effort vary per conversation.
    pub fn with_model_override(
        &self,
        model: Option<&str>,
        reasoning_effort: Option<Option<&str>>,
    ) -> Self {
        let mut resolved = self.clone();
        if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
            resolved.model = model.to_string();
        }
        if let Some(reasoning_effort) = reasoning_effort {
            let value = reasoning_effort
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            resolved
                .model_settings
                .entry(resolved.model.clone())
                .or_default()
                .reasoning_effort = Some(value);
        }
        resolved
    }

    /// Applies the complete thread route without mutating the persisted
    /// connection. The adapter override is stored on the cloned model settings
    /// so it outranks connection defaults during runtime resolution.
    pub fn with_model_route_override(
        &self,
        model: Option<&str>,
        reasoning_effort: Option<Option<&str>>,
        adapter: Option<ProviderAdapterKind>,
    ) -> Self {
        let mut resolved = self.with_model_override(model, reasoning_effort);
        if let Some(adapter) = adapter {
            resolved
                .model_settings
                .entry(resolved.model.clone())
                .or_default()
                .preferred_adapter = Some(adapter);
        }
        resolved
    }

    pub fn from_env() -> Self {
        let mut settings = Self::default();
        if let Some(base_url) = first_env([
            "OPENTOPIA_OPENAI_BASE_URL",
            "AUDIT_COPILOT_LLM_BASE_URL",
            "CREDIT_REVIEW_LLM_BASE_URL",
            "OPENAI_BASE_URL",
        ]) {
            settings.base_url = base_url;
        }
        if let Some(model) = first_env([
            "OPENTOPIA_MODEL",
            "AUDIT_COPILOT_LLM_MODEL",
            "CREDIT_REVIEW_LLM_MODEL",
            "CREDIT_REVIEW_LLM_CHEAP_MODEL",
            "CREDIT_REVIEW_LLM_STRONG_MODEL",
        ]) {
            settings.model = model;
        }
        if let Some((source, _value)) = first_env_with_key([
            "OPENTOPIA_API_KEY",
            "AUDIT_COPILOT_LLM_API_KEY",
            "CREDIT_REVIEW_LLM_API_KEY",
            "OPENAI_API_KEY",
        ]) {
            settings.api_key_source = source;
            settings.api_key_configured = true;
        }
        if let Some(limit_tokens) = std::env::var("OPENTOPIA_ROLLOUT_TOKEN_LIMIT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            let sampling_token_weight = std::env::var("OPENTOPIA_ROLLOUT_OUTPUT_WEIGHT")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or_else(default_rollout_sampling_token_weight);
            let prefill_token_weight = std::env::var("OPENTOPIA_ROLLOUT_INPUT_WEIGHT")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or_else(default_rollout_prefill_token_weight);
            let rollout_budget = RolloutBudgetSettings {
                limit_tokens,
                sampling_token_weight,
                prefill_token_weight,
            };
            if rollout_budget.validate().is_ok() {
                settings.rollout_budget = Some(rollout_budget);
            }
        }
        settings
    }
}

/// Context window for model families we ship knowledge about.
///
/// This is a hand-maintained fallback, not capability detection: it only runs
/// when the user has not set an override and the connection's model catalog did
/// not report a window. Entries are matched against the model id with vendor
/// prefixes stripped, because relays rename freely (`openai/gpt-5.6`).
///
/// An unlisted newer generation may inherit its same-series, same-variant
/// predecessor. This lets a newly released `deepseek-v5-flash` use the known
/// `deepseek-v4-flash` window, but never borrows across variants such as Pro,
/// Image, or Coder. Models that do not meet that narrow rule return `None` so
/// the caller applies its conservative default.
pub fn known_model_context_window_tokens(model: &str) -> Option<usize> {
    let bases = model_bases(model);
    bases
        .iter()
        .find_map(|base| {
            known_model_capability_registry()
                .models
                .get(base)
                .map(|capabilities| capabilities.context_window_tokens)
        })
        .or_else(|| {
            bases
                .iter()
                .find_map(|base| inferred_model_context_window_tokens(base))
        })
}

fn inferred_model_context_window_tokens(model: &str) -> Option<usize> {
    let target = model_generation(model)?;
    known_model_capability_registry()
        .models
        .iter()
        .filter_map(|(model_id, capabilities)| {
            let candidate = model_generation(model_id)?;
            (candidate.prefix == target.prefix
                && candidate.suffix == target.suffix
                && compare_model_generations(&candidate.version, &target.version) == Ordering::Less)
                .then_some((candidate.version, capabilities.context_window_tokens))
        })
        .max_by(|(left, _), (right, _)| compare_model_generations(left, right))
        .map(|(_, tokens)| tokens)
}

#[derive(Debug)]
struct ModelGeneration<'a> {
    prefix: &'a str,
    version: Vec<usize>,
    suffix: &'a str,
}

fn model_generation(model: &str) -> Option<ModelGeneration<'_>> {
    let bytes = model.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }

        let mut version = Vec::new();
        let mut cursor = start;
        loop {
            let part_start = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            version.push(model[part_start..cursor].parse().ok()?);

            if cursor + 1 >= bytes.len()
                || bytes[cursor] != b'.'
                || !bytes[cursor + 1].is_ascii_digit()
            {
                break;
            }
            cursor += 1;
        }

        if cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
            continue;
        }
        return Some(ModelGeneration {
            prefix: &model[..start],
            version,
            suffix: &model[cursor..],
        });
    }
    None
}

fn compare_model_generations(left: &[usize], right: &[usize]) -> Ordering {
    for index in 0..left.len().max(right.len()) {
        match left
            .get(index)
            .copied()
            .unwrap_or_default()
            .cmp(&right.get(index).copied().unwrap_or_default())
        {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

const KNOWN_MODEL_CAPABILITY_REGISTRY_JSON: &str = include_str!("../../model-capabilities.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KnownModelCapabilityRegistry {
    schema_version: u32,
    models: BTreeMap<String, KnownModelCapabilities>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KnownModelCapabilities {
    context_window_tokens: usize,
    supports_vision: bool,
}

fn known_model_capability_registry() -> &'static KnownModelCapabilityRegistry {
    static REGISTRY: OnceLock<KnownModelCapabilityRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry: KnownModelCapabilityRegistry =
            serde_json::from_str(KNOWN_MODEL_CAPABILITY_REGISTRY_JSON)
                .expect("model-capabilities.json must be valid");
        assert_eq!(
            registry.schema_version, 1,
            "unsupported shared model capability registry schema"
        );
        registry
    })
}

/// Image-input support from the checked-in model registry.
///
/// Matching follows the same exact-ID and relay-prefix normalization used for
/// context windows. A missing entry remains unknown so custom models fail
/// closed instead of inheriting a family-wide guess.
pub fn known_model_supports_vision(model: &str) -> Option<bool> {
    model_bases(model).iter().find_map(|base| {
        known_model_capability_registry()
            .models
            .get(base)
            .map(|capabilities| capabilities.supports_vision)
    })
}

/// Candidate model names after removing the vendor prefixes relays prepend.
///
/// Only a leading `vendor.` is stripped, never an inner dot, because dots also
/// carry version numbers (`gpt-5.6`).
fn model_bases(model: &str) -> Vec<String> {
    let normalized = model.trim().to_ascii_lowercase();
    let after_slash = normalized.rsplit('/').next().unwrap_or(&normalized);
    // OpenRouter uses modifiers such as `:free` and `:thinking`; they don't
    // identify a different base model or context window.
    let after_variant = after_slash
        .split_once(':')
        .map_or(after_slash, |(base, _)| base)
        .to_string();

    let mut bases = vec![after_variant.clone()];
    if let Some((prefix, rest)) = after_variant.split_once('.') {
        // `azure.o3-mini` is a vendor prefix; `gpt-5.6` is a version.
        let looks_like_vendor = !prefix.is_empty()
            && prefix.chars().all(|c| c.is_ascii_alphabetic())
            && rest.starts_with(|c: char| c.is_ascii_alphabetic());
        if looks_like_vendor {
            bases.push(rest.to_string());
        }
    }
    bases
}

pub const MIN_PROVIDER_CONTEXT_WINDOW_TOKENS: usize = 4_096;
pub const DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS: usize = 128_000;

fn default_parallel_tool_calls() -> bool {
    true
}

fn default_rollout_sampling_token_weight() -> f64 {
    1.0
}

fn default_rollout_prefill_token_weight() -> f64 {
    1.0
}

fn first_env<const N: usize>(keys: [&str; N]) -> Option<String> {
    first_env_with_key(keys).map(|(_key, value)| value)
}

fn first_env_with_key<const N: usize>(keys: [&str; N]) -> Option<(String, String)> {
    keys.into_iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.is_empty())
            .map(|value| (key.to_string(), value))
    })
}
