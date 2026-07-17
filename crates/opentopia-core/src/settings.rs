use crate::policy::PermissionMode;
use crate::sandbox::{LocalSandboxConfig, NetworkPolicy, OsSandboxMode, SandboxMode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Mock,
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSettings {
    pub id: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_provider_temperature")]
    pub temperature: f64,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default = "default_provider_context_window_tokens")]
    pub context_window_tokens: usize,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    pub api_key_source: String,
    pub api_key_configured: bool,
    pub health_status: Option<String>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4.1-mini".to_string(),
            temperature: default_provider_temperature(),
            max_output_tokens: None,
            context_window_tokens: default_provider_context_window_tokens(),
            reasoning_effort: None,
            api_key_source: "OPENTOPIA_API_KEY".to_string(),
            api_key_configured: false,
            health_status: None,
        }
    }
}

impl ProviderSettings {
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
        settings
    }
}

fn default_provider_temperature() -> f64 {
    0.2
}

fn default_provider_context_window_tokens() -> usize {
    128_000
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
            network: NetworkPolicy::Deny,
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
            Err(_) if sandbox_mode == SandboxMode::DangerFullAccess => NetworkPolicy::Allow,
            Err(_) => NetworkPolicy::Deny,
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
            provider.api_key_configured =
                std::env::var(&provider.api_key_source).is_ok_and(|value| !value.is_empty());
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
        let api_key_configured = std::env::var(&settings.api_key_source)
            .is_ok_and(|value| !value.is_empty())
            || settings.api_key_configured;
        let using_mock = settings.kind == ProviderKind::Mock || !api_key_configured;
        Self {
            id: settings.id.clone(),
            kind: settings.kind.clone(),
            base_url: settings.base_url.clone(),
            model: settings.model.clone(),
            api_key_source: settings.api_key_source.clone(),
            api_key_configured,
            using_mock,
            status: if using_mock {
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

        assert_eq!(provider.temperature, 0.2);
        assert_eq!(provider.max_output_tokens, None);
        assert_eq!(provider.context_window_tokens, 128_000);
        assert_eq!(provider.reasoning_effort, None);
    }

    #[test]
    fn sandbox_settings_from_env_uses_defaults_and_legacy_mode() {
        let env = EnvGuard::cleared(&SANDBOX_ENV_KEYS);
        let settings = AppSettings::from_env(PermissionMode::Auto);
        assert_eq!(settings.sandbox, SandboxSettings::default());

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
