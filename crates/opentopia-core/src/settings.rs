use crate::policy::PermissionMode;
use crate::prompt_runtime::AgentRuntimeSettings;
use crate::sandbox::{
    LocalSandboxConfig, NetworkPolicy, OsSandboxMode, SandboxMode, WindowsSandboxBackend,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;

mod provider;
#[cfg(test)]
use provider::PREVIOUS_PROVIDER_ADAPTER_PROFILE_VERSION;
pub use provider::*;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthCheck {
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub model_available: bool,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_compatibility: Option<OpenAiCompatibilityReport>,
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
    fn context_window_table_uses_verified_models_and_previous_generations() {
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
        assert_eq!(
            known_model_context_window_tokens("glm-5.3"),
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
            known_model_context_window_tokens("deepseek-v5-flash"),
            Some(1_048_576)
        );
        assert_eq!(
            known_model_context_window_tokens("gemini-3.8-flash"),
            Some(1_048_576)
        );
        assert_eq!(known_model_context_window_tokens("deepseek-v5-image"), None);
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
    fn model_vision_settings_override_catalog_detection_and_unknown_fails_closed() {
        let mut provider = ProviderSettings {
            model: "text-only".to_string(),
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
        assert!(!provider.supports_vision_for_model());
    }

    #[test]
    fn vision_registry_covers_major_vendors_and_normalizes_relay_ids() {
        for model in [
            "openai/gpt-5.6-sol:batch",
            "anthropic/claude-sonnet-4.6",
            "google/gemini-3.5-flash",
            "moonshotai/kimi-k2.5",
            "kimi-for-coding",
            "qwen/qwen3.7-plus",
            "z-ai/glm-5v-turbo",
            "x-ai/grok-4.5",
            "mistralai/mistral-small-3.2-24b-instruct",
            "meta-llama/llama-4-scout",
            "minimax/minimax-m3",
            "deepseek-v4-flash-vision-exp",
        ] {
            assert_eq!(known_model_supports_vision(model), Some(true), "{model}");
        }

        for model in [
            "moonshot-v1-128k",
            "kimi-k2",
            "deepseek-v4-flash",
            "qwen3-coder-plus",
            "glm-5",
            "glm-5.3",
            "minimax-m2.5",
        ] {
            assert_eq!(known_model_supports_vision(model), Some(false), "{model}");
        }
        assert_eq!(known_model_supports_vision("custom/unknown-vlm"), None);
    }

    #[test]
    fn catalog_metadata_outranks_the_vision_registry_and_manual_settings_outrank_both() {
        let mut provider = ProviderSettings {
            model: "kimi-k2.5".to_string(),
            ..ProviderSettings::default()
        };
        assert!(provider.supports_vision_for_model());

        provider.model_capabilities.insert(
            "kimi-k2.5".to_string(),
            ProviderModelCapabilities {
                supports_vision: Some(false),
            },
        );
        assert!(!provider.supports_vision_for_model());

        provider.model_settings.insert(
            "kimi-k2.5".to_string(),
            ProviderModelSettings {
                supports_vision: Some(true),
                ..ProviderModelSettings::default()
            },
        );
        assert!(provider.supports_vision_for_model());
    }

    #[test]
    fn legacy_connection_vision_default_is_ignored_and_not_reserialized() {
        let mut value = serde_json::to_value(ProviderSettings::default()).unwrap();
        value["model"] = serde_json::json!("unknown-model");
        value["supportsVision"] = serde_json::json!(true);

        let provider = serde_json::from_value::<ProviderSettings>(value)
            .expect("legacy supportsVision should be ignored");

        assert!(!provider.supports_vision_for_model());
        assert!(serde_json::to_value(provider)
            .unwrap()
            .get("supportsVision")
            .is_none());
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
            chat_reasoning_protocol: Some(ProviderReasoningProtocol::ChatReasoningEffort),
            responses: ProviderFeatureSupport::Unsupported,
            responses_native_tools: ProviderFeatureSupport::Unsupported,
            responses_function_tools: ProviderFeatureSupport::Unknown,
            responses_strict_function_tools: ProviderFeatureSupport::Unknown,
            responses_streaming_tools: ProviderFeatureSupport::Unknown,
            responses_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Unknown,
            responses_apply_patch: ProviderFeatureSupport::Unknown,
            responses_reasoning_protocol: Some(ProviderReasoningProtocol::ResponsesReasoning),
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
            chat_reasoning_protocol: Some(ProviderReasoningProtocol::ChatReasoningEffort),
            responses: ProviderFeatureSupport::Supported,
            responses_native_tools: ProviderFeatureSupport::Supported,
            responses_function_tools: ProviderFeatureSupport::Supported,
            responses_strict_function_tools: ProviderFeatureSupport::Supported,
            responses_streaming_tools: ProviderFeatureSupport::Supported,
            responses_parallel_tool_calls: ProviderFeatureSupport::Supported,
            responses_json_schema_output: ProviderFeatureSupport::Supported,
            responses_custom_tools: ProviderFeatureSupport::Supported,
            responses_apply_patch: ProviderFeatureSupport::Supported,
            responses_reasoning_protocol: Some(ProviderReasoningProtocol::ResponsesReasoning),
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
            reasoning_protocol: ProviderReasoningProtocol::ChatReasoningEffort,
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
            chat_reasoning_protocol: None,
            responses: ProviderFeatureSupport::Unknown,
            responses_native_tools: ProviderFeatureSupport::Unknown,
            responses_function_tools: ProviderFeatureSupport::Unknown,
            responses_strict_function_tools: ProviderFeatureSupport::Unknown,
            responses_streaming_tools: ProviderFeatureSupport::Unknown,
            responses_parallel_tool_calls: ProviderFeatureSupport::Unknown,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Unknown,
            responses_apply_patch: ProviderFeatureSupport::Unknown,
            responses_reasoning_protocol: None,
            developer_messages: ProviderFeatureSupport::Unknown,
            message_compatibility: true,
            checked_at: Utc::now(),
            notes: Vec::new(),
        });
        assert!(relay.active_adapter_profile().is_none());
    }

    #[test]
    fn legacy_vendor_reasoning_names_deserialize_to_structural_protocols() {
        assert_eq!(
            serde_json::from_str::<ProviderReasoningProtocol>(r#""glm_thinking""#).unwrap(),
            ProviderReasoningProtocol::ChatThinkingReasoningEffort
        );
        assert_eq!(
            serde_json::from_str::<ProviderReasoningProtocol>(r#""deep_seek_thinking""#).unwrap(),
            ProviderReasoningProtocol::ChatThinkingHighMaxNoToolChoice
        );
        assert_eq!(
            serde_json::from_str::<ProviderReasoningProtocol>(r#""reasoning_effort""#).unwrap(),
            ProviderReasoningProtocol::ChatReasoningEffort
        );
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
            chat_reasoning_protocol: None,
            responses: ProviderFeatureSupport::Supported,
            responses_native_tools: ProviderFeatureSupport::Supported,
            responses_function_tools: ProviderFeatureSupport::Supported,
            responses_strict_function_tools: ProviderFeatureSupport::Supported,
            responses_streaming_tools: ProviderFeatureSupport::Supported,
            responses_parallel_tool_calls: ProviderFeatureSupport::Supported,
            responses_json_schema_output: ProviderFeatureSupport::Unknown,
            responses_custom_tools: ProviderFeatureSupport::Supported,
            responses_apply_patch: ProviderFeatureSupport::Unsupported,
            responses_reasoning_protocol: Some(ProviderReasoningProtocol::ResponsesReasoning),
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
