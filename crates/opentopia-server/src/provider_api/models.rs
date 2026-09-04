use super::{current_settings, save_settings_and_refresh_runtime};
use crate::{
    negotiate_provider_settings, truncate_chars, ApiError, AppState, ProviderAdapterKind,
    ProviderAuthKind, ProviderSettings, ProviderTransportKind, SessionStore, ThreadModelSelection,
    MIN_PROVIDER_CONTEXT_WINDOW_TOKENS,
};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::routing::{post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/provider/:provider_id/models/sync",
            post(sync_provider_models),
        )
        .route("/api/threads/:thread_id/model", put(set_thread_model))
}

async fn sync_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderModelSyncResult>, ApiError> {
    let settings = current_settings(&state);
    let provider = settings
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| ApiError::not_found(format!("provider not found: {provider_id}")))?
        .clone();
    let expected_transport = provider.effective_transport();
    let expected_auth = provider.effective_auth();
    let expected_allowed_adapters = provider.effective_allowed_adapters();
    let expected_preferred_adapter = provider.preferred_adapter;
    let expected_base_url = provider.base_url.clone();
    let expected_model = provider.model.clone();
    let expected_api_key_source = provider.api_key_source.clone();

    if provider.effective_transport() == ProviderTransportKind::Mock {
        return Err(ApiError::bad_request(
            "mock provider has no remote model list",
        ));
    }

    let api_key = match provider.effective_auth() {
        ProviderAuthKind::Bearer | ProviderAuthKind::XApiKey => Some(
            std::env::var(&provider.api_key_source)
                .ok()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "provider '{}' has no configured API key",
                        provider.id
                    ))
                })?,
        ),
        ProviderAuthKind::CodexSession | ProviderAuthKind::None => None,
    };

    let url = provider_model_catalog_url(&provider);
    let client = reqwest::Client::new();
    let mut rate_limit_retries = 0usize;
    let (status, body, retry_after_seconds) = loop {
        let mut request = client.get(&url).timeout(Duration::from_secs(20));
        request = match (provider.effective_auth(), api_key.as_deref()) {
            (ProviderAuthKind::Bearer, Some(api_key)) => {
                request.header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            }
            (ProviderAuthKind::XApiKey, Some(api_key)) => request.header("x-api-key", api_key),
            (ProviderAuthKind::CodexSession | ProviderAuthKind::None, _) => request,
            _ => request,
        };
        if provider.resolved_adapter_for_model(&provider.model)
            == ProviderAdapterKind::AnthropicMessages
        {
            request = request.header("anthropic-version", "2023-06-01");
        }

        let response = request.send().await.map_err(|error| {
            warn!(
                provider_id = %provider.id,
                transport = ?provider.effective_transport(),
                auth = ?provider.effective_auth(),
                adapter = ?provider.resolved_adapter_for_model(&provider.model),
                "model discovery request failed"
            );
            ApiError::bad_gateway(format!("model list request failed: {error}"))
        })?;
        let status = response.status();
        let retry_after_seconds = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = response.text().await.map_err(|error| {
            warn!(
                provider_id = %provider.id,
                transport = ?provider.effective_transport(),
                auth = ?provider.effective_auth(),
                adapter = ?provider.resolved_adapter_for_model(&provider.model),
                "model discovery response could not be read"
            );
            ApiError::bad_gateway(format!("model list read failed: {error}"))
        })?;

        let Some(delay) = provider_model_catalog_rate_limit_delay(
            status,
            retry_after_seconds,
            rate_limit_retries,
        ) else {
            break (status, body, retry_after_seconds);
        };
        rate_limit_retries += 1;
        warn!(
            provider_id = %provider.id,
            retry_index = rate_limit_retries,
            delay_ms = delay.as_millis(),
            "model discovery was rate-limited; retrying"
        );
        tokio::time::sleep(delay).await;
    };
    if !status.is_success() {
        warn!(
            provider_id = %provider.id,
            transport = ?provider.effective_transport(),
            auth = ?provider.effective_auth(),
            adapter = ?provider.resolved_adapter_for_model(&provider.model),
            %status,
            "model discovery endpoint returned a non-success status"
        );
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_hint = retry_after_seconds
                .map(|seconds| format!(" Retry after {seconds} seconds."))
                .unwrap_or_default();
            return Err(ApiError::too_many_requests(format!(
                "provider model list is rate-limited after {rate_limit_retries} retries.{retry_hint} {}",
                truncate_chars(body.trim(), 300)
            )));
        }
        return Err(ApiError::bad_gateway(format!(
            "model list request returned {status}: {}",
            truncate_chars(body.trim(), 300)
        )));
    }
    let payload: Value = serde_json::from_str(&body).map_err(|error| {
        warn!(
            provider_id = %provider.id,
            transport = ?provider.effective_transport(),
            auth = ?provider.effective_auth(),
            adapter = ?provider.resolved_adapter_for_model(&provider.model),
            "model discovery response was not valid JSON"
        );
        ApiError::bad_gateway(format!("model list response was not valid JSON: {error}"))
    })?;

    let catalog = extract_model_catalog(&payload);
    let catalog_default_model = provider_model_catalog_default(&catalog);
    let mut models: Vec<String> = catalog.iter().map(|entry| entry.id.clone()).collect();
    models.sort();
    models.dedup();
    if models.is_empty() {
        warn!(
            provider_id = %provider.id,
            transport = ?provider.effective_transport(),
            auth = ?provider.effective_auth(),
            adapter = ?provider.resolved_adapter_for_model(&provider.model),
            "model discovery response contained no model IDs"
        );
        return Err(ApiError::bad_gateway(
            "model list response contained no model ids",
        ));
    }
    let context_windows: BTreeMap<String, usize> = catalog
        .iter()
        .filter_map(|entry| {
            entry
                .context_window
                .map(|window| (entry.id.clone(), window))
        })
        .collect();
    let model_capabilities: BTreeMap<String, opentopia_core::ProviderModelCapabilities> = catalog
        .into_iter()
        .filter_map(|entry| {
            entry.supports_vision.map(|supports_vision| {
                (
                    entry.id,
                    opentopia_core::ProviderModelCapabilities {
                        supports_vision: Some(supports_vision),
                    },
                )
            })
        })
        .collect();

    let synced_at = Utc::now();
    let mut settings = current_settings(&state);
    let Some(target) = settings
        .providers
        .iter_mut()
        .find(|candidate| candidate.id == provider_id)
    else {
        return Err(ApiError::not_found(format!(
            "provider not found: {provider_id}"
        )));
    };
    if target.effective_transport() != expected_transport
        || target.effective_auth() != expected_auth
        || target.effective_allowed_adapters() != expected_allowed_adapters
        || target.preferred_adapter != expected_preferred_adapter
        || target.base_url != expected_base_url
        || target.model != expected_model
        || target.api_key_source != expected_api_key_source
    {
        return Err(ApiError::conflict(
            "provider settings changed while model discovery was running",
        ));
    }
    target.synced_models = models.clone();
    target.model_context_windows = context_windows.clone();
    target.model_capabilities = model_capabilities.clone();
    if !models.iter().any(|model| model == target.model.trim()) {
        // Setup begins with an empty model. Once the endpoint exposes a
        // catalog, respect the provider's advertised priority instead of
        // accidentally choosing an internal alias by alphabetical order.
        target.model = catalog_default_model
            .clone()
            .unwrap_or_else(|| models[0].clone());
    }
    let default_model = target.model.clone();
    target.models_synced_at = Some(synced_at);
    let provider_to_negotiate = target.clone();
    let catalog_settings = save_settings_and_refresh_runtime(&state, settings)?;
    let catalog_provider = catalog_settings
        .providers
        .iter()
        .find(|candidate| candidate.id == provider_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("provider not found: {provider_id}")))?;

    // Catalog discovery and runtime capability negotiation have different
    // availability boundaries. A relay may keep `/models` healthy while its
    // inference account pool is temporarily rate-limited. Persist the catalog
    // above so a transient generation failure cannot make the connection
    // impossible to import; conversation routes still require a negotiated
    // adapter profile before they can use the selected model.
    let mut default_model_ready = false;
    let mut capability_warning = None;
    let mut synced_provider = catalog_provider;
    match negotiate_provider_settings(&provider_to_negotiate).await {
        Ok(negotiation) => {
            let health = &negotiation.health;
            if !health.reachable || !health.model_available {
                capability_warning = Some(format!(
                    "provider adapter negotiation failed: {}",
                    health.error.clone().unwrap_or_else(|| {
                        "the selected model is not conversation-ready".to_string()
                    })
                ));
            } else if health.openai_compatibility.is_none()
                && negotiation.adapter_profiles.is_empty()
            {
                capability_warning =
                    Some("provider negotiation returned no adapter profile".to_string());
            } else {
                // Negotiation performs network I/O. Re-read settings afterwards
                // instead of committing the pre-await snapshot, otherwise a
                // concurrent settings edit could be silently overwritten.
                let mut latest = current_settings(&state);
                let target = latest
                    .providers
                    .iter_mut()
                    .find(|candidate| candidate.id == provider_id)
                    .ok_or_else(|| {
                        ApiError::not_found(format!("provider not found: {provider_id}"))
                    })?;
                if target.effective_transport() != provider_to_negotiate.effective_transport()
                    || target.effective_auth() != provider_to_negotiate.effective_auth()
                    || target.effective_allowed_adapters()
                        != provider_to_negotiate.effective_allowed_adapters()
                    || target.preferred_adapter != provider_to_negotiate.preferred_adapter
                    || target.base_url != provider_to_negotiate.base_url
                    || target.model != provider_to_negotiate.model
                    || target.api_key_source != provider_to_negotiate.api_key_source
                {
                    return Err(ApiError::conflict(
                        "provider settings changed while adapter negotiation was running",
                    ));
                }
                if let Some(report) = health.openai_compatibility.as_ref() {
                    target.apply_openai_compatibility_report(report.clone());
                } else {
                    for profile in negotiation.adapter_profiles {
                        target.apply_adapter_profile(profile);
                    }
                }
                let negotiated_settings = save_settings_and_refresh_runtime(&state, latest)?;
                synced_provider = negotiated_settings
                    .providers
                    .iter()
                    .find(|candidate| candidate.id == provider_id)
                    .cloned()
                    .ok_or_else(|| {
                        ApiError::not_found(format!("provider not found: {provider_id}"))
                    })?;
                default_model_ready = true;
            }
        }
        Err(error) => {
            capability_warning = Some(format!("provider adapter negotiation failed: {error}"));
        }
    }

    info!(
        provider_id = %provider_id,
        transport = ?provider.effective_transport(),
        auth = ?provider.effective_auth(),
        adapter = ?provider.resolved_adapter_for_model(&provider.model),
        model_count = models.len(),
        default_model = %default_model,
        default_model_ready,
        has_capability_warning = capability_warning.is_some(),
        "model discovery completed"
    );

    Ok(Json(ProviderModelSyncResult {
        provider_id,
        models,
        model_context_windows: context_windows,
        model_capabilities,
        default_model,
        synced_at,
        default_model_ready,
        capability_warning,
        provider: synced_provider,
    }))
}

const PROVIDER_MODEL_CATALOG_RATE_LIMIT_RETRY_DELAYS: [Duration; 2] =
    [Duration::from_secs(2), Duration::from_secs(5)];
const PROVIDER_MODEL_CATALOG_MAX_INLINE_RETRY_AFTER: Duration = Duration::from_secs(15);

pub(crate) fn provider_model_catalog_rate_limit_delay(
    status: StatusCode,
    retry_after_seconds: Option<u64>,
    retry_index: usize,
) -> Option<Duration> {
    if status != StatusCode::TOO_MANY_REQUESTS
        || retry_index >= PROVIDER_MODEL_CATALOG_RATE_LIMIT_RETRY_DELAYS.len()
    {
        return None;
    }
    let delay = retry_after_seconds
        .map(Duration::from_secs)
        .unwrap_or(PROVIDER_MODEL_CATALOG_RATE_LIMIT_RETRY_DELAYS[retry_index]);
    (delay <= PROVIDER_MODEL_CATALOG_MAX_INLINE_RETRY_AFTER).then_some(delay)
}

pub(crate) fn provider_model_catalog_url(provider: &ProviderSettings) -> String {
    let base_url = provider.base_url.trim_end_matches('/');
    if base_url.ends_with("/v1") {
        format!("{base_url}/models")
    } else {
        // The setup form accepts both a service root and an OpenAI-compatible
        // API root. Normalizing here means users can provide either
        // `https://service.example` or `https://service.example/v1`.
        format!("{base_url}/v1/models")
    }
}

/// Model ids paired with the context window the endpoint reported, when it
/// reports one at all.
///
/// Accepts the OpenAI and Anthropic (`{"data":[{"id":...}]}`) shapes plus the
/// bare arrays some relays return.
///
/// This is the only genuine capability detection in the system: OpenAI's own
/// `/v1/models` returns nothing but ids, but OpenRouter, vLLM, LiteLLM and many
/// relay panels do publish a window, and a value from the endpoint always beats
/// the hand-maintained table in `settings.rs`.
pub(crate) struct DiscoveredModel {
    pub(crate) id: String,
    pub(crate) context_window: Option<usize>,
    pub(crate) supports_vision: Option<bool>,
}

pub(crate) fn extract_model_catalog(payload: &Value) -> Vec<DiscoveredModel> {
    let entries = payload
        .get("data")
        .or_else(|| payload.get("models"))
        .or(Some(payload));
    let Some(entries) = entries.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            Value::String(id) => Some(DiscoveredModel {
                id: id.trim().to_string(),
                context_window: None,
                supports_vision: None,
            }),
            Value::Object(_) => {
                let id = entry
                    .get("id")
                    .or_else(|| entry.get("name"))
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                Some(DiscoveredModel {
                    id,
                    context_window: extract_context_window(entry),
                    supports_vision: extract_supports_vision(entry),
                })
            }
            _ => None,
        })
        .filter(|entry| !entry.id.is_empty())
        .collect()
}

pub(crate) fn provider_model_catalog_default(catalog: &[DiscoveredModel]) -> Option<String> {
    catalog.first().map(|entry| entry.id.clone())
}

/// Reads common catalog modality shapes. OpenAI's own `/v1/models` endpoint
/// only returns model ids, so a missing field remains unknown rather than being
/// guessed. Relays such as OpenRouter commonly expose `input_modalities`.
pub(crate) fn extract_supports_vision(entry: &Value) -> Option<bool> {
    const BOOLEAN_FIELDS: [&str; 2] = ["supports_vision", "vision"];
    const MODALITY_FIELDS: [&str; 3] = [
        "input_modalities",
        "modalities",
        "input_modalities_supported",
    ];

    let read_boolean = |object: &Value| {
        BOOLEAN_FIELDS
            .iter()
            .find_map(|field| object.get(*field).and_then(Value::as_bool))
    };
    if let Some(value) =
        read_boolean(entry).or_else(|| entry.get("capabilities").and_then(read_boolean))
    {
        return Some(value);
    }

    let has_image_modality = |value: &Value| {
        let modalities: Vec<String> = match value {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| value.trim().to_ascii_lowercase())
                .collect(),
            Value::String(value) => value
                .split(|character: char| matches!(character, ',' | '+' | '/' | ' '))
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect(),
            _ => return None,
        };
        Some(modalities.iter().any(|modality| {
            matches!(
                modality.as_str(),
                "image" | "images" | "vision" | "image_url"
            )
        }))
    };
    MODALITY_FIELDS.iter().find_map(|field| {
        entry
            .get(*field)
            .or_else(|| {
                entry
                    .get("architecture")
                    .and_then(|value| value.get(*field))
            })
            .and_then(has_image_modality)
    })
}

/// Reads whichever context-window field the endpoint happens to use. Values are
/// sanity-checked so a bogus catalog cannot inflate the window and overflow the
/// real limit mid-conversation.
pub(crate) fn extract_context_window(entry: &Value) -> Option<usize> {
    const CONTEXT_WINDOW_FIELDS: [&str; 8] = [
        "context_length",   // OpenRouter, many relay panels
        "max_model_len",    // vLLM
        "context_window",   // assorted gateways
        "max_input_tokens", // LiteLLM
        "context_size",
        "max_context_length",
        "max_context_tokens",
        "max_sequence_length",
    ];

    let read_window = |object: &Value| {
        CONTEXT_WINDOW_FIELDS.iter().find_map(|field| {
            object.get(*field).and_then(|value| {
                value.as_u64().or_else(|| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .and_then(|value| value.parse::<u64>().ok())
                })
            })
        })
    };
    let direct = read_window(entry);
    // OpenRouter and several relay catalogs nest provider-specific limits.
    let nested = entry.get("top_provider").and_then(read_window);

    direct
        .or(nested)
        .filter(|tokens| *tokens >= MIN_PROVIDER_CONTEXT_WINDOW_TOKENS as u64)
        .filter(|tokens| *tokens <= MAX_REPORTED_CONTEXT_WINDOW_TOKENS as u64)
        .map(|tokens| tokens as usize)
}

/// Upper bound on a self-reported window. Guards against catalogs that publish
/// byte counts or placeholder values where a token count belongs.
const MAX_REPORTED_CONTEXT_WINDOW_TOKENS: usize = 20_000_000;

/// Ensures a model switch has a persisted adapter contract before the thread
/// can reference it. Negotiation is a settings concern; turn execution never
/// probes capabilities or rewrites a request after it has been encoded.
async fn ensure_thread_model_adapter_ready(
    state: &AppState,
    selection: &ThreadModelSelection,
) -> Result<(), ApiError> {
    let settings = current_settings(state);
    let connection = settings
        .providers
        .iter()
        .find(|provider| provider.id == selection.connection_id)
        .ok_or_else(|| {
            ApiError::bad_request(format!("unknown connection: {}", selection.connection_id))
        })?;
    if connection.effective_transport() != ProviderTransportKind::Http {
        return Ok(());
    }
    let adapter = connection.resolved_adapter_for_model(&selection.model_id);
    if !connection.allows_adapter(adapter) {
        return Err(ApiError::bad_request(format!(
            "adapter '{}' is not enabled for connection '{}'",
            adapter.as_str(),
            connection.id
        )));
    }
    if connection
        .adapter_profile_for_model_and_adapter(&selection.model_id, adapter)
        .is_some()
    {
        return Ok(());
    }

    let expected_transport = connection.effective_transport();
    let expected_auth = connection.effective_auth();
    let expected_allowed_adapters = connection.effective_allowed_adapters();
    let expected_preferred_adapter = connection.preferred_adapter;
    let expected_base_url = connection.base_url.clone();
    let expected_api_key_source = connection.api_key_source.clone();
    let candidate = connection.with_model_route_override(
        Some(selection.model_id.as_str()),
        Some(selection.reasoning_effort.as_deref()),
        Some(adapter),
    );
    let negotiation = negotiate_provider_settings(&candidate).await?;
    let health = &negotiation.health;
    if !health.reachable || !health.model_available {
        return Err(ApiError::bad_gateway(format!(
            "model adapter negotiation failed: {}",
            health
                .error
                .clone()
                .unwrap_or_else(|| "the selected model is not conversation-ready".to_string())
        )));
    }

    let mut latest = current_settings(state);
    let target = latest
        .providers
        .iter_mut()
        .find(|provider| provider.id == selection.connection_id)
        .ok_or_else(|| {
            ApiError::bad_request(format!("unknown connection: {}", selection.connection_id))
        })?;
    if target.effective_transport() != expected_transport
        || target.effective_auth() != expected_auth
        || target.effective_allowed_adapters() != expected_allowed_adapters
        || target.preferred_adapter != expected_preferred_adapter
        || target.base_url != expected_base_url
        || target.api_key_source != expected_api_key_source
    {
        return Err(ApiError::conflict(
            "provider settings changed while adapter negotiation was running",
        ));
    }
    if negotiation.adapter_profiles.is_empty() {
        return Err(ApiError::bad_gateway(
            "provider negotiation returned no adapter profile",
        ));
    }
    if negotiation
        .adapter_profiles
        .iter()
        .any(|profile| !profile.applies_to(&target.base_url, &selection.model_id))
    {
        return Err(ApiError::conflict(
            "provider settings changed while adapter negotiation was running",
        ));
    }
    if let Some(report) = health.openai_compatibility.as_ref() {
        target.apply_openai_compatibility_report(report.clone());
    } else {
        for profile in negotiation.adapter_profiles {
            target.apply_adapter_profile(profile);
        }
    }
    if target
        .adapter_profile_for_model_and_adapter(&selection.model_id, adapter)
        .is_none()
    {
        return Err(ApiError::bad_gateway(format!(
            "adapter '{}' is not available for model '{}'",
            adapter.as_str(),
            selection.model_id
        )));
    }
    save_settings_and_refresh_runtime(state, latest)?;
    Ok(())
}

/// Pins (or clears) the model a thread runs with.
async fn set_thread_model(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<ThreadModelRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    let selection = match request.selection {
        Some(selection) => {
            let settings = current_settings(&state);
            if !settings
                .providers
                .iter()
                .any(|provider| provider.id == selection.connection_id)
            {
                return Err(ApiError::bad_request(format!(
                    "unknown connection: {}",
                    selection.connection_id
                )));
            }
            if selection.model_id.trim().is_empty() {
                return Err(ApiError::bad_request("modelId cannot be empty"));
            }
            ensure_thread_model_adapter_ready(&state, &selection).await?;
            Some(selection)
        }
        None => None,
    };
    state
        .store
        .set_thread_model_selection(thread_id, selection)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderModelSyncResult {
    provider_id: String,
    models: Vec<String>,
    model_context_windows: BTreeMap<String, usize>,
    model_capabilities: BTreeMap<String, opentopia_core::ProviderModelCapabilities>,
    default_model: String,
    synced_at: DateTime<Utc>,
    default_model_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    capability_warning: Option<String>,
    provider: ProviderSettings,
}

/// `selection: null` clears the pin and returns the thread to the active
/// connection's default model.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadModelRequest {
    #[serde(default)]
    selection: Option<ThreadModelSelection>,
}
