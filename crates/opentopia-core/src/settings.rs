use crate::policy::PermissionMode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Mock,
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
            "CREDIT_REVIEW_LLM_BASE_URL",
            "AUDIT_COPILOT_LLM_BASE_URL",
            "OPENAI_BASE_URL",
        ]) {
            settings.base_url = base_url;
        }
        if let Some(model) = first_env([
            "OPENTOPIA_MODEL",
            "CREDIT_REVIEW_LLM_MODEL",
            "AUDIT_COPILOT_LLM_MODEL",
            "CREDIT_REVIEW_LLM_CHEAP_MODEL",
            "CREDIT_REVIEW_LLM_STRONG_MODEL",
        ]) {
            settings.model = model;
        }
        if let Some((source, _value)) = first_env_with_key([
            "OPENTOPIA_API_KEY",
            "CREDIT_REVIEW_LLM_API_KEY",
            "AUDIT_COPILOT_LLM_API_KEY",
            "OPENAI_API_KEY",
        ]) {
            settings.api_key_source = source;
            settings.api_key_configured = true;
        }
        settings
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
