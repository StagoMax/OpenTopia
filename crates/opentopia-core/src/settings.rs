use crate::policy::PermissionMode;
use crate::prompt_runtime::AgentRuntimeSettings;
use crate::sandbox::{LocalSandboxConfig, NetworkPolicy, OsSandboxMode, SandboxMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiCompatibilityReport {
    pub base_url: String,
    pub model: String,
    pub selected_protocol: OpenAiProtocol,
    pub chat_completions: ProviderFeatureSupport,
    #[serde(default)]
    pub chat_function_tools: ProviderFeatureSupport,
    pub responses: ProviderFeatureSupport,
    #[serde(default)]
    pub responses_native_tools: ProviderFeatureSupport,
    /// Function tools using the Responses wire shape. Kept separate from
    /// `responses_native_tools`: a relay may accept hosted web search while
    /// rejecting application-defined functions, or vice versa.
    #[serde(default)]
    pub responses_function_tools: ProviderFeatureSupport,
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
#[serde(rename_all = "camelCase")]
pub struct ProviderToolProtocolCapabilities {
    pub function_tools: ProviderFeatureSupport,
    pub freeform_tools: ProviderFeatureSupport,
    pub hosted_apply_patch: ProviderFeatureSupport,
    pub assistant_phase: ProviderFeatureSupport,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub id: String,
    /// User-facing label. Empty values from legacy settings fall back to `id`.
    #[serde(default)]
    pub name: String,
    pub kind: ProviderKind,
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
    #[serde(default)]
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
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4.1-mini".to_string(),
            enabled_families: Vec::new(),
            synced_models: Vec::new(),
            model_context_windows: BTreeMap::new(),
            models_synced_at: None,
            temperature: None,
            max_output_tokens: None,
            context_window_tokens: None,
            reasoning_effort: None,
            store_responses: false,
            parallel_tool_calls: false,
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
    pub fn capabilities(&self) -> ProviderCapabilities {
        match self.kind {
            ProviderKind::OpenAiResponses => {
                let negotiated = self
                    .openai_compatibility
                    .as_ref()
                    .filter(|report| report.applies_to(&self.base_url, &self.model));
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
                            .map(|report| report.responses_function_tools)
                            .unwrap_or(ProviderFeatureSupport::Supported),
                        freeform_tools: negotiated
                            .map(|report| report.responses_custom_tools)
                            .unwrap_or_default(),
                        hosted_apply_patch: negotiated
                            .map(|report| report.responses_apply_patch)
                            .unwrap_or_default(),
                        // Phase is optional on the wire and cannot be proven
                        // without observing an assistant message. Parsing is
                        // always tolerant; replayed items are authoritative.
                        assistant_phase: ProviderFeatureSupport::Unknown,
                    },
                }
            }
            ProviderKind::OpenAiCompatible => ProviderCapabilities {
                supports_prompt_cache: true,
                tool_protocol: ProviderToolProtocolCapabilities {
                    function_tools: self
                        .openai_compatibility
                        .as_ref()
                        .filter(|report| report.applies_to(&self.base_url, &self.model))
                        .map(|report| report.chat_function_tools)
                        .unwrap_or(ProviderFeatureSupport::Unknown),
                    ..ProviderToolProtocolCapabilities::default()
                },
                ..ProviderCapabilities::default()
            },
            ProviderKind::Mock | ProviderKind::CodexAppServer | ProviderKind::Anthropic => {
                ProviderCapabilities::default()
            }
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
        self.context_window_tokens
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
            if model != resolved.model {
                // A different model invalidates the connection's context-window
                // override, which was declared for the previous model.
                resolved.context_window_tokens = None;
            }
            resolved.model = model.to_string();
        }
        if let Some(reasoning_effort) = reasoning_effort {
            resolved.reasoning_effort = reasoning_effort
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
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
    // ── OpenAI ──────────────────────────────────────────────────────────
    if model.starts_with("gpt-5.6") {
        return Some(1_047_576);
    }
    if model.starts_with("gpt-5") {
        return Some(272_000);
    }
    if model.starts_with("gpt-4.1") {
        return Some(1_047_576);
    }
    if model.starts_with("gpt-4o") || model.starts_with("gpt-4-turbo") {
        return Some(128_000);
    }
    if is_openai_reasoning_model(model) {
        return Some(200_000);
    }

    // ── Anthropic ───────────────────────────────────────────────────────
    if claude_model_has_one_million_context(model) {
        return Some(1_000_000);
    }
    if model.starts_with("claude-") || model.contains("claude") {
        return Some(200_000);
    }

    // ── Google ──────────────────────────────────────────────────────────
    if model.starts_with("gemini-1.5") || model.starts_with("gemini-2") {
        return Some(1_000_000);
    }
    if model.starts_with("gemini") {
        return Some(128_000);
    }

    // ── Moonshot / Kimi ─────────────────────────────────────────────────
    // moonshot-v1-* has explicit context-tier suffixes.
    if model.starts_with("moonshot-v1-8k") {
        return Some(8_000);
    }
    if model.starts_with("moonshot-v1-32k") {
        return Some(32_000);
    }
    if model.starts_with("moonshot-v1-128k") {
        return Some(128_000);
    }
    if model.starts_with("moonshot-v1") {
        return Some(8_000); // base tier
    }
    // K3 has a tier-dependent 1M variant and an explicit 256K variant. A
    // reported connection limit or manual override takes precedence here.
    if model.starts_with("k3-256k") || model.starts_with("kimi-k3-256k") {
        return Some(256_000);
    }
    if model == "k3" || model.starts_with("k3-") || model.starts_with("kimi-k3") {
        return Some(1_000_000);
    }
    // kimi-k2.5 is a 256K variant; other legacy Kimi IDs are 128K.
    if model.starts_with("kimi-k2.5") {
        return Some(256_000);
    }
    if model.starts_with("kimi") {
        return Some(128_000);
    }

    // ── DeepSeek ────────────────────────────────────────────────────────
    // V4 introduced 1M context; earlier models are 128K.
    if model.starts_with("deepseek-v4") {
        return Some(1_000_000);
    }
    if model.starts_with("deepseek") {
        return Some(128_000);
    }

    // ── Qwen / Alibaba ──────────────────────────────────────────────────
    if model.starts_with("qwen") || model.starts_with("qwq") || model.starts_with("qvq") {
        return Some(128_000);
    }

    // ── Zhipu GLM ───────────────────────────────────────────────────────
    if model.starts_with("glm-5.2") {
        return Some(1_000_000);
    }
    if model.starts_with("glm") || model.starts_with("chatglm") {
        return Some(128_000);
    }

    // ── xAI Grok ────────────────────────────────────────────────────────
    if model.starts_with("grok") {
        return Some(128_000);
    }

    // ── Mistral ─────────────────────────────────────────────────────────
    if model.starts_with("mistral")
        || model.starts_with("mixtral")
        || model.starts_with("codestral")
    {
        return Some(128_000);
    }

    // ── Meta Llama ──────────────────────────────────────────────────────
    if model.starts_with("llama") || model.starts_with("codellama") {
        return Some(128_000);
    }

    // ── MiniMax ─────────────────────────────────────────────────────────
    if model.starts_with("minimax") || model.starts_with("abab") {
        return Some(128_000);
    }

    None
}

fn claude_model_has_one_million_context(model: &str) -> bool {
    [
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-opus-4-8",
        "claude-opus-5",
        "claude-sonnet-4-6",
        "claude-sonnet-5",
        "claude-fable-5",
        "claude-mythos-5",
        "claude-mythos-preview",
    ]
    .iter()
    .any(|prefix| model.starts_with(prefix))
}

/// Whether a model accepts a caller-supplied `temperature`.
///
/// OpenAI's reasoning families (o-series, GPT-5.x) reject any value other than
/// the default and answer with HTTP 400, so the parameter must be omitted
/// rather than clamped. Unknown models are assumed to accept it: relay
/// endpoints serve arbitrary ids, and dropping the parameter for everything
/// would silently change behaviour for models that do honour it.
pub fn model_accepts_temperature(model: &str) -> bool {
    !model_bases(model)
        .iter()
        .any(|base| is_openai_reasoning_model(base))
}

/// Candidate model names after removing the vendor prefixes relays prepend.
///
/// Only a leading `vendor.` is stripped, never an inner dot, because dots also
/// carry version numbers (`gpt-5.6`).
fn model_bases(model: &str) -> Vec<String> {
    let normalized = model.trim().to_ascii_lowercase();
    let after_slash = normalized
        .rsplit('/')
        .next()
        .unwrap_or(&normalized)
        .to_string();

    let mut bases = vec![after_slash.clone()];
    if let Some((prefix, rest)) = after_slash.split_once('.') {
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

fn is_openai_reasoning_model(model: &str) -> bool {
    if model.starts_with("gpt-5") {
        return true;
    }
    // o1 / o3 / o4-mini and friends, but not `olmo` or `openai`.
    let mut chars = model.chars();
    chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit())
}

pub const MIN_PROVIDER_CONTEXT_WINDOW_TOKENS: usize = 4_096;
pub const DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS: usize = 128_000;

fn default_provider_supports_vision() -> bool {
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
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            sandbox_mode: SandboxMode::WorkspaceWrite,
            enforcement: SandboxEnforcement::Enforce,
            network: NetworkPolicy::Allow,
            writable_roots: Vec::new(),
            read_paths: Vec::new(),
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

        match (sandbox_mode, enforcement, network) {
            (Some(sandbox_mode), Some(enforcement), Some(network)) => Ok(Self {
                sandbox_mode,
                enforcement,
                network,
                writable_roots: wire.writable_roots,
                read_paths: wire.read_paths,
            }),
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
            Err(_) => NetworkPolicy::Allow,
        };

        Self {
            sandbox_mode,
            enforcement,
            network,
            writable_roots,
            read_paths,
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
        }
    }
}

impl From<&SandboxSettings> for LocalSandboxConfig {
    fn from(settings: &SandboxSettings) -> Self {
        settings.to_local_sandbox_config()
    }
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
            provider.api_key_configured = if provider.kind == ProviderKind::CodexAppServer {
                true
            } else {
                std::env::var(&provider.api_key_source).is_ok_and(|value| !value.is_empty())
            };
        }
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealth {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    pub api_key_source: String,
    pub api_key_configured: bool,
    pub using_mock: bool,
    pub status: String,
}

impl ProviderHealth {
    pub fn from_settings(settings: &ProviderSettings) -> Self {
        let codex_app_server = settings.kind == ProviderKind::CodexAppServer;
        let api_key_configured = codex_app_server
            || std::env::var(&settings.api_key_source).is_ok_and(|value| !value.is_empty())
            || settings.api_key_configured;
        let using_mock =
            settings.kind == ProviderKind::Mock || (!codex_app_server && !api_key_configured);
        Self {
            id: settings.id.clone(),
            kind: settings.kind.clone(),
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key_source: settings.api_key_source.clone(),
            api_key_configured,
            using_mock,
            status: if codex_app_server {
                "local_codex".to_string()
            } else if using_mock {
                "mock_or_unconfigured".to_string()
            } else {
                "configured".to_string()
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
    fn reasoning_models_do_not_accept_a_temperature() {
        for model in [
            "o1",
            "o3-mini",
            "o4-mini",
            "gpt-5",
            "gpt-5.6-sol",
            "openai/gpt-5.6",
            "azure.o3-mini",
        ] {
            assert!(
                !model_accepts_temperature(model),
                "{model} should reject temperature"
            );
        }
    }

    #[test]
    fn ordinary_models_still_accept_a_temperature() {
        // `olmo` and `openai` must not be mistaken for the o-series, and a
        // version dot must not be mistaken for a vendor prefix.
        for model in [
            "gpt-4.1-mini",
            "gpt-4o",
            "claude-sonnet-4-5",
            "kimi-k2.5-turbo",
            "deepseek-chat",
            "olmo-2-13b",
            "openai-mirror",
        ] {
            assert!(
                model_accepts_temperature(model),
                "{model} should accept temperature"
            );
        }
    }

    #[test]
    fn context_window_table_covers_the_families_the_picker_offers() {
        assert_eq!(
            known_model_context_window_tokens("gpt-5.6-sol"),
            Some(1_047_576)
        );
        assert_eq!(
            known_model_context_window_tokens("gpt-4.1-mini"),
            Some(1_047_576)
        );
        assert_eq!(known_model_context_window_tokens("o3-mini"), Some(200_000));
        assert_eq!(
            known_model_context_window_tokens("anthropic/claude-sonnet-4-5"),
            Some(200_000)
        );
        // K3 has a 1M tier and an explicit 256K model ID; per-connection
        // entitlement still wins through detected or manually set values.
        assert_eq!(
            known_model_context_window_tokens("kimi-k2.5"),
            Some(256_000)
        );
        assert_eq!(known_model_context_window_tokens("kimi-k2"), Some(128_000));
        assert_eq!(known_model_context_window_tokens("k3-256k"), Some(256_000));
        assert_eq!(
            known_model_context_window_tokens("glm-5.2"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("claude-sonnet-4-6"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("kimi-k3"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("moonshot-v1-32k"),
            Some(32_000)
        );
        assert_eq!(
            known_model_context_window_tokens("moonshot-v1-8k"),
            Some(8_000)
        );
        // deepseek-v4 is 1M; earlier models are 128K
        assert_eq!(
            known_model_context_window_tokens("deepseek-v4"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window_tokens("deepseek-reasoner"),
            Some(128_000)
        );
        assert_eq!(
            known_model_context_window_tokens("gemini-2.5-pro"),
            Some(1_000_000)
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
        assert!(!provider.parallel_tool_calls);
        assert_eq!(provider.prompt_cache_key, None);
        assert_eq!(provider.prompt_cache_policy, None);
        assert_eq!(provider.responses_compaction_threshold_tokens, None);
        assert_eq!(provider.rollout_budget, None);
        assert_eq!(provider.openai_compatibility, None);
    }

    #[test]
    fn openai_compatibility_report_round_trips_and_rejects_stale_settings() {
        let mut provider = ProviderSettings::default();
        provider.base_url = "https://relay.example/v1".to_string();
        provider.model = "relay-model".to_string();
        provider.openai_compatibility = Some(OpenAiCompatibilityReport {
            base_url: provider.base_url.clone(),
            model: provider.model.clone(),
            selected_protocol: OpenAiProtocol::ChatCompletions,
            chat_completions: ProviderFeatureSupport::Supported,
            chat_function_tools: ProviderFeatureSupport::Supported,
            responses: ProviderFeatureSupport::Unsupported,
            responses_native_tools: ProviderFeatureSupport::Unsupported,
            responses_function_tools: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Unknown,
            responses_apply_patch: ProviderFeatureSupport::Unknown,
            developer_messages: ProviderFeatureSupport::Unsupported,
            message_compatibility: true,
            checked_at: Utc::now(),
            notes: vec!["developer messages: HTTP 400".to_string()],
        });

        let encoded = serde_json::to_string(&provider).unwrap();
        let restored: ProviderSettings = serde_json::from_str(&encoded).unwrap();
        let report = restored.openai_compatibility.unwrap();

        assert!(report.applies_to("https://relay.example/v1/", "relay-model"));
        assert!(!report.applies_to("https://other.example/v1", "relay-model"));
        assert!(!report.applies_to("https://relay.example/v1", "other-model"));
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
    fn responses_tool_capabilities_use_matching_probe_without_model_name_mapping() {
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
            responses: ProviderFeatureSupport::Supported,
            responses_native_tools: ProviderFeatureSupport::Supported,
            responses_function_tools: ProviderFeatureSupport::Supported,
            responses_custom_tools: ProviderFeatureSupport::Supported,
            responses_apply_patch: ProviderFeatureSupport::Unsupported,
            developer_messages: ProviderFeatureSupport::Unknown,
            message_compatibility: false,
            checked_at: Utc::now(),
            notes: Vec::new(),
        });

        let capabilities = provider.capabilities().tool_protocol;
        assert_eq!(
            capabilities.freeform_tools,
            ProviderFeatureSupport::Supported
        );
        assert_eq!(
            capabilities.hosted_apply_patch,
            ProviderFeatureSupport::Unsupported
        );

        provider.model = "different-model".to_string();
        let stale = provider.capabilities().tool_protocol;
        assert_eq!(stale.freeform_tools, ProviderFeatureSupport::Unknown);
        assert_eq!(stale.hosted_apply_patch, ProviderFeatureSupport::Unknown);
    }

    #[test]
    fn sandbox_settings_from_env_uses_defaults_and_legacy_mode() {
        let env = EnvGuard::cleared(&SANDBOX_ENV_KEYS);
        let settings = AppSettings::from_env(PermissionMode::Auto);
        assert_eq!(settings.sandbox, SandboxSettings::default());
        assert_eq!(settings.sandbox.network, NetworkPolicy::Allow);

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
}
