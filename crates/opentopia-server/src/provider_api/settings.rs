use crate::{
    negotiate_provider_settings, remove_windows_sandbox, setup_windows_sandbox,
    windows_sandbox_setup_status, AgentRuntimeSettings, ApiError, AppSettings, AppState,
    CodexAccountStatus, CodexLoginStart, DeleteResponse, PermissionMode, ProviderAdapterKind,
    ProviderAuthKind, ProviderDriverDescriptor, ProviderDriverRegistry, ProviderHealth,
    ProviderHealthCheck, ProviderKind, ProviderSettings, ProviderTransportKind, SandboxSettings,
    WindowsSandboxSetupStatus,
};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/settings", get(get_settings).patch(update_settings))
        .route(
            "/api/sandbox/windows/setup",
            get(get_windows_sandbox_setup)
                .post(configure_windows_sandbox)
                .delete(remove_windows_sandbox_configuration),
        )
        .route("/api/provider/drivers", get(list_provider_drivers))
        .route("/api/provider/health", get(provider_health))
        .route("/api/provider/test", post(test_provider_connection))
        .route("/api/codex/account", get(get_codex_account))
        .route("/api/codex/account/login", post(start_codex_login))
        .route("/api/codex/account/login/cancel", post(cancel_codex_login))
        .route("/api/codex/account/logout", post(logout_codex_account))
}

async fn get_settings(State(state): State<AppState>) -> Json<AppSettings> {
    Json(current_settings(&state))
}

async fn get_windows_sandbox_setup() -> Result<Json<WindowsSandboxSetupStatus>, ApiError> {
    let status = tokio::task::spawn_blocking(windows_sandbox_setup_status)
        .await
        .map_err(|error| ApiError::internal(format!("sandbox status task failed: {error}")))?
        .map_err(|error| ApiError::internal(format!("sandbox status failed: {error:#}")))?;
    Ok(Json(status))
}

async fn configure_windows_sandbox() -> Result<Json<WindowsSandboxSetupStatus>, ApiError> {
    let status = tokio::task::spawn_blocking(setup_windows_sandbox)
        .await
        .map_err(|error| ApiError::internal(format!("sandbox setup task failed: {error}")))?
        .map_err(|error| ApiError::internal(format!("sandbox setup failed: {error:#}")))?;
    Ok(Json(status))
}

async fn remove_windows_sandbox_configuration() -> Result<Json<WindowsSandboxSetupStatus>, ApiError>
{
    let status = tokio::task::spawn_blocking(remove_windows_sandbox)
        .await
        .map_err(|error| ApiError::internal(format!("sandbox removal task failed: {error}")))?
        .map_err(|error| ApiError::internal(format!("sandbox removal failed: {error:#}")))?;
    Ok(Json(status))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(request): Json<SettingsPatchRequest>,
) -> Result<Json<AppSettings>, ApiError> {
    let mut settings = current_settings(&state);
    if let Some(providers) = request.providers {
        validate_provider_settings(&providers)?;
        settings.providers = providers;
    }
    if let Some(active_provider_id) = request.active_provider_id {
        settings.active_provider_id = active_provider_id;
    }
    if let Some(kind) = request.provider_kind {
        settings
            .active_provider_mut()
            .apply_legacy_kind_preset(kind);
    }
    if let Some(base_url) = request.base_url {
        let base_url = base_url.trim();
        if base_url.is_empty() {
            return Err(ApiError::bad_request("baseUrl cannot be empty"));
        }
        settings.active_provider_mut().base_url = base_url.to_string();
    }
    if let Some(model) = request.model {
        let model = model.trim();
        if model.is_empty() {
            return Err(ApiError::bad_request("model cannot be empty"));
        }
        settings.active_provider_mut().model = model.to_string();
    }
    if let Some(api_key_source) = request.api_key_source {
        let api_key_source = api_key_source.trim();
        if api_key_source.is_empty() {
            return Err(ApiError::bad_request("apiKeySource cannot be empty"));
        }
        settings.active_provider_mut().api_key_source = api_key_source.to_string();
    }
    if let Some(permission_mode) = request.permission_mode {
        settings.permission_mode = permission_mode;
    }
    if let Some(agent_runtime) = request.agent_runtime {
        settings.agent_runtime = agent_runtime;
    }
    if request.clear_default_workspace_root.unwrap_or(false) {
        settings.default_workspace_root = None;
    } else if let Some(default_workspace_root) = request.default_workspace_root {
        settings.default_workspace_root = Some(default_workspace_root);
    }
    if let Some(sandbox) = request.sandbox {
        settings.sandbox = sandbox;
    }
    validate_provider_settings(&settings.providers)?;
    if !settings
        .providers
        .iter()
        .any(|provider| provider.id == settings.active_provider_id)
    {
        return Err(ApiError::bad_request(
            "active provider must reference a configured provider",
        ));
    }
    let settings = save_settings_and_refresh_runtime(&state, settings)?;
    Ok(Json(settings))
}

async fn provider_health(State(state): State<AppState>) -> Json<Vec<ProviderHealth>> {
    let settings = current_settings(&state);
    Json(
        settings
            .providers
            .iter()
            .map(ProviderHealth::from_settings)
            .collect(),
    )
}

async fn list_provider_drivers() -> Json<Vec<ProviderDriverDescriptor>> {
    Json(ProviderDriverRegistry::built_in().descriptors())
}

async fn get_codex_account(
    State(state): State<AppState>,
) -> Result<Json<CodexAccountStatus>, ApiError> {
    Ok(Json(state.codex_account.status().await?))
}

async fn start_codex_login(
    State(state): State<AppState>,
    Json(request): Json<CodexLoginRequest>,
) -> Result<Json<CodexLoginStart>, ApiError> {
    Ok(Json(
        state
            .codex_account
            .start_chatgpt_login(request.device_code)
            .await?,
    ))
}

async fn cancel_codex_login(
    State(state): State<AppState>,
) -> Result<Json<DeleteResponse>, ApiError> {
    state.codex_account.cancel_login().await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn logout_codex_account(
    State(state): State<AppState>,
) -> Result<Json<DeleteResponse>, ApiError> {
    state.codex_account.logout().await?;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn test_provider_connection(
    State(state): State<AppState>,
    Json(request): Json<ProviderTestRequest>,
) -> Result<Json<ProviderHealthCheck>, ApiError> {
    let settings = current_settings(&state);
    let provider_settings = if let Some(provider_id) = &request.provider_id {
        settings
            .providers
            .iter()
            .find(|p| &p.id == provider_id)
            .ok_or_else(|| ApiError::not_found(format!("provider not found: {provider_id}")))?
    } else {
        settings.active_provider()
    }
    .clone();
    if provider_settings.effective_transport() == ProviderTransportKind::Mock {
        return Err(ApiError::bad_request(
            "mock provider has no remote connection",
        ));
    }
    let negotiation = negotiate_provider_settings(&provider_settings).await?;
    let result = negotiation.health;

    if result.reachable && result.model_available {
        let mut latest = current_settings(&state);
        if let Some(target) = latest
            .providers
            .iter_mut()
            .find(|provider| provider.id == provider_settings.id)
        {
            if let Some(report) = result
                .openai_compatibility
                .as_ref()
                .filter(|report| report.applies_to(&target.base_url, &target.model))
            {
                target.apply_openai_compatibility_report(report.clone());
            } else {
                let base_url = target.base_url.clone();
                let model = target.model.clone();
                for profile in negotiation
                    .adapter_profiles
                    .into_iter()
                    .filter(|profile| profile.applies_to(&base_url, &model))
                {
                    target.apply_adapter_profile(profile);
                }
            }
            save_settings_and_refresh_runtime(&state, latest)?;
        }
    }
    Ok(Json(result))
}

/// Fetches the model ids a connection actually serves and caches them on the
/// connection. Relay endpoints ("中转站") front many vendors behind one key, so
/// this is what turns a single credential into a browsable model list.

pub(crate) fn validate_provider_settings(providers: &[ProviderSettings]) -> Result<(), ApiError> {
    if providers.is_empty() {
        return Err(ApiError::bad_request("at least one provider is required"));
    }
    let mut ids = HashSet::new();
    for provider in providers {
        let id = provider.id.trim();
        if id.is_empty() || !ids.insert(id) {
            return Err(ApiError::bad_request(
                "provider IDs must be non-empty and unique",
            ));
        }
        if id.len() > 80
            || !id.chars().enumerate().all(|(index, ch)| {
                ch.is_ascii_alphanumeric() || (index > 0 && matches!(ch, '.' | '_' | '-'))
            })
        {
            return Err(ApiError::bad_request(
                "provider IDs may contain only letters, numbers, dots, underscores, and hyphens",
            ));
        }
        let name = provider.name.trim();
        if (!provider.name.is_empty() && name.is_empty())
            || name.chars().count() > 80
            || name.chars().any(char::is_control)
        {
            return Err(ApiError::bad_request(
                "provider names must contain 1 to 80 visible characters",
            ));
        }
        let transport = provider.effective_transport();
        let allowed_adapters = provider.effective_allowed_adapters();
        let adapter_matches_transport = |adapter: &ProviderAdapterKind| match transport {
            ProviderTransportKind::Http => matches!(
                adapter,
                ProviderAdapterKind::OpenAiChat
                    | ProviderAdapterKind::OpenAiResponses
                    | ProviderAdapterKind::AnthropicMessages
            ),
            ProviderTransportKind::CodexAppServer => {
                *adapter == ProviderAdapterKind::CodexAppServer
            }
            ProviderTransportKind::Mock => *adapter == ProviderAdapterKind::Mock,
        };
        if allowed_adapters.is_empty() || !allowed_adapters.iter().all(adapter_matches_transport) {
            return Err(ApiError::bad_request(
                "provider adapters are incompatible with the selected transport",
            ));
        }
        if provider
            .preferred_adapter
            .is_some_and(|adapter| !allowed_adapters.contains(&adapter))
        {
            return Err(ApiError::bad_request(
                "preferred provider adapter must be enabled on the connection",
            ));
        }
        let auth_matches_transport = match transport {
            ProviderTransportKind::Http => {
                !matches!(provider.effective_auth(), ProviderAuthKind::CodexSession)
            }
            ProviderTransportKind::CodexAppServer => {
                provider.effective_auth() == ProviderAuthKind::CodexSession
            }
            ProviderTransportKind::Mock => provider.effective_auth() == ProviderAuthKind::None,
        };
        if !auth_matches_transport {
            return Err(ApiError::bad_request(
                "provider authentication is incompatible with the selected transport",
            ));
        }
        if provider.effective_transport() == ProviderTransportKind::Http {
            let base_url = reqwest::Url::parse(provider.base_url.trim()).map_err(|_| {
                ApiError::bad_request(format!("invalid provider base URL: {}", provider.base_url))
            })?;
            if !matches!(base_url.scheme(), "http" | "https") {
                return Err(ApiError::bad_request(
                    "provider base URL must use HTTP or HTTPS",
                ));
            }
        }
        if provider.effective_transport() == ProviderTransportKind::Http
            && provider.model.trim().is_empty()
            && !provider.synced_models.is_empty()
        {
            return Err(ApiError::bad_request("provider model cannot be empty"));
        }
        if let Some(temperature) = provider.temperature {
            if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
                return Err(ApiError::bad_request(
                    "provider temperature must be between 0 and 2",
                ));
            }
        }
        if provider.max_output_tokens == Some(0) {
            return Err(ApiError::bad_request(
                "max output tokens must be greater than zero",
            ));
        }
        if provider
            .context_window_tokens
            .is_some_and(|tokens| tokens < 4_096)
        {
            return Err(ApiError::bad_request(
                "context window must be at least 4096 tokens",
            ));
        }
        if let Some(threshold) = provider.responses_compaction_threshold_tokens {
            if threshold < 4_096 || threshold as usize >= provider.resolved_context_window_tokens()
            {
                return Err(ApiError::bad_request(
                    "native compaction threshold must be at least 4096 tokens and below the context window",
                ));
            }
        }
        if let Some(rollout_budget) = &provider.rollout_budget {
            rollout_budget.validate().map_err(ApiError::bad_request)?;
        }
        if let Some(effort) = provider.reasoning_effort.as_deref() {
            if ![
                "", "none", "minimal", "low", "medium", "high", "xhigh", "max",
            ]
            .contains(&effort)
            {
                return Err(ApiError::bad_request("reasoning effort is not supported"));
            }
        }
        for (model_id, model_settings) in &provider.model_settings {
            if model_id.trim().is_empty() {
                return Err(ApiError::bad_request("model setting IDs must not be empty"));
            }
            if let Some(temperature) = model_settings.temperature.flatten() {
                if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
                    return Err(ApiError::bad_request(
                        "model temperature must be between 0 and 2",
                    ));
                }
            }
            if model_settings.max_output_tokens.flatten() == Some(0) {
                return Err(ApiError::bad_request(
                    "model max output tokens must be greater than zero",
                ));
            }
            if model_settings
                .context_window_tokens
                .flatten()
                .is_some_and(|tokens| tokens < 4_096)
            {
                return Err(ApiError::bad_request(
                    "model context window must be at least 4096 tokens",
                ));
            }
            if let Some(effort) = model_settings
                .reasoning_effort
                .as_ref()
                .and_then(|value| value.as_deref())
            {
                if ![
                    "", "none", "minimal", "low", "medium", "high", "xhigh", "max",
                ]
                .contains(&effort)
                {
                    return Err(ApiError::bad_request(
                        "model reasoning effort is not supported",
                    ));
                }
            }
        }
        if provider
            .prompt_cache_key
            .as_deref()
            .is_some_and(|value| value.len() > 256)
        {
            return Err(ApiError::bad_request(
                "prompt cache key must be at most 256 characters",
            ));
        }
    }
    Ok(())
}

pub(crate) fn current_settings(state: &AppState) -> AppSettings {
    state
        .settings
        .read()
        .expect("settings lock poisoned")
        .clone()
}

/// Commits one settings snapshot and rebuilds every runtime view from that
/// exact persisted value. Capability negotiation callers use this single path
/// so the adapter never observes a half-updated report/profile pair.
pub(crate) fn save_settings_and_refresh_runtime(
    state: &AppState,
    settings: AppSettings,
) -> Result<AppSettings, ApiError> {
    let settings = state.store.save_settings(settings)?;
    {
        let mut settings_guard = state.settings.write().expect("settings lock poisoned");
        *settings_guard = settings.clone();
    }
    {
        let mut agent_guard = state.agent.write().expect("agent lock poisoned");
        *agent_guard = state.agent_factory.build(&settings);
    }
    Ok(settings)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatchRequest {
    providers: Option<Vec<ProviderSettings>>,
    active_provider_id: Option<String>,
    provider_kind: Option<ProviderKind>,
    base_url: Option<String>,
    model: Option<String>,
    api_key_source: Option<String>,
    permission_mode: Option<PermissionMode>,
    agent_runtime: Option<AgentRuntimeSettings>,
    default_workspace_root: Option<PathBuf>,
    clear_default_workspace_root: Option<bool>,
    sandbox: Option<SandboxSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderTestRequest {
    provider_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexLoginRequest {
    #[serde(default)]
    device_code: bool,
}
