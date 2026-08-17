use crate::policy::PermissionMode;
use crate::prompt_runtime::AgentRuntimeSettings;
use crate::sandbox::{
    LocalSandboxConfig, NetworkPolicy, OsSandboxMode, SandboxMode, WindowsSandboxBackend,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Legacy provider preset identity. New runtime code resolves transport,
/// authentication, and adapter independently; this enum remains serialized for
/// one compatibility window so older desktop builds can still read settings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTransportKind {
    Http,
    CodexAppServer,
    Mock,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthKind {
    Bearer,
    XApiKey,
    CodexSession,
    None,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiProtocol {
    ChatCompletions,
    Responses,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderInstructionEncoding {
    NativeRoles,
    FoldDeveloperIntoSystem,
    PortableChatEnvelope,
}

/// Provider-specific request fields used to control model reasoning. The
/// negotiated profile owns this choice; request codecs never infer it again
/// from a model id.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReasoningProtocol {
    #[default]
    ReasoningEffort,
    DeepSeekThinking,
    GlmThinking,
}

pub const PROVIDER_ADAPTER_PROFILE_VERSION: u32 = 5;
const MIN_MIGRATABLE_PROVIDER_ADAPTER_PROFILE_VERSION: u32 = 2;
#[cfg(test)]
const PREVIOUS_PROVIDER_ADAPTER_PROFILE_VERSION: u32 = PROVIDER_ADAPTER_PROFILE_VERSION - 1;

/// Assistant-message constraints imposed by one concrete wire protocol. These
/// are negotiated or supplied by a trusted built-in endpoint contract; request
/// codecs consume the result without inspecting vendor or model names.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderMessageProtocolCapabilities {
    /// Every assistant message that contains tool calls must preserve the
    /// provider-issued `reasoning_content` field in subsequent requests.
    pub requires_reasoning_content_for_tool_calls: bool,
}

/// Structured final-output features exposed by one concrete wire protocol.
/// These are negotiated during provider setup and never inferred by retrying a
/// modified request after a live turn has already started.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
                if self.adapter == ProviderAdapterKind::OpenAiChat {
                    self.reasoning_protocol = chat_reasoning_protocol_for_model(model);
                    self.message_protocol = self
                        .message_protocol
                        .union(trusted_chat_message_protocol_contract(base_url, model)?);
                }
                Some(self)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
            reasoning_protocol: if protocol == OpenAiProtocol::ChatCompletions {
                chat_reasoning_protocol_for_model(&self.model)
            } else {
                ProviderReasoningProtocol::ReasoningEffort
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
            && self.responses_native_tools == ProviderFeatureSupport::Supported
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCachePolicy {
    /// GPT-5.6 and later: cache only prefixes ending at explicit breakpoints.
    Explicit30m,
    /// Earlier models: keep the prompt cache in volatile memory.
    LegacyInMemory,
    /// Earlier models that support extended retention.
    Legacy24h,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeCompactionProtocol {
    OpenAiResponsesCompact,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
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

pub(crate) fn chat_reasoning_protocol_for_model(model: &str) -> ProviderReasoningProtocol {
    let model = model.trim().to_ascii_lowercase();
    let model = model.rsplit('/').next().unwrap_or(&model);
    let model = model.split(':').next().unwrap_or(model);
    if model.starts_with("deepseek-v4-flash")
        || model.starts_with("deepseek-v4-pro")
        || model.starts_with("deepseek-reasoner")
    {
        ProviderReasoningProtocol::DeepSeekThinking
    } else if model.starts_with("glm") || model.starts_with("chatglm") {
        ProviderReasoningProtocol::GlmThinking
    } else {
        ProviderReasoningProtocol::ReasoningEffort
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelCapabilities {
    /// Image-input support reported by the provider's model catalog. `None`
    /// means the endpoint did not publish modality metadata for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Whether the selected model accepts image inputs. This is the only
    /// capability users need to declare; transport support is probed at use.
    #[serde(default = "default_provider_supports_vision")]
    pub supports_vision: bool,
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
            supports_vision: default_provider_supports_vision(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// so provider construction consumes the stored v5 profile verbatim.
    pub fn migrate_adapter_profiles(&mut self) {
        for (model, profiles) in &mut self.adapter_profiles {
            for profile in profiles.values_mut() {
                if let Some(normalized) =
                    profile.clone().normalized_for(&self.base_url, model.trim())
                {
                    *profile = normalized;
                }
            }
        }
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
        if let Some(adapter) = self
            .model_settings
            .get(model)
            .and_then(|settings| settings.preferred_adapter)
            .filter(|adapter| allowed_contains(*adapter))
        {
            return adapter;
        }
        if let Some(adapter) = self
            .preferred_adapter
            .filter(|adapter| allowed_contains(*adapter))
        {
            return adapter;
        }
        if let Some(adapter) = allowed.iter().copied().find(|adapter| {
            self.adapter_profile_for_model_and_adapter(model, *adapter)
                .is_some()
        }) {
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
        if let Some(profiles) = self.adapter_profiles.get_mut(report.model.trim()) {
            profiles.remove(&ProviderAdapterKind::OpenAiChat);
            profiles.remove(&ProviderAdapterKind::OpenAiResponses);
        }
        for profile in report.adapter_profiles() {
            self.apply_adapter_profile(profile);
        }
        if self.preferred_adapter.is_none() {
            self.model_settings
                .entry(model)
                .or_default()
                .preferred_adapter
                .get_or_insert(selected_adapter);
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

    /// Resolves image-input support for the selected model. Legacy connections
    /// fall back to the connection-wide setting until their catalog supplies
    /// model-level metadata or the user creates an explicit override.
    pub fn supports_vision_for_model(&self) -> bool {
        self.current_model_settings()
            .and_then(|settings| settings.supports_vision)
            .or_else(|| {
                self.model_capabilities
                    .get(self.model.trim())
                    .and_then(|capabilities| capabilities.supports_vision)
            })
            .unwrap_or(self.supports_vision)
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
/// Unmatched models deliberately return `None` so the caller applies its
/// conservative default instead of guessing high and overflowing the context.
pub fn known_model_context_window_tokens(model: &str) -> Option<usize> {
    model_bases(model)
        .iter()
        .find_map(|base| context_window_for_base(base))
}

fn context_window_for_base(model: &str) -> Option<usize> {
    // Snapshot from https://openrouter.ai/api/v1/models, verified 2026-08-02.
    // Direct-provider aliases not currently present in that catalog are marked
    // below. This is deliberately an exact-ID table: a model-family prefix is
    // not a reliable context window contract.
    const WINDOWS: &[(&str, usize)] = &[
        // OpenAI (official docs and OpenRouter catalog)
        ("gpt-3.5-turbo", 16_385),
        ("gpt-3.5-turbo-0613", 4_095),
        ("gpt-3.5-turbo-16k", 16_385),
        ("gpt-3.5-turbo-instruct", 4_095),
        ("gpt-4", 8_191),
        ("gpt-4.1", 1_047_576),
        ("gpt-4.1-mini", 1_047_576),
        ("gpt-4.1-nano", 1_047_576),
        ("gpt-4o", 128_000),
        ("gpt-4o-2024-05-13", 128_000),
        ("gpt-4o-2024-08-06", 128_000),
        ("gpt-4o-2024-11-20", 128_000),
        ("gpt-4o-mini", 128_000),
        ("gpt-4o-mini-2024-07-18", 128_000),
        ("gpt-4-turbo", 128_000),
        ("gpt-4-turbo-preview", 128_000),
        ("gpt-5", 400_000),
        ("gpt-5-mini", 400_000),
        ("gpt-5-nano", 400_000),
        ("gpt-5-pro", 400_000),
        ("gpt-5.1", 400_000),
        ("gpt-5.1-codex", 400_000),
        ("gpt-5.1-codex-max", 400_000),
        ("gpt-5.1-codex-mini", 400_000),
        ("gpt-5.2", 400_000),
        ("gpt-5.2-chat", 128_000),
        ("gpt-5.2-codex", 400_000),
        ("gpt-5.2-pro", 400_000),
        ("gpt-5.3-chat", 128_000),
        ("gpt-5.3-codex", 400_000),
        ("gpt-5.4", 1_050_000),
        ("gpt-5.4-image-2", 272_000),
        ("gpt-5.4-mini", 400_000),
        ("gpt-5.4-nano", 400_000),
        ("gpt-5.4-pro", 1_050_000),
        ("gpt-5.5", 1_050_000),
        ("gpt-5.5-pro", 1_050_000),
        ("gpt-5.6", 1_050_000), // Direct-provider alias for GPT-5.6 Sol.
        ("gpt-5.6-luna", 1_050_000),
        ("gpt-5.6-luna-pro", 1_050_000),
        ("gpt-5.6-sol", 1_050_000),
        ("gpt-5.6-sol-pro", 1_050_000),
        ("gpt-5.6-terra", 1_050_000),
        ("gpt-5.6-terra-pro", 1_050_000),
        ("gpt-5-image", 400_000),
        ("gpt-5-image-mini", 400_000),
        ("gpt-audio", 128_000),
        ("gpt-audio-mini", 128_000),
        ("gpt-chat-latest", 400_000),
        ("gpt-oss-120b", 131_072),
        ("gpt-oss-20b", 131_072),
        ("gpt-oss-safeguard-20b", 131_072),
        ("o1", 200_000),
        ("o1-pro", 200_000),
        ("o3", 200_000),
        ("o3-mini", 200_000),
        ("o3-mini-high", 200_000),
        ("o3-pro", 200_000),
        ("o4-mini", 200_000),
        ("o4-mini-high", 200_000),
        // Anthropic
        ("claude-3-haiku", 200_000),
        ("claude-3-5-haiku", 200_000), // Direct-provider historical ID.
        ("claude-3-5-sonnet", 200_000), // Direct-provider historical ID.
        ("claude-3-7-sonnet", 200_000), // Direct-provider historical ID.
        ("claude-haiku-4.5", 200_000),
        ("claude-opus-4", 200_000),
        ("claude-opus-4.1", 200_000),
        ("claude-opus-4.5", 200_000),
        ("claude-opus-4-6", 1_000_000), // Direct-provider spelling.
        ("claude-opus-4.6", 1_000_000),
        ("claude-opus-4-7", 1_000_000), // Direct-provider spelling.
        ("claude-opus-4.7", 1_000_000),
        ("claude-opus-4.7-fast", 1_000_000),
        ("claude-opus-4-8", 1_000_000), // Direct-provider spelling.
        ("claude-opus-4.8", 1_000_000),
        ("claude-opus-4.8-fast", 1_000_000),
        ("claude-opus-5", 1_000_000),
        ("claude-opus-5-fast", 1_000_000),
        ("claude-sonnet-4", 1_000_000),
        ("claude-sonnet-4-5", 1_000_000), // Direct-provider spelling.
        ("claude-sonnet-4.5", 1_000_000),
        ("claude-sonnet-4-6", 1_000_000), // Direct-provider spelling.
        ("claude-sonnet-4.6", 1_000_000),
        ("claude-sonnet-5", 1_000_000),
        ("claude-fable-5", 1_000_000),
        // Google
        ("gemini-1.5-flash", 1_000_000), // Direct-provider historical ID.
        ("gemini-1.5-pro", 1_000_000),   // Direct-provider historical ID.
        ("gemini-2.0-flash", 1_000_000), // Direct-provider historical ID.
        ("gemini-2.0-flash-lite", 1_000_000), // Direct-provider historical ID.
        ("gemini-2.5-flash", 1_048_576),
        ("gemini-2.5-flash-image", 32_768),
        ("gemini-2.5-flash-lite", 1_048_576),
        ("gemini-2.5-pro", 1_048_576),
        ("gemini-2.5-pro-preview", 1_048_576),
        ("gemini-2.5-pro-preview-05-06", 1_048_576),
        ("gemini-3-flash-preview", 1_048_576),
        ("gemini-3-pro-image", 131_072),
        ("gemini-3-pro-image-preview", 65_536),
        ("gemini-3.1-flash-image", 131_072),
        ("gemini-3.1-flash-image-preview", 65_536),
        ("gemini-3.1-flash-lite", 1_048_576),
        ("gemini-3.1-flash-lite-image", 65_536),
        ("gemini-3.1-flash-lite-preview", 1_048_576),
        ("gemini-3.1-pro-preview", 1_048_576),
        ("gemini-3.1-pro-preview-customtools", 1_048_576),
        ("gemini-3.5-flash", 1_048_576),
        ("gemini-3.5-flash-lite", 1_048_576),
        ("gemini-3.6-flash", 1_048_576),
        ("gemma-2-27b-it", 8_192),
        ("gemma-3-4b-it", 131_072),
        ("gemma-3-12b-it", 131_072),
        ("gemma-3-27b-it", 262_144),
        ("gemma-3n-e4b-it", 32_768),
        ("gemma-4-26b-a4b-it", 262_144),
        ("gemma-4-31b-it", 262_144),
        // Moonshot / Kimi
        ("moonshot-v1", 8_000), // Direct-provider base tier.
        ("moonshot-v1-8k", 8_000),
        ("moonshot-v1-32k", 32_000),
        ("moonshot-v1-128k", 128_000),
        ("k3", 1_000_000),    // Direct-provider alias.
        ("k3-256k", 256_000), // Direct-provider tier.
        ("kimi-k2", 131_072),
        ("kimi-k2-0905", 262_144),
        ("kimi-k2-thinking", 262_144),
        ("kimi-k2.5", 262_144),
        ("kimi-k2.6", 262_144),
        ("kimi-k2.7-code", 262_144),
        ("kimi-k3", 1_048_576),
        ("kimi-k3-256k", 256_000), // Direct-provider tier.
        // DeepSeek
        ("deepseek-chat", 163_840),
        ("deepseek-chat-v3.1", 163_840),
        ("deepseek-chat-v3-0324", 163_840),
        ("deepseek-r1", 163_840),
        ("deepseek-r1-0528", 163_840),
        ("deepseek-r1-distill-llama-70b", 8_192),
        ("deepseek-reasoner", 163_840), // Direct-provider alias.
        ("deepseek-v3.1-terminus", 163_840),
        ("deepseek-v3.2", 163_840),
        ("deepseek-v3.2-exp", 163_840),
        ("deepseek-v4-flash", 1_048_576),
        ("deepseek-v4-flash-0731", 1_048_576),
        ("deepseek-v4-pro", 1_048_576),
        // Qwen / Alibaba
        ("qwen-2.5-7b-instruct", 32_768),
        ("qwen-2.5-72b-instruct", 32_768),
        ("qwen-2.5-coder-32b-instruct", 32_768),
        ("qwen2.5-vl-72b-instruct", 128_000),
        ("qwen-plus", 1_000_000),
        ("qwen-plus-2025-07-28", 1_000_000),
        ("qwen3-8b", 131_072),
        ("qwen3-14b", 131_072),
        ("qwen3-30b-a3b", 131_072),
        ("qwen3-32b", 131_072),
        ("qwen3-235b-a22b", 131_072),
        ("qwen3-235b-a22b-2507", 262_144),
        ("qwen3-235b-a22b-thinking-2507", 262_144),
        ("qwen3-30b-a3b-instruct-2507", 262_144),
        ("qwen3-30b-a3b-thinking-2507", 81_920),
        ("qwen3-coder", 262_144),
        ("qwen3-coder-30b-a3b-instruct", 262_144),
        ("qwen3-coder-flash", 1_000_000),
        ("qwen3-coder-next", 262_144),
        ("qwen3-coder-plus", 1_000_000),
        ("qwen3-max", 262_144),
        ("qwen3-max-thinking", 262_144),
        ("qwen3-next-80b-a3b-instruct", 262_144),
        ("qwen3-next-80b-a3b-thinking", 262_144),
        ("qwen3-vl-8b-instruct", 262_144),
        ("qwen3-vl-8b-thinking", 131_072),
        ("qwen3-vl-32b-instruct", 131_072),
        ("qwen3-vl-30b-a3b-instruct", 262_144),
        ("qwen3-vl-30b-a3b-thinking", 262_144),
        ("qwen3-vl-235b-a22b-instruct", 262_144),
        ("qwen3-vl-235b-a22b-thinking", 131_072),
        ("qwen3.5-9b", 262_144),
        ("qwen3.5-27b", 262_144),
        ("qwen3.5-35b-a3b", 262_144),
        ("qwen3.5-122b-a10b", 262_144),
        ("qwen3.5-397b-a17b", 262_144),
        ("qwen3.5-flash-02-23", 1_000_000),
        ("qwen3.5-plus-02-15", 1_000_000),
        ("qwen3.5-plus-20260420", 1_000_000),
        ("qwen3.6-27b", 262_144),
        ("qwen3.6-35b-a3b", 262_144),
        ("qwen3.6-flash", 1_000_000),
        ("qwen3.6-max-preview", 262_144),
        ("qwen3.6-plus", 1_000_000),
        ("qwen3.7-flash", 1_000_000),
        ("qwen3.7-max", 1_000_000),
        ("qwen3.7-plus", 1_000_000),
        // Z.AI GLM
        ("glm-4.5", 131_072),
        ("glm-4.5-air", 131_072),
        ("glm-4.5v", 65_536),
        ("glm-4.6", 204_800),
        ("glm-4.6v", 131_072),
        ("glm-4.7", 204_800),
        ("glm-4.7-flash", 202_752),
        ("glm-5", 204_800),
        ("glm-5-turbo", 202_752),
        ("glm-5v-turbo", 202_752),
        ("glm-5.1", 204_800),
        ("glm-5.2", 1_048_576),
        // xAI Grok
        ("grok-4.20", 2_000_000),
        ("grok-4.20-multi-agent", 2_000_000),
        ("grok-4.3", 1_000_000),
        ("grok-4.5", 500_000),
        ("grok-build-0.1", 256_000),
        // Mistral
        ("codestral-2508", 256_000),
        ("ministral-3b-2512", 131_072),
        ("ministral-8b-2512", 262_144),
        ("ministral-14b-2512", 262_144),
        ("mistral-large", 128_000),
        ("mistral-large-2407", 131_072),
        ("mistral-large-2512", 262_144),
        ("mistral-medium-3", 131_072),
        ("mistral-medium-3.1", 131_072),
        ("mistral-medium-3-5", 262_144),
        ("mistral-nemo", 131_072),
        ("mistral-saba", 32_768),
        ("mistral-small-24b-instruct-2501", 32_768),
        ("mistral-small-3.1-24b-instruct", 128_000),
        ("mistral-small-3.2-24b-instruct", 256_000),
        ("mistral-small-2603", 262_144),
        ("mixtral-8x22b-instruct", 65_536),
        ("voxtral-small-24b-2507", 32_000),
        // Meta Llama
        ("llama-3.1-8b-instruct", 131_072),
        ("llama-3.1-70b-instruct", 131_072),
        ("llama-3.2-1b-instruct", 60_000),
        ("llama-3.2-3b-instruct", 131_072),
        ("llama-3.3-70b-instruct", 131_072),
        ("llama-4-maverick", 1_048_576),
        ("llama-4-scout", 1_310_720),
        ("llama-guard-4-12b", 1_048_576),
        // MiniMax
        ("minimax-01", 1_000_192),
        ("minimax-m1", 1_000_000),
        ("minimax-m2", 204_800),
        ("minimax-m2.1", 204_800),
        ("minimax-m2.5", 204_800),
        ("minimax-m2.7", 204_800),
        ("minimax-m2-her", 65_536),
        ("minimax-m3", 1_048_576),
    ];

    WINDOWS
        .iter()
        .find_map(|(id, context_window)| (*id == model).then_some(*context_window))
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

fn default_provider_supports_vision() -> bool {
    true
}

fn default_parallel_tool_calls() -> bool {
    true
}

fn default_rollout_sampling_token_weight() -> f64 {
    1.0
}

fn default_rollout_prefill_token_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxEnforcement {
    Disabled,
    BestEffort,
    Enforce,
}

impl Default for SandboxEnforcement {
    fn default() -> Self {
        Self::Enforce
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSettings {
    pub sandbox_mode: SandboxMode,
    pub enforcement: SandboxEnforcement,
    pub network: NetworkPolicy,
    pub writable_roots: Vec<PathBuf>,
    pub read_paths: Vec<PathBuf>,
    pub windows_backend: WindowsSandboxBackend,
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            sandbox_mode: SandboxMode::WorkspaceWrite,
            enforcement: SandboxEnforcement::Enforce,
            network: NetworkPolicy::Deny,
            writable_roots: Vec::new(),
            read_paths: Vec::new(),
            windows_backend: WindowsSandboxBackend::Auto,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SandboxSettingsWire {
    sandbox_mode: Option<String>,
    enforcement: Option<String>,
    network: Option<String>,
    writable_roots: Vec<PathBuf>,
    read_paths: Vec<PathBuf>,
    windows_backend: Option<String>,
}

impl<'de> Deserialize<'de> for SandboxSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SandboxSettingsWire::deserialize(deserializer)?;
        let defaults = Self::default();
        let sandbox_mode = wire
            .sandbox_mode
            .as_deref()
            .map(parse_sandbox_mode)
            .unwrap_or(Some(defaults.sandbox_mode));
        let enforcement = wire
            .enforcement
            .as_deref()
            .map(parse_sandbox_enforcement)
            .unwrap_or(Some(defaults.enforcement));
        let network = wire
            .network
            .as_deref()
            .map(parse_sandbox_network)
            .unwrap_or(Some(defaults.network));
        let windows_backend = wire
            .windows_backend
            .as_deref()
            .map(parse_windows_sandbox_backend)
            .unwrap_or(Some(defaults.windows_backend));

        match (sandbox_mode, enforcement, network, windows_backend) {
            (Some(sandbox_mode), Some(enforcement), Some(network), Some(windows_backend)) => {
                Ok(Self {
                    sandbox_mode,
                    enforcement,
                    network,
                    writable_roots: wire.writable_roots,
                    read_paths: wire.read_paths,
                    windows_backend,
                })
            }
            _ => Ok(Self::fail_safe(wire.writable_roots, wire.read_paths)),
        }
    }
}

impl SandboxSettings {
    pub fn from_env() -> Self {
        let writable_roots = env_path_list("OPENTOPIA_SANDBOX_WRITABLE_ROOTS");
        let read_paths = env_path_list("OPENTOPIA_SANDBOX_READ_PATHS");
        let mode_value = std::env::var("OPENTOPIA_SANDBOX_MODE")
            .unwrap_or_else(|_| "workspace-write".to_string());
        let normalized_mode = normalize_sandbox_value(&mode_value);
        let (legacy_enforcement, sandbox_mode) = match normalized_mode.as_str() {
            "enforce" | "strict" => (
                Some(SandboxEnforcement::Enforce),
                SandboxMode::WorkspaceWrite,
            ),
            "best-effort" => (
                Some(SandboxEnforcement::BestEffort),
                SandboxMode::WorkspaceWrite,
            ),
            "disabled" => (
                Some(SandboxEnforcement::Disabled),
                SandboxMode::DangerFullAccess,
            ),
            _ => match parse_sandbox_mode(&normalized_mode) {
                Some(sandbox_mode) => (None, sandbox_mode),
                None => return Self::fail_safe(writable_roots, read_paths),
            },
        };

        let enforcement = match std::env::var("OPENTOPIA_SANDBOX_ENFORCEMENT") {
            Ok(value) => match parse_sandbox_enforcement(&value) {
                Some(enforcement) => enforcement,
                None => return Self::fail_safe(writable_roots, read_paths),
            },
            Err(_) => legacy_enforcement.unwrap_or_else(|| {
                if sandbox_mode == SandboxMode::DangerFullAccess {
                    SandboxEnforcement::Disabled
                } else {
                    SandboxEnforcement::Enforce
                }
            }),
        };
        let network = match std::env::var("OPENTOPIA_SANDBOX_NETWORK") {
            Ok(value) => match parse_sandbox_network(&value) {
                Some(network) => network,
                None => return Self::fail_safe(writable_roots, read_paths),
            },
            Err(_) => NetworkPolicy::Deny,
        };
        let windows_backend = match std::env::var("OPENTOPIA_WINDOWS_SANDBOX") {
            Ok(value) => match parse_windows_sandbox_backend(&value) {
                Some(backend) => backend,
                None => return Self::fail_safe(writable_roots, read_paths),
            },
            Err(_) => WindowsSandboxBackend::Auto,
        };

        Self {
            sandbox_mode,
            enforcement,
            network,
            writable_roots,
            read_paths,
            windows_backend,
        }
    }

    pub fn to_local_sandbox_config(&self) -> LocalSandboxConfig {
        if self.sandbox_mode == SandboxMode::DangerFullAccess {
            return LocalSandboxConfig {
                read_paths: self.read_paths.clone(),
                writable_roots: self.writable_roots.clone(),
                ..LocalSandboxConfig::danger_full_access()
            };
        }

        let mode = match self.enforcement {
            SandboxEnforcement::Disabled => OsSandboxMode::Disabled,
            SandboxEnforcement::BestEffort => OsSandboxMode::BestEffort,
            SandboxEnforcement::Enforce => OsSandboxMode::Enforce,
        };
        LocalSandboxConfig {
            enabled: mode != OsSandboxMode::Disabled,
            mode,
            network: self.network,
            read_paths: self.read_paths.clone(),
            write_paths: Vec::new(),
            sandbox_mode: self.sandbox_mode,
            writable_roots: self.writable_roots.clone(),
            sandbox_home: None,
            windows_backend: self.windows_backend,
            approved_read_paths: Vec::new(),
            approved_write_paths: Vec::new(),
        }
    }

    fn fail_safe(writable_roots: Vec<PathBuf>, read_paths: Vec<PathBuf>) -> Self {
        Self {
            sandbox_mode: SandboxMode::ReadOnly,
            enforcement: SandboxEnforcement::Enforce,
            network: NetworkPolicy::Deny,
            writable_roots,
            read_paths,
            windows_backend: WindowsSandboxBackend::Auto,
        }
    }
}

impl From<&SandboxSettings> for LocalSandboxConfig {
    fn from(settings: &SandboxSettings) -> Self {
        settings.to_local_sandbox_config()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseSettings {
    /// Deployment-owned gate. It is intentionally not writable through the
    /// settings API, so a consumer session cannot enable enterprise surfaces.
    pub enabled: bool,
}

impl Default for EnterpriseSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl EnterpriseSettings {
    pub fn from_env() -> Self {
        let value = std::env::var("OPENTOPIA_ENTERPRISE_ENABLED").ok();
        let enabled = enterprise_enabled_from_env_value(value.as_deref());
        Self { enabled }
    }
}

fn enterprise_enabled_from_env_value(value: Option<&str>) -> bool {
    value.map_or_else(
        || EnterpriseSettings::default().enabled,
        |value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        },
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    pub providers: Vec<ProviderSettings>,
    #[serde(default)]
    pub active_provider_id: String,
    pub permission_mode: PermissionMode,
    #[serde(default)]
    pub agent_runtime: AgentRuntimeSettings,
    #[serde(default)]
    pub default_workspace_root: Option<PathBuf>,
    #[serde(default)]
    pub sandbox: SandboxSettings,
    #[serde(default)]
    pub enterprise: EnterpriseSettings,
    /// One-time settings migration marker. Older desktop builds persisted
    /// `parallelToolCalls: false` as their UI default, so absence means those
    /// values have not yet been upgraded to the runtime's default-on policy.
    #[serde(default)]
    pub parallel_tool_calls_migrated: bool,
    pub updated_at: DateTime<Utc>,
}

impl AppSettings {
    pub fn from_env(permission_mode: PermissionMode) -> Self {
        let provider = ProviderSettings::from_env();
        Self {
            providers: vec![provider.clone()],
            active_provider_id: provider.id.clone(),
            permission_mode,
            agent_runtime: AgentRuntimeSettings::default(),
            default_workspace_root: None,
            sandbox: SandboxSettings::from_env(),
            enterprise: EnterpriseSettings::from_env(),
            parallel_tool_calls_migrated: true,
            updated_at: Utc::now(),
        }
    }

    pub fn active_provider(&self) -> &ProviderSettings {
        self.providers
            .iter()
            .find(|p| p.id == self.active_provider_id)
            .or_else(|| self.providers.first())
            .expect("AppSettings has no providers configured")
    }

    /// Resolves the connection a thread pinned. Falls back to the active
    /// connection when the pin refers to a connection the user has since
    /// deleted, so an old thread stays usable instead of erroring.
    pub fn provider_by_id_or_active(&self, connection_id: Option<&str>) -> &ProviderSettings {
        connection_id
            .and_then(|id| self.providers.iter().find(|provider| provider.id == id))
            .unwrap_or_else(|| self.active_provider())
    }

    pub fn active_provider_mut(&mut self) -> &mut ProviderSettings {
        let id = self.active_provider_id.clone();
        if self.providers.is_empty() {
            self.providers.push(ProviderSettings::default());
            self.active_provider_id = self.providers[0].id.clone();
        }
        let pos = self.providers.iter().position(|p| p.id == id).unwrap_or(0);
        &mut self.providers[pos]
    }

    pub fn touch(&mut self) {
        for provider in &mut self.providers {
            provider.migrate_legacy_connection_axes();
            provider.migrate_legacy_openai_compatibility_report();
            provider.migrate_adapter_profiles();
            provider.api_key_configured = match provider.effective_auth() {
                ProviderAuthKind::CodexSession | ProviderAuthKind::None => true,
                ProviderAuthKind::Bearer | ProviderAuthKind::XApiKey => {
                    std::env::var(&provider.api_key_source).is_ok_and(|value| !value.is_empty())
                }
            };
        }
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealth {
    pub id: String,
    /// Legacy preset identity for older desktop builds.
    pub kind: ProviderKind,
    pub transport: ProviderTransportKind,
    pub auth: ProviderAuthKind,
    pub adapter: ProviderAdapterKind,
    pub base_url: String,
    pub model: String,
    pub api_key_source: String,
    pub api_key_configured: bool,
    pub using_mock: bool,
    pub status: String,
}

impl ProviderHealth {
    pub fn from_settings(settings: &ProviderSettings) -> Self {
        let transport = settings.effective_transport();
        let auth = settings.effective_auth();
        let adapter = settings.resolved_adapter_for_model(&settings.model);
        let local_or_anonymous = matches!(
            auth,
            ProviderAuthKind::CodexSession | ProviderAuthKind::None
        );
        let api_key_configured = local_or_anonymous
            || std::env::var(&settings.api_key_source).is_ok_and(|value| !value.is_empty())
            || settings.api_key_configured;
        let using_mock = transport == ProviderTransportKind::Mock
            || (!local_or_anonymous && !api_key_configured);
        let needs_negotiation =
            transport == ProviderTransportKind::Http && settings.active_adapter_profile().is_none();
        Self {
            id: settings.id.clone(),
            kind: settings.kind,
            transport,
            auth,
            adapter,
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key_source: settings.api_key_source.clone(),
            api_key_configured,
            using_mock,
            status: if transport == ProviderTransportKind::CodexAppServer {
                "local_codex".to_string()
            } else if using_mock {
                "mock_or_unconfigured".to_string()
            } else if needs_negotiation {
                "needs_negotiation".to_string()
            } else {
                "ready".to_string()
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthCheck {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub model_available: bool,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_compatibility: Option<OpenAiCompatibilityReport>,
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

fn normalize_sandbox_value(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "-")
}

fn parse_sandbox_mode(value: &str) -> Option<SandboxMode> {
    match normalize_sandbox_value(value).as_str() {
        "read-only" => Some(SandboxMode::ReadOnly),
        "workspace-write" => Some(SandboxMode::WorkspaceWrite),
        "danger-full-access" => Some(SandboxMode::DangerFullAccess),
        _ => None,
    }
}

fn parse_sandbox_enforcement(value: &str) -> Option<SandboxEnforcement> {
    match normalize_sandbox_value(value).as_str() {
        "disabled" => Some(SandboxEnforcement::Disabled),
        "best-effort" => Some(SandboxEnforcement::BestEffort),
        "enforce" | "strict" => Some(SandboxEnforcement::Enforce),
        _ => None,
    }
}

fn parse_sandbox_network(value: &str) -> Option<NetworkPolicy> {
    match normalize_sandbox_value(value).as_str() {
        "inherit" => Some(NetworkPolicy::Inherit),
        "allow" => Some(NetworkPolicy::Allow),
        "deny" => Some(NetworkPolicy::Deny),
        _ => None,
    }
}

fn parse_windows_sandbox_backend(value: &str) -> Option<WindowsSandboxBackend> {
    match normalize_sandbox_value(value).as_str() {
        "auto" => Some(WindowsSandboxBackend::Auto),
        "dedicated_user" | "dedicated-user" | "elevated" => {
            Some(WindowsSandboxBackend::DedicatedUser)
        }
        "unelevated" | "legacy" => Some(WindowsSandboxBackend::Unelevated),
        _ => None,
    }
}

fn env_path_list(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{OsStr, OsString};

    #[test]
    fn explicit_prompt_cache_is_gated_to_official_gpt_5_6_or_later() {
        assert_eq!(
            official_openai_explicit_prompt_cache_support(
                "https://api.openai.com/v1",
                "gpt-5.6-sol"
            ),
            ProviderFeatureSupport::Supported
        );
        assert_eq!(
            official_openai_explicit_prompt_cache_support("https://api.openai.com/v1/", "gpt-6"),
            ProviderFeatureSupport::Supported
        );
        assert_eq!(
            official_openai_explicit_prompt_cache_support("https://api.openai.com/v1", "gpt-5.4"),
            ProviderFeatureSupport::Unknown
        );
        assert_eq!(
            official_openai_explicit_prompt_cache_support("https://relay.example/v1", "gpt-5.6"),
            ProviderFeatureSupport::Unknown
        );
    }

    #[test]
    fn enterprise_flow_is_enabled_by_default_and_can_be_disabled() {
        assert!(EnterpriseSettings::default().enabled);
        assert!(enterprise_enabled_from_env_value(None));
        for value in ["1", "true", "YES", " on "] {
            assert!(enterprise_enabled_from_env_value(Some(value)));
        }
        for value in ["0", "false", "no", "off", "invalid"] {
            assert!(!enterprise_enabled_from_env_value(Some(value)));
        }
    }

    #[test]
    fn context_window_table_uses_verified_exact_model_ids() {
        assert_eq!(
            known_model_context_window_tokens("gpt-5.6-sol"),
            Some(1_050_000)
        );
        assert_eq!(
            known_model_context_window_tokens("gpt-5.5-pro"),
            Some(1_050_000)
        );
        assert_eq!(
            known_model_context_window_tokens("gpt-5.4"),
            Some(1_050_000)
        );
        assert_eq!(
            known_model_context_window_tokens("gpt-5.4-mini"),
            Some(400_000)
        );
        assert_eq!(
            known_model_context_window_tokens("gpt-5.3-codex"),
            Some(400_000)
        );
        assert_eq!(known_model_context_window_tokens("gpt-5"), Some(400_000));
        assert_eq!(
            known_model_context_window_tokens("gpt-4.1-mini"),
            Some(1_047_576)
        );
        assert_eq!(known_model_context_window_tokens("o3-mini"), Some(200_000));
        assert_eq!(
            known_model_context_window_tokens("anthropic/claude-sonnet-4-5"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("kimi-k2.5"),
            Some(262_144)
        );
        assert_eq!(known_model_context_window_tokens("kimi-k2"), Some(131_072));
        assert_eq!(known_model_context_window_tokens("k3-256k"), Some(256_000));
        assert_eq!(
            known_model_context_window_tokens("glm-5.2"),
            Some(1_048_576)
        );
        assert_eq!(known_model_context_window_tokens("glm-5"), Some(204_800));
        assert_eq!(
            known_model_context_window_tokens("claude-sonnet-4-6"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("kimi-k3"),
            Some(1_048_576)
        );
        assert_eq!(
            known_model_context_window_tokens("moonshot-v1-32k"),
            Some(32_000)
        );
        assert_eq!(
            known_model_context_window_tokens("moonshot-v1-8k"),
            Some(8_000)
        );
        assert_eq!(
            known_model_context_window_tokens("deepseek-v4-flash"),
            Some(1_048_576)
        );
        assert_eq!(
            known_model_context_window_tokens("deepseek-reasoner"),
            Some(163_840)
        );
        assert_eq!(
            known_model_context_window_tokens("gemini-2.5-pro"),
            Some(1_048_576)
        );
        assert_eq!(
            known_model_context_window_tokens("qwen-2.5-72b-instruct"),
            Some(32_768)
        );
        assert_eq!(
            known_model_context_window_tokens("openrouter/qwen-plus:free"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("deepseek-r1-distill-llama-70b"),
            Some(8_192)
        );
        assert_eq!(
            known_model_context_window_tokens("gemini-2.5-flash-image"),
            Some(32_768)
        );
        assert_eq!(
            known_model_context_window_tokens("grok-4.20"),
            Some(2_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("mistral-small-3.2-24b-instruct"),
            Some(256_000)
        );
        assert_eq!(
            known_model_context_window_tokens("llama-4-scout"),
            Some(1_310_720)
        );
        assert_eq!(
            known_model_context_window_tokens("minimax-m2"),
            Some(204_800)
        );
        assert_eq!(known_model_context_window_tokens("glm-4.6"), Some(204_800));
        assert_eq!(
            known_model_context_window_tokens("qwen3-14b-unpublished"),
            None
        );
        assert_eq!(known_model_context_window_tokens("my-finetune-v3"), None);
    }

    #[test]
    fn a_window_reported_by_the_endpoint_outranks_the_builtin_table() {
        let mut provider = ProviderSettings {
            model: "kimi-k2.5".to_string(),
            ..ProviderSettings::default()
        };
        // The table would say 256K; the endpoint knows this deployment is 1M.
        provider
            .model_context_windows
            .insert("kimi-k2.5".to_string(), 1_000_000);
        assert_eq!(provider.resolved_context_window_tokens(), 1_000_000);

        // An explicit user override still beats both.
        provider.context_window_tokens = Some(64_000);
        assert_eq!(provider.resolved_context_window_tokens(), 64_000);
    }

    #[test]
    fn model_vision_settings_override_catalog_detection_and_legacy_default() {
        let mut provider = ProviderSettings {
            model: "text-only".to_string(),
            supports_vision: true,
            ..ProviderSettings::default()
        };
        provider.model_capabilities.insert(
            "text-only".to_string(),
            ProviderModelCapabilities {
                supports_vision: Some(false),
            },
        );
        assert!(!provider.supports_vision_for_model());

        provider.model_settings.insert(
            "text-only".to_string(),
            ProviderModelSettings {
                supports_vision: Some(true),
                ..ProviderModelSettings::default()
            },
        );
        assert!(provider.supports_vision_for_model());

        provider.model = "unknown".to_string();
        assert!(provider.supports_vision_for_model());
    }

    #[test]
    fn model_generation_settings_override_connection_defaults() {
        let mut provider = ProviderSettings {
            model: "legacy-model".to_string(),
            temperature: Some(0.2),
            max_output_tokens: Some(1_024),
            context_window_tokens: Some(32_000),
            reasoning_effort: Some("low".to_string()),
            ..ProviderSettings::default()
        };
        provider.model_settings.insert(
            "configured-model".to_string(),
            ProviderModelSettings {
                temperature: Some(Some(0.7)),
                max_output_tokens: Some(Some(8_192)),
                context_window_tokens: Some(Some(128_000)),
                reasoning_effort: Some(Some("high".to_string())),
                ..ProviderModelSettings::default()
            },
        );

        let selected = provider.with_model_override(Some("configured-model"), None);
        assert_eq!(selected.temperature_for_model(), Some(0.7));
        assert_eq!(selected.max_output_tokens_for_model(), Some(8_192));
        assert_eq!(selected.resolved_context_window_tokens(), 128_000);
        assert_eq!(
            selected.reasoning_effort_for_model(),
            Some("high".to_string())
        );

        assert_eq!(provider.temperature_for_model(), Some(0.2));
        assert_eq!(provider.max_output_tokens_for_model(), Some(1_024));
        assert_eq!(provider.resolved_context_window_tokens(), 32_000);
        assert_eq!(
            provider.reasoning_effort_for_model(),
            Some("low".to_string())
        );
    }

    #[test]
    fn an_unknown_model_without_reported_metadata_uses_the_conservative_default() {
        let provider = ProviderSettings {
            model: "some-relay-only-model".to_string(),
            ..ProviderSettings::default()
        };
        assert_eq!(
            provider.resolved_context_window_tokens(),
            DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS
        );
    }

    const SANDBOX_ENV_KEYS: [&str; 5] = [
        "OPENTOPIA_SANDBOX_MODE",
        "OPENTOPIA_SANDBOX_ENFORCEMENT",
        "OPENTOPIA_SANDBOX_NETWORK",
        "OPENTOPIA_SANDBOX_WRITABLE_ROOTS",
        "OPENTOPIA_SANDBOX_READ_PATHS",
    ];

    struct EnvGuard(Vec<(&'static str, Option<OsString>)>);

    impl EnvGuard {
        fn cleared(keys: &'static [&'static str]) -> Self {
            let values = keys
                .iter()
                .map(|key| {
                    let value = std::env::var_os(key);
                    std::env::remove_var(key);
                    (*key, value)
                })
                .collect();
            Self(values)
        }

        fn set(&self, key: &str, value: impl AsRef<OsStr>) {
            std::env::set_var(key, value);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn old_app_settings_json_uses_sandbox_defaults() {
        let settings: AppSettings = serde_json::from_str(
            r#"{
                "providers": [],
                "activeProviderId": "",
                "permissionMode": "auto",
                "defaultWorkspaceRoot": null,
                "updatedAt": "2026-01-01T00:00:00Z"
            }"#,
        )
        .expect("deserialize settings without sandbox");

        assert_eq!(settings.sandbox, SandboxSettings::default());
        assert_eq!(settings.agent_runtime, AgentRuntimeSettings::default());
    }

    #[test]
    fn legacy_provider_json_uses_generation_defaults() {
        let provider: ProviderSettings = serde_json::from_str(
            r#"{
                "id": "legacy",
                "kind": "openai_compatible",
                "baseUrl": "https://example.test/v1",
                "model": "legacy-model",
                "apiKeySource": "OPENTOPIA_API_KEY",
                "apiKeyConfigured": true,
                "healthStatus": null
            }"#,
        )
        .expect("deserialize provider without generation settings");

        assert_eq!(provider.temperature, None);
        assert_eq!(provider.name, "");
        assert_eq!(provider.display_name(), "legacy");
        assert_eq!(provider.max_output_tokens, None);
        assert_eq!(provider.context_window_tokens, None);
        assert_eq!(
            provider.resolved_context_window_tokens(),
            DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS
        );
        assert_eq!(provider.reasoning_effort, None);
        assert!(!provider.store_responses);
        assert!(provider.parallel_tool_calls);
        assert_eq!(provider.prompt_cache_key, None);
        assert_eq!(provider.prompt_cache_policy, None);
        assert_eq!(provider.responses_compaction_threshold_tokens, None);
        assert_eq!(provider.rollout_budget, None);
        assert_eq!(provider.openai_compatibility, None);
    }

    #[test]
    fn explicit_parallel_tool_call_disable_is_preserved() {
        let provider: ProviderSettings = serde_json::from_str(
            r#"{
                "id": "serial",
                "kind": "openai_compatible",
                "baseUrl": "https://example.test/v1",
                "model": "serial-model",
                "parallelToolCalls": false,
                "apiKeySource": "OPENTOPIA_API_KEY",
                "apiKeyConfigured": true,
                "healthStatus": null
            }"#,
        )
        .expect("deserialize explicit serial provider setting");

        assert!(!provider.parallel_tool_calls);
    }

    #[test]
    fn adapter_profiles_round_trip_per_model_and_reject_stale_settings() {
        let mut provider = ProviderSettings::default();
        provider.base_url = "https://relay.example/v1".to_string();
        provider.model = "relay-model".to_string();
        provider.apply_openai_compatibility_report(OpenAiCompatibilityReport {
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            selected_protocol: OpenAiProtocol::ChatCompletions,
            chat_completions: ProviderFeatureSupport::Supported,
            chat_function_tools: ProviderFeatureSupport::Supported,
            chat_strict_function_tools: ProviderFeatureSupport::Unsupported,
            chat_streaming_tools: ProviderFeatureSupport::Supported,
            chat_parallel_tool_calls: ProviderFeatureSupport::Unsupported,
            chat_json_schema_output: ProviderFeatureSupport::Unsupported,
            chat_message_protocol: ProviderMessageProtocolCapabilities::default(),
            responses: ProviderFeatureSupport::Unsupported,
            responses_native_tools: ProviderFeatureSupport::Unsupported,
            responses_function_tools: ProviderFeatureSupport::Unknown,
            responses_strict_function_tools: ProviderFeatureSupport::Unknown,
            responses_streaming_tools: ProviderFeatureSupport::Unknown,
            responses_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Unknown,
            responses_apply_patch: ProviderFeatureSupport::Unknown,
            developer_messages: ProviderFeatureSupport::Unsupported,
            message_compatibility: true,
            checked_at: Utc::now(),
            notes: vec!["developer messages: HTTP 400".to_string()],
        });
        provider.apply_openai_compatibility_report(OpenAiCompatibilityReport {
            base_url: provider.base_url.clone(),
            model: "responses-model".to_string(),
            selected_protocol: OpenAiProtocol::Responses,
            chat_completions: ProviderFeatureSupport::Supported,
            chat_function_tools: ProviderFeatureSupport::Supported,
            chat_strict_function_tools: ProviderFeatureSupport::Unsupported,
            chat_streaming_tools: ProviderFeatureSupport::Supported,
            chat_parallel_tool_calls: ProviderFeatureSupport::Unsupported,
            chat_json_schema_output: ProviderFeatureSupport::Unsupported,
            chat_message_protocol: ProviderMessageProtocolCapabilities::default(),
            responses: ProviderFeatureSupport::Supported,
            responses_native_tools: ProviderFeatureSupport::Supported,
            responses_function_tools: ProviderFeatureSupport::Supported,
            responses_strict_function_tools: ProviderFeatureSupport::Supported,
            responses_streaming_tools: ProviderFeatureSupport::Supported,
            responses_parallel_tool_calls: ProviderFeatureSupport::Supported,
            responses_json_schema_output: ProviderFeatureSupport::Supported,
            responses_custom_tools: ProviderFeatureSupport::Supported,
            responses_apply_patch: ProviderFeatureSupport::Supported,
            developer_messages: ProviderFeatureSupport::Unsupported,
            message_compatibility: false,
            checked_at: Utc::now(),
            notes: Vec::new(),
        });

        let encoded = serde_json::to_string(&provider).unwrap();
        let restored: ProviderSettings = serde_json::from_str(&encoded).unwrap();
        let chat = restored.active_adapter_profile().unwrap();

        assert_eq!(chat.adapter, ProviderAdapterKind::OpenAiChat);
        assert_eq!(
            chat.instruction_encoding,
            ProviderInstructionEncoding::PortableChatEnvelope
        );
        assert!(chat.applies_to("https://relay.example/v1/", "relay-model"));
        assert!(!chat.applies_to("https://other.example/v1", "relay-model"));

        let responses = restored.with_model_override(Some("responses-model"), None);
        assert_eq!(
            responses.kind,
            ProviderKind::OpenAiCompatible,
            "protocol negotiation must not rewrite the connection preset"
        );
        assert_eq!(
            responses.active_adapter_profile().unwrap().adapter,
            ProviderAdapterKind::OpenAiResponses
        );
        assert!(responses
            .adapter_profile_for_model_and_adapter(
                "responses-model",
                ProviderAdapterKind::OpenAiChat,
            )
            .is_some());
        assert!(responses
            .adapter_profile_for_model_and_adapter(
                "responses-model",
                ProviderAdapterKind::OpenAiResponses,
            )
            .is_some());
        assert!(restored.adapter_profile_for_model("other-model").is_none());

        let legacy_chat_profile = provider
            .adapter_profile_for_model_and_adapter("relay-model", ProviderAdapterKind::OpenAiChat)
            .unwrap();
        let mut legacy_encoded = serde_json::to_value(&provider).unwrap();
        legacy_encoded["adapterProfiles"] = serde_json::json!({
            "relay-model": legacy_chat_profile
        });
        let migrated: ProviderSettings = serde_json::from_value(legacy_encoded).unwrap();
        assert!(migrated
            .adapter_profile_for_model_and_adapter("relay-model", ProviderAdapterKind::OpenAiChat,)
            .is_some());
    }

    #[test]
    fn connection_auth_and_thread_adapter_are_independent_from_legacy_kind() {
        let mut connection = ProviderSettings::default();
        connection.kind = ProviderKind::OpenAiCompatible;
        connection.transport = Some(ProviderTransportKind::Http);
        connection.auth = Some(ProviderAuthKind::Bearer);
        connection.allowed_adapters = vec![
            ProviderAdapterKind::OpenAiChat,
            ProviderAdapterKind::OpenAiResponses,
            ProviderAdapterKind::AnthropicMessages,
        ];

        let anthropic_route = connection.with_model_route_override(
            Some("claude-relay"),
            None,
            Some(ProviderAdapterKind::AnthropicMessages),
        );
        assert_eq!(anthropic_route.effective_auth(), ProviderAuthKind::Bearer);
        assert_eq!(
            anthropic_route.resolved_route().adapter,
            ProviderAdapterKind::AnthropicMessages
        );
        assert_eq!(connection.kind, ProviderKind::OpenAiCompatible);
        assert_eq!(
            connection.resolved_adapter_for_model(&connection.model),
            ProviderAdapterKind::OpenAiChat
        );
    }

    #[test]
    fn legacy_provider_kind_materializes_independent_connection_axes() {
        let mut provider: ProviderSettings = serde_json::from_value(serde_json::json!({
            "id": "legacy-anthropic",
            "name": "Legacy Anthropic",
            "kind": "anthropic",
            "baseUrl": "https://api.anthropic.com",
            "model": "claude-test",
            "apiKeySource": "ANTHROPIC_API_KEY",
            "apiKeyConfigured": true,
            "healthStatus": null
        }))
        .unwrap();

        assert_eq!(provider.effective_transport(), ProviderTransportKind::Http);
        assert_eq!(provider.effective_auth(), ProviderAuthKind::XApiKey);
        provider.migrate_legacy_connection_axes();
        assert_eq!(provider.transport, Some(ProviderTransportKind::Http));
        assert_eq!(provider.auth, Some(ProviderAuthKind::XApiKey));
        assert_eq!(
            provider.allowed_adapters,
            vec![ProviderAdapterKind::AnthropicMessages]
        );

        provider.apply_legacy_kind_preset(ProviderKind::OpenAiResponses);
        assert_eq!(provider.transport, Some(ProviderTransportKind::Http));
        assert_eq!(provider.auth, Some(ProviderAuthKind::Bearer));
        assert_eq!(
            provider.allowed_adapters,
            vec![
                ProviderAdapterKind::OpenAiChat,
                ProviderAdapterKind::OpenAiResponses,
            ]
        );
        assert_eq!(
            provider.preferred_adapter,
            Some(ProviderAdapterKind::OpenAiResponses)
        );
    }

    #[test]
    fn trusted_chat_message_contract_is_scoped_to_the_exact_vendor_endpoint() {
        assert!(
            trusted_chat_message_protocol_contract(
                "https://api.deepseek.com/v1",
                "deepseek-v4-flash-0731"
            )
            .unwrap()
            .requires_reasoning_content_for_tool_calls
        );
        assert!(
            trusted_chat_message_protocol_contract(
                "https://api.deepseek.com/v1/",
                "deepseek/deepseek-reasoner"
            )
            .unwrap()
            .requires_reasoning_content_for_tool_calls
        );
        assert!(trusted_chat_message_protocol_contract(
            "https://relay.example/v1",
            "deepseek-v4-flash"
        )
        .is_none());
        assert!(
            !trusted_chat_message_protocol_contract(
                "https://api.openai.com/v1",
                "opaque-chat-model"
            )
            .unwrap()
            .requires_reasoning_content_for_tool_calls
        );
    }

    #[test]
    fn legacy_chat_profiles_require_a_trusted_contract_or_renegotiation() {
        let legacy_profile = |base_url: &str, model: &str| ProviderAdapterProfile {
            profile_version: PREVIOUS_PROVIDER_ADAPTER_PROFILE_VERSION,
            base_url: base_url.to_string(),
            model: model.to_string(),
            adapter: ProviderAdapterKind::OpenAiChat,
            instruction_encoding: ProviderInstructionEncoding::PortableChatEnvelope,
            reasoning_protocol: ProviderReasoningProtocol::ReasoningEffort,
            message_protocol: ProviderMessageProtocolCapabilities::default(),
            output_protocol: ProviderOutputProtocolCapabilities::default(),
            tool_protocol: ProviderToolProtocolCapabilities::default(),
            checked_at: Utc::now(),
        };

        let mut direct = ProviderSettings {
            base_url: "https://api.deepseek.com/v1".to_string(),
            model: "deepseek-v4-pro".to_string(),
            ..ProviderSettings::default()
        };
        direct.apply_adapter_profile(legacy_profile(&direct.base_url, &direct.model));
        let migrated = direct.active_adapter_profile().unwrap();
        assert_eq!(migrated.profile_version, PROVIDER_ADAPTER_PROFILE_VERSION);
        assert!(
            migrated
                .message_protocol
                .requires_reasoning_content_for_tool_calls
        );

        let mut relay = ProviderSettings {
            base_url: "https://relay.example/v1".to_string(),
            model: "opaque-model".to_string(),
            ..ProviderSettings::default()
        };
        relay.apply_adapter_profile(legacy_profile(&relay.base_url, &relay.model));
        relay.openai_compatibility = Some(OpenAiCompatibilityReport {
            base_url: relay.base_url.clone(),
            model: relay.model.clone(),
            selected_protocol: OpenAiProtocol::ChatCompletions,
            chat_completions: ProviderFeatureSupport::Supported,
            chat_function_tools: ProviderFeatureSupport::Supported,
            chat_strict_function_tools: ProviderFeatureSupport::Unknown,
            chat_streaming_tools: ProviderFeatureSupport::Unknown,
            chat_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            chat_json_schema_output: ProviderFeatureSupport::Unknown,
            chat_message_protocol: ProviderMessageProtocolCapabilities::default(),
            responses: ProviderFeatureSupport::Unknown,
            responses_native_tools: ProviderFeatureSupport::Unknown,
            responses_function_tools: ProviderFeatureSupport::Unknown,
            responses_strict_function_tools: ProviderFeatureSupport::Unknown,
            responses_streaming_tools: ProviderFeatureSupport::Unknown,
            responses_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Unknown,
            responses_apply_patch: ProviderFeatureSupport::Unknown,
            developer_messages: ProviderFeatureSupport::Unknown,
            message_compatibility: true,
            checked_at: Utc::now(),
            notes: Vec::new(),
        });
        assert!(relay.active_adapter_profile().is_none());
    }

    #[test]
    fn responses_provider_settings_round_trip_state_and_cache_options() {
        let mut provider = ProviderSettings::default();
        provider.name = "Primary Responses".to_string();
        provider.kind = ProviderKind::OpenAiResponses;
        provider.store_responses = true;
        provider.parallel_tool_calls = true;
        provider.prompt_cache_key = Some("workspace-cache".to_string());
        provider.prompt_cache_policy = Some(PromptCachePolicy::Explicit30m);
        provider.responses_compaction_threshold_tokens = Some(96_000);
        provider.rollout_budget = Some(RolloutBudgetSettings {
            limit_tokens: 120_000,
            sampling_token_weight: 1.0,
            prefill_token_weight: 0.25,
        });

        let json = serde_json::to_value(&provider).unwrap();
        assert_eq!(json["kind"], "openai_responses");
        let restored: ProviderSettings = serde_json::from_value(json).unwrap();

        assert_eq!(restored.kind, ProviderKind::OpenAiResponses);
        assert_eq!(restored.name, "Primary Responses");
        assert!(restored.store_responses);
        assert!(restored.parallel_tool_calls);
        assert_eq!(
            restored.prompt_cache_key.as_deref(),
            Some("workspace-cache")
        );
        assert_eq!(
            restored.prompt_cache_policy,
            Some(PromptCachePolicy::Explicit30m)
        );
        assert_eq!(restored.responses_compaction_threshold_tokens, Some(96_000));
        assert_eq!(restored.rollout_budget, provider.rollout_budget);
        assert!(restored
            .rollout_budget
            .as_ref()
            .expect("rollout budget")
            .validate()
            .is_ok());
    }

    #[test]
    fn provider_context_limit_uses_override_known_model_then_fallback() {
        let mut provider = ProviderSettings::default();
        assert_eq!(provider.resolved_context_window_tokens(), 1_047_576);

        provider.context_window_tokens = Some(64_000);
        assert_eq!(provider.resolved_context_window_tokens(), 64_000);

        provider.context_window_tokens = None;
        provider.model = "private-model".to_string();
        assert_eq!(
            provider.resolved_context_window_tokens(),
            DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS
        );
    }

    #[test]
    fn rollout_budget_rejects_unbounded_or_invalid_accounting() {
        let mut budget = RolloutBudgetSettings {
            limit_tokens: 0,
            sampling_token_weight: 1.0,
            prefill_token_weight: 1.0,
        };
        assert!(budget.validate().is_err());

        budget.limit_tokens = 100;
        budget.sampling_token_weight = 0.0;
        budget.prefill_token_weight = 0.0;
        assert!(budget.validate().is_err());

        budget.prefill_token_weight = f64::NAN;
        assert!(budget.validate().is_err());
    }

    #[test]
    fn responses_provider_accepts_legacy_kind_spelling() {
        let kind: ProviderKind = serde_json::from_str("\"open_ai_responses\"").unwrap();
        assert_eq!(kind, ProviderKind::OpenAiResponses);
    }

    #[test]
    fn app_settings_ignore_legacy_web_search_configuration() {
        let provider = ProviderSettings::default();
        let settings = AppSettings {
            providers: vec![provider.clone()],
            active_provider_id: provider.id,
            permission_mode: PermissionMode::Auto,
            agent_runtime: AgentRuntimeSettings::default(),
            default_workspace_root: None,
            sandbox: SandboxSettings::default(),
            enterprise: EnterpriseSettings::default(),
            parallel_tool_calls_migrated: true,
            updated_at: Utc::now(),
        };
        let mut value = serde_json::to_value(settings).unwrap();
        value["webSearch"] = serde_json::json!({
            "mode": "custom_api",
            "endpoint": "https://search.example.test",
            "apiKeySource": "legacy",
            "apiKeyConfigured": true,
            "maxResults": 5
        });

        serde_json::from_value::<AppSettings>(value)
            .expect("legacy web search settings should be ignored");
    }

    #[test]
    fn codex_app_server_provider_needs_no_api_key_or_base_url() {
        let mut provider = ProviderSettings::default();
        provider.kind = ProviderKind::CodexAppServer;
        provider.base_url.clear();
        provider.model.clear();
        provider.api_key_configured = false;

        let health = ProviderHealth::from_settings(&provider);

        assert!(health.api_key_configured);
        assert!(!health.using_mock);
        assert_eq!(health.status, "local_codex");
        assert!(provider.supports_vision);
    }

    #[test]
    fn responses_capabilities_distinguish_stored_and_opaque_state() {
        let mut provider = ProviderSettings::default();
        provider.kind = ProviderKind::OpenAiResponses;
        provider.store_responses = false;
        provider.responses_compaction_threshold_tokens = Some(96_000);

        let stateless = provider.capabilities();
        assert!(stateless.supports_native_compaction);
        assert_eq!(
            stateless.native_compaction_protocol,
            Some(NativeCompactionProtocol::OpenAiResponsesCompact)
        );
        assert!(stateless.supports_provider_items);
        assert!(!stateless.supports_response_state);
        assert!(!stateless.supports_previous_response_id);
        assert_eq!(
            stateless.tool_protocol.function_tools,
            ProviderFeatureSupport::Supported
        );
        assert_eq!(
            stateless.tool_protocol.hosted_apply_patch,
            ProviderFeatureSupport::Unknown
        );

        provider.store_responses = true;
        let stateful = provider.capabilities();
        assert!(stateful.supports_response_state);
        assert!(stateful.supports_previous_response_id);

        provider.kind = ProviderKind::OpenAiCompatible;
        let chat = provider.capabilities();
        assert!(!chat.supports_native_compaction);
        assert_eq!(chat.native_compaction_protocol, None);
        assert!(!chat.supports_provider_items);
    }

    #[test]
    fn legacy_report_is_migrated_before_runtime_capabilities_are_resolved() {
        let mut provider = ProviderSettings::default();
        provider.kind = ProviderKind::OpenAiResponses;
        provider.base_url = "https://relay.example/v1".to_string();
        provider.model = "opaque-model-id".to_string();
        provider.openai_compatibility = Some(OpenAiCompatibilityReport {
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            selected_protocol: OpenAiProtocol::Responses,
            chat_completions: ProviderFeatureSupport::Unknown,
            chat_function_tools: ProviderFeatureSupport::Unknown,
            chat_strict_function_tools: ProviderFeatureSupport::Unknown,
            chat_streaming_tools: ProviderFeatureSupport::Unknown,
            chat_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            chat_json_schema_output: ProviderFeatureSupport::Unknown,
            chat_message_protocol: ProviderMessageProtocolCapabilities::default(),
            responses: ProviderFeatureSupport::Supported,
            responses_native_tools: ProviderFeatureSupport::Supported,
            responses_function_tools: ProviderFeatureSupport::Supported,
            responses_strict_function_tools: ProviderFeatureSupport::Supported,
            responses_streaming_tools: ProviderFeatureSupport::Supported,
            responses_parallel_tool_calls: ProviderFeatureSupport::Supported,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Supported,
            responses_apply_patch: ProviderFeatureSupport::Unsupported,
            developer_messages: ProviderFeatureSupport::Unknown,
            message_compatibility: false,
            checked_at: Utc::now(),
            notes: Vec::new(),
        });

        assert_eq!(
            provider.capabilities().tool_protocol.freeform_tools,
            ProviderFeatureSupport::Unknown,
            "diagnostic reports are not a runtime capability source"
        );
        provider.migrate_legacy_openai_compatibility_report();
        let capabilities = provider.capabilities().tool_protocol;
        assert_eq!(
            capabilities.freeform_tools,
            ProviderFeatureSupport::Supported
        );
        assert_eq!(
            capabilities.hosted_apply_patch,
            ProviderFeatureSupport::Unsupported
        );
        assert_eq!(
            capabilities.strict_function_tools,
            ProviderFeatureSupport::Supported
        );

        provider.model = "different-model".to_string();
        let stale = provider.capabilities().tool_protocol;
        assert_eq!(stale.freeform_tools, ProviderFeatureSupport::Unknown);
        assert_eq!(stale.hosted_apply_patch, ProviderFeatureSupport::Unknown);
        assert_eq!(stale.strict_function_tools, ProviderFeatureSupport::Unknown);
    }

    #[test]
    fn sandbox_settings_from_env_uses_defaults_and_legacy_mode() {
        let env = EnvGuard::cleared(&SANDBOX_ENV_KEYS);
        let settings = AppSettings::from_env(PermissionMode::Auto);
        assert_eq!(settings.sandbox, SandboxSettings::default());
        assert_eq!(settings.sandbox.network, NetworkPolicy::Deny);

        env.set("OPENTOPIA_SANDBOX_MODE", "best_effort");
        env.set("OPENTOPIA_SANDBOX_NETWORK", "inherit");
        let writable_roots = [PathBuf::from("C:/workspace"), PathBuf::from("D:/scratch")];
        let read_paths = [PathBuf::from("C:/reference")];
        env.set(
            "OPENTOPIA_SANDBOX_WRITABLE_ROOTS",
            std::env::join_paths(&writable_roots).expect("join writable roots"),
        );
        env.set(
            "OPENTOPIA_SANDBOX_READ_PATHS",
            std::env::join_paths(&read_paths).expect("join read paths"),
        );

        let settings = SandboxSettings::from_env();
        assert_eq!(settings.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert_eq!(settings.enforcement, SandboxEnforcement::BestEffort);
        assert_eq!(settings.network, NetworkPolicy::Inherit);
        assert_eq!(settings.writable_roots, writable_roots);
        assert_eq!(settings.read_paths, read_paths);

        env.set("OPENTOPIA_SANDBOX_NETWORK", "deny");
        assert_eq!(SandboxSettings::from_env().network, NetworkPolicy::Deny);
    }

    #[test]
    fn sandbox_settings_convert_to_local_config() {
        let settings = SandboxSettings {
            sandbox_mode: SandboxMode::WorkspaceWrite,
            enforcement: SandboxEnforcement::BestEffort,
            network: NetworkPolicy::Inherit,
            writable_roots: vec![PathBuf::from("C:/workspace")],
            read_paths: vec![PathBuf::from("C:/reference")],
            windows_backend: WindowsSandboxBackend::DedicatedUser,
        };

        let config = settings.to_local_sandbox_config();
        assert!(config.enabled);
        assert_eq!(config.mode, OsSandboxMode::BestEffort);
        assert_eq!(config.network, NetworkPolicy::Inherit);
        assert_eq!(config.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert_eq!(config.writable_roots, settings.writable_roots);
        assert_eq!(config.read_paths, settings.read_paths);
        assert!(config.write_paths.is_empty());
        assert_eq!(config.sandbox_home, None);
        assert_eq!(config.windows_backend, WindowsSandboxBackend::DedicatedUser);
    }

    #[test]
    fn danger_full_access_forces_disabled_enforcement_and_network_allow() {
        let settings = SandboxSettings {
            sandbox_mode: SandboxMode::DangerFullAccess,
            enforcement: SandboxEnforcement::Enforce,
            network: NetworkPolicy::Deny,
            ..SandboxSettings::default()
        };

        let config = settings.to_local_sandbox_config();
        assert!(!config.enabled);
        assert_eq!(config.mode, OsSandboxMode::Disabled);
        assert_eq!(config.network, NetworkPolicy::Allow);
        assert_eq!(config.sandbox_mode, SandboxMode::DangerFullAccess);
    }

    #[test]
    fn invalid_sandbox_settings_fail_safe() {
        let settings: SandboxSettings = serde_json::from_str(
            r#"{
                "sandboxMode": "workspace-write",
                "enforcement": "unexpected",
                "network": "allow"
            }"#,
        )
        .expect("invalid settings should deserialize to a safe configuration");

        assert_eq!(settings.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(settings.enforcement, SandboxEnforcement::Enforce);
        assert_eq!(settings.network, NetworkPolicy::Deny);
    }

    #[test]
    fn release_gate_provider_settings_enable_native_tool_search_only_when_known() {
        let mut official = ProviderSettings::default();
        official.kind = ProviderKind::OpenAiResponses;
        official.base_url = "https://api.openai.com/v1".to_string();
        official.model = "gpt-5.4".to_string();
        let native = official.capabilities().tool_protocol;
        assert_eq!(
            native.deferred_tool_loading,
            ProviderFeatureSupport::Supported
        );
        assert_eq!(native.namespace_tools, ProviderFeatureSupport::Supported);
        assert_eq!(native.hosted_tool_search, ProviderFeatureSupport::Supported);

        official.model = "gpt-5.3".to_string();
        assert_eq!(
            official.capabilities().tool_protocol.hosted_tool_search,
            ProviderFeatureSupport::Unknown
        );

        official.model = "gpt-6".to_string();
        assert_eq!(
            official.capabilities().tool_protocol.hosted_tool_search,
            ProviderFeatureSupport::Supported
        );

        official.model = "gpt-5.4".to_string();
        official.base_url = "https://responses-relay.example/v1".to_string();
        assert_eq!(
            official.capabilities().tool_protocol.namespace_tools,
            ProviderFeatureSupport::Unknown
        );

        official.kind = ProviderKind::Anthropic;
        assert_eq!(
            official.capabilities().tool_protocol.hosted_tool_search,
            ProviderFeatureSupport::Unknown
        );
    }
}
