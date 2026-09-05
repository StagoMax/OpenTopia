use super::mcp_api::McpServerView;
use super::plugin_runtime::{
    load_plugin_outcome, load_plugin_outcome_for_thread, sync_plugin_mcp_configs, LoadedPlugin,
};
use super::{ensure_thread, load_bound_agent_context, ApiError, AppState, DeleteResponse};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use opentopia_core::{
    discover_plugins, discover_skills, inspect_plugin_control_manifest, install_plugin,
    permission_requested, uninstall_plugin, validate_plugin_settings, CapabilityActivationSnapshot,
    CapabilityProjection, ExperienceMode, ExperienceSurfaceProfile, PluginActivationRecord,
    PluginActivationScope, PluginActivationScopeType, PluginContribution, PluginControlManifest,
    PluginControlScope, PluginControlScopeType, PluginDescriptor, PluginError,
    PluginPermissionGrantRecord, PluginPermissionGrantStatus, PluginRuntimeHealthRecord,
    PluginSecretBindingRecord, PluginSettingsRecord, PluginSource, SessionStore, SkillDescriptor,
    SqliteSessionStore,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/install", post(install_local_plugin))
        .route("/api/plugins/uninstall", post(uninstall_local_plugin))
        .route("/api/plugins/:plugin_id", get(get_plugin_detail))
        .route(
            "/api/plugins/:plugin_id/activation",
            put(put_plugin_activation),
        )
        .route(
            "/api/plugins/:plugin_id/settings",
            get(get_plugin_settings).patch(patch_plugin_settings),
        )
        .route(
            "/api/plugins/:plugin_id/permissions",
            get(get_plugin_permissions).put(put_plugin_permission),
        )
        .route(
            "/api/plugins/:plugin_id/contributions",
            get(get_plugin_contributions),
        )
        .route("/api/plugins/:plugin_id/health", get(get_plugin_health))
        .route(
            "/api/threads/:thread_id/capabilities",
            get(get_thread_capabilities),
        )
}

async fn list_plugins(
    State(state): State<AppState>,
    Query(query): Query<PluginsQuery>,
) -> Result<Json<Vec<PluginView>>, ApiError> {
    let (workspace_root, thread_id) = resolve_plugin_context(&state, query)?;
    let outcome = load_plugin_outcome(&state.store, workspace_root.as_deref(), thread_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let skills = discover_skills(workspace_root.as_deref());
    let mut views = Vec::with_capacity(outcome.plugins().len());
    for plugin in outcome.plugins() {
        sync_plugin_mcp_configs(&state.store, &state.mcp_host, &plugin.descriptor)
            .await
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        views.push(plugin_view(&state, plugin, &skills).await?);
    }
    Ok(Json(views))
}

async fn install_local_plugin(
    State(state): State<AppState>,
    Json(request): Json<InstallPluginRequest>,
) -> Result<Json<PluginView>, ApiError> {
    let source = request.path;
    let plugin = tokio::task::spawn_blocking(move || install_plugin(&source))
        .await
        .map_err(|error| ApiError::internal(format!("plugin installation failed: {error}")))?
        .map_err(plugin_bad_request)?;
    state
        .store
        .migrate_plugin_identity(&plugin.id, &plugin.legacy_ids)?;
    state
        .store
        .set_plugin_activation(&plugin.id, &PluginActivationScope::global(), true)?;
    sync_plugin_mcp_configs(&state.store, &state.mcp_host, &plugin)
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let outcome = load_plugin_outcome(&state.store, None, None)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let loaded = outcome
        .plugin(&plugin.id)
        .ok_or_else(|| ApiError::internal("installed plugin was not discoverable"))?;
    let skills = discover_skills(None);
    Ok(Json(plugin_view(&state, loaded, &skills).await?))
}

async fn uninstall_local_plugin(
    State(state): State<AppState>,
    Json(request): Json<UninstallPluginRequest>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let workspace_root = validate_plugin_workspace(&state, request.workspace_root)?;
    let plugin_id = request.plugin_id;
    let plugin_servers = state.store.list_plugin_mcp_servers(&plugin_id)?;
    let uninstall_id = plugin_id.clone();
    let uninstall_root = workspace_root.clone();
    tokio::task::spawn_blocking(move || uninstall_plugin(&uninstall_id, uninstall_root.as_deref()))
        .await
        .map_err(|error| ApiError::internal(format!("plugin removal failed: {error}")))?
        .map_err(plugin_bad_request)?;
    for server in plugin_servers {
        state.mcp_host.stop_server(server.server_id).await.ok();
        state.store.delete_mcp_server(server.server_id)?;
    }
    state.store.delete_plugin_configuration(&plugin_id)?;
    Ok(Json(DeleteResponse { deleted: true }))
}

async fn plugin_view(
    state: &AppState,
    loaded: &LoadedPlugin,
    skills: &[SkillDescriptor],
) -> Result<PluginView, ApiError> {
    let plugin = &loaded.descriptor;
    let skill_ids = skills
        .iter()
        .filter(|skill| skill.plugin_id.as_deref() == Some(plugin.id.as_str()))
        .map(|skill| skill.id.clone())
        .collect::<Vec<_>>();
    let servers = state.store.list_plugin_mcp_servers(&plugin.id)?;
    let mut mcp_servers = Vec::with_capacity(servers.len());
    for server in servers {
        let status = state.mcp_host.status_for_config(&server).await;
        mcp_servers.push(McpServerView { server, status });
    }
    Ok(PluginView {
        compatible: plugin.is_compatible(),
        plugin: plugin.clone(),
        skill_ids,
        mcp_servers,
        effective_enabled: loaded.enabled,
    })
}

fn resolve_plugin_context(
    state: &AppState,
    query: PluginsQuery,
) -> Result<(Option<PathBuf>, Option<Uuid>), ApiError> {
    if let Some(thread_id) = query.thread_id {
        let thread = ensure_thread(state, thread_id)?;
        if query
            .workspace_root
            .as_ref()
            .is_some_and(|root| root != &thread.workspace_root)
        {
            return Err(ApiError::bad_request(
                "plugin workspace does not match the selected thread",
            ));
        }
        return Ok((Some(thread.workspace_root), Some(thread_id)));
    }
    Ok((
        validate_plugin_workspace(state, query.workspace_root)?,
        None,
    ))
}

fn validate_plugin_workspace(
    state: &AppState,
    workspace_root: Option<PathBuf>,
) -> Result<Option<PathBuf>, ApiError> {
    if let Some(workspace_root) = workspace_root {
        if state
            .store
            .find_project_by_workspace(&workspace_root)?
            .is_none()
        {
            return Err(ApiError::bad_request(
                "workspace is not registered as a project",
            ));
        }
        Ok(Some(workspace_root))
    } else {
        Ok(None)
    }
}

fn plugin_bad_request(error: PluginError) -> ApiError {
    ApiError::bad_request(error.to_string())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginsQuery {
    workspace_root: Option<PathBuf>,
    thread_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallPluginRequest {
    path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UninstallPluginRequest {
    plugin_id: String,
    workspace_root: Option<PathBuf>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginView {
    plugin: PluginDescriptor,
    skill_ids: Vec<String>,
    mcp_servers: Vec<McpServerView>,
    effective_enabled: bool,
    compatible: bool,
}

async fn get_plugin_detail(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginContextQuery>,
) -> Result<Json<PluginDetailResponse>, ApiError> {
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &query)?;
    let workspace_root = resolve_context(&state, &query)?;
    let effective_enabled = state.store.plugin_effectively_enabled(
        &plugin.id,
        plugin.default_enabled,
        workspace_root.as_deref(),
    )?;
    Ok(Json(PluginDetailResponse {
        plugin: plugin.clone(),
        contributions: manifest.contributions.clone(),
        manifest,
        activations: state.store.list_plugin_activations(&plugin.id)?,
        effective_enabled,
        health: state.store.list_plugin_runtime_health(&plugin.id)?,
    }))
}

async fn put_plugin_activation(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginActivationRequest>,
) -> Result<Json<PluginActivationResponse>, ApiError> {
    let scope = validate_activation_scope(&state, &request.scope)?;
    let query = context_for_activation_scope(&state, &scope)?;
    let (plugin, _) = prepare_plugin(&state, &plugin_id, &query)?;
    let activation = state
        .store
        .set_plugin_activation(&plugin.id, &scope, request.enabled)?;
    let effective_enabled = state.store.plugin_effectively_enabled(
        &plugin.id,
        plugin.default_enabled,
        query.workspace_root.as_deref(),
    )?;
    Ok(Json(PluginActivationResponse {
        activation,
        effective_enabled,
    }))
}

async fn get_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginScopedQuery>,
) -> Result<Json<PluginSettingsResponse>, ApiError> {
    let scope = scope_from_query(&state, &query)?;
    let context = context_for_scope(&state, &scope)?;
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &context)?;
    let mut settings = state
        .store
        .get_plugin_settings(&plugin.id, &scope)?
        .unwrap_or_else(|| PluginSettingsRecord {
            plugin_id: plugin.id.clone(),
            scope: scope.clone(),
            settings: Value::Object(Map::new()),
            updated_at: Utc::now(),
        });
    remove_secret_settings(&mut settings.settings, &manifest.secret_setting_keys)?;
    Ok(Json(PluginSettingsResponse {
        schema: manifest.configuration_schema,
        settings,
        secret_bindings: state
            .store
            .list_plugin_secret_bindings(&plugin.id, &scope)?,
    }))
}

async fn patch_plugin_settings(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginSettingsPatchRequest>,
) -> Result<Json<PluginSettingsResponse>, ApiError> {
    let scope = validate_scope(&state, &request.scope)?;
    let context = context_for_scope(&state, &scope)?;
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &context)?;
    let secret_keys = manifest
        .secret_setting_keys
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(key) = request
        .settings
        .keys()
        .find(|key| secret_keys.contains(*key))
    {
        return Err(ApiError::bad_request(format!(
            "secret setting `{key}` must use an opaque binding ID"
        )));
    }
    let mut settings = state
        .store
        .get_plugin_settings(&plugin.id, &scope)?
        .map(|record| record.settings)
        .unwrap_or_else(|| Value::Object(Map::new()));
    remove_secret_settings(&mut settings, &manifest.secret_setting_keys)?;
    let values = settings
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("stored plugin settings are not an object"))?;
    for (key, value) in request.settings {
        if value.is_null() {
            values.remove(&key);
        } else {
            values.insert(key, value);
        }
    }
    validate_plugin_settings(manifest.configuration_schema.as_ref(), &settings)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let mut prospective_bindings = state
        .store
        .list_plugin_secret_bindings(&plugin.id, &scope)?
        .into_iter()
        .map(|binding| binding.setting_key)
        .collect::<BTreeSet<_>>();
    for (key, binding_id) in &request.secret_bindings {
        if !secret_keys.contains(key) {
            return Err(ApiError::bad_request(format!(
                "`{key}` is not declared as a secret setting"
            )));
        }
        if binding_id.is_some() {
            prospective_bindings.insert(key.clone());
        } else {
            prospective_bindings.remove(key);
        }
    }
    for key in &manifest.required_secret_setting_keys {
        if !prospective_bindings.contains(key) {
            return Err(ApiError::bad_request(format!(
                "secret setting `{key}` requires an opaque binding ID"
            )));
        }
    }
    for (key, binding_id) in request.secret_bindings {
        match binding_id {
            Some(binding_id) => {
                state.store.put_plugin_secret_binding(
                    &plugin.id,
                    &scope,
                    &key,
                    &binding_id,
                    &Value::Object(Map::new()),
                )?;
            }
            None => {
                state
                    .store
                    .delete_plugin_secret_binding(&plugin.id, &scope, &key)?;
            }
        }
    }
    let settings = state
        .store
        .put_plugin_settings(&plugin.id, &scope, &settings)?;
    Ok(Json(PluginSettingsResponse {
        schema: manifest.configuration_schema,
        settings,
        secret_bindings: state
            .store
            .list_plugin_secret_bindings(&plugin.id, &scope)?,
    }))
}

fn remove_secret_settings(settings: &mut Value, secret_keys: &[String]) -> Result<(), ApiError> {
    let values = settings
        .as_object_mut()
        .ok_or_else(|| ApiError::internal("stored plugin settings are not an object"))?;
    for key in secret_keys {
        values.remove(key);
    }
    Ok(())
}

async fn get_plugin_permissions(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginContextQuery>,
) -> Result<Json<PluginPermissionsResponse>, ApiError> {
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &query)?;
    Ok(Json(PluginPermissionsResponse {
        requests: manifest.permission_requests,
        grants: state.store.list_plugin_permission_grants(&plugin.id)?,
    }))
}

async fn put_plugin_permission(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginPermissionRequestBody>,
) -> Result<Json<PluginPermissionGrantRecord>, ApiError> {
    if !request.constraint.is_object() {
        return Err(ApiError::bad_request(
            "plugin permission constraint must be a JSON object",
        ));
    }
    let scope = validate_scope(&state, &request.scope)?;
    let context = context_for_scope(&state, &scope)?;
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &context)?;
    if !permission_requested(&manifest, &request.permission) {
        return Err(ApiError::bad_request(format!(
            "plugin manifest does not request permission `{}`",
            request.permission
        )));
    }
    Ok(Json(state.store.set_manifest_plugin_permission_grant(
        &plugin.id,
        &manifest,
        &scope,
        &request.permission,
        &request.constraint,
        if request.granted {
            PluginPermissionGrantStatus::Granted
        } else {
            PluginPermissionGrantStatus::Revoked
        },
    )?))
}

async fn get_plugin_contributions(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginContextQuery>,
) -> Result<Json<Vec<PluginContribution>>, ApiError> {
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &query)?;
    debug_assert!(manifest
        .contributions
        .iter()
        .all(|contribution| contribution.plugin_id == plugin.id));
    Ok(Json(manifest.contributions))
}

async fn get_plugin_health(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginContextQuery>,
) -> Result<Json<Vec<PluginRuntimeHealthRecord>>, ApiError> {
    let (plugin, _) = prepare_plugin(&state, &plugin_id, &query)?;
    Ok(Json(state.store.list_plugin_runtime_health(&plugin.id)?))
}

async fn get_thread_capabilities(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<ThreadCapabilitiesResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let surface_profile = ExperienceSurfaceProfile::for_mode(thread.experience_mode);
    let mut effective_capabilities = surface_profile.capabilities.clone();
    if let (Some(instance), _) = load_bound_agent_context(&state, &thread)? {
        effective_capabilities =
            effective_capabilities.intersect(&instance.execution_context.capabilities);
    }
    let outcome = load_plugin_outcome_for_thread(&state.store, &thread)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let mut plugins = Vec::new();
    for loaded in outcome.plugins() {
        let plugin = &loaded.descriptor;
        let enabled = loaded.enabled
            && (effective_capabilities.allows_plugin(&plugin.id)
                || effective_capabilities.allows_plugin(&plugin.name));
        plugins.push(ThreadPluginCapabilities {
            plugin_id: plugin.id.clone(),
            plugin_name: plugin.name.clone(),
            enabled,
            contributions: enabled
                .then(|| {
                    outcome
                        .active_contributions()
                        .filter(|contribution| contribution.plugin_id == plugin.id)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default(),
            granted_permissions: loaded.granted_permissions.clone(),
        });
    }
    plugins.sort_by(|left, right| left.plugin_name.cmp(&right.plugin_name));
    let snapshot = outcome.capability_snapshot().clone();
    Ok(Json(ThreadCapabilitiesResponse {
        thread_id,
        experience_mode: thread.experience_mode,
        prompt_profile_id: surface_profile.prompt_profile_id,
        capability_projection: effective_capabilities,
        workspace_root: thread.workspace_root,
        generated_at: Utc::now(),
        snapshot,
        plugins,
    }))
}

pub(crate) fn ensure_default_bundled_plugin_permissions(
    store: &SqliteSessionStore,
) -> anyhow::Result<()> {
    for plugin in discover_plugins(None)
        .into_iter()
        .filter(|plugin| plugin.source == PluginSource::Bundled && plugin.default_enabled)
    {
        store.migrate_plugin_identity(&plugin.id, &plugin.legacy_ids)?;
        // Bootstrap each official default exactly once. Any existing grant or
        // revocation is an explicit user decision and remains authoritative.
        if !store.list_plugin_permission_grants(&plugin.id)?.is_empty() {
            continue;
        }

        let manifest = inspect_plugin_control_manifest(&plugin)?;
        for request in &manifest.permission_requests {
            store.set_manifest_plugin_permission_grant(
                &plugin.id,
                &manifest,
                &PluginControlScope::global(),
                &request.permission,
                &Value::Null,
                PluginPermissionGrantStatus::Granted,
            )?;
        }
    }
    Ok(())
}

fn prepare_plugin(
    state: &AppState,
    plugin_id: &str,
    query: &PluginContextQuery,
) -> Result<(PluginDescriptor, PluginControlManifest), ApiError> {
    let workspace_root = resolve_context(state, query)?;
    let outcome = load_plugin_outcome(&state.store, workspace_root.as_deref(), query.thread_id)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let plugin = outcome
        .plugin(plugin_id)
        .map(|plugin| plugin.descriptor.clone())
        .ok_or_else(|| ApiError::not_found("plugin is not available in this context"))?;
    let manifest = inspect_plugin_control_manifest(&plugin)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok((plugin, manifest))
}

fn resolve_context(
    state: &AppState,
    query: &PluginContextQuery,
) -> Result<Option<PathBuf>, ApiError> {
    if let Some(thread_id) = query.thread_id {
        let thread = ensure_thread(state, thread_id)?;
        if query
            .workspace_root
            .as_ref()
            .is_some_and(|root| root != &thread.workspace_root)
        {
            return Err(ApiError::bad_request(
                "workspaceRoot does not match the thread workspace",
            ));
        }
        return Ok(Some(thread.workspace_root));
    }
    if let Some(workspace_root) = &query.workspace_root {
        if state
            .store
            .find_project_by_workspace(workspace_root)?
            .is_none()
        {
            return Err(ApiError::bad_request(
                "workspace is not registered as a project",
            ));
        }
        return Ok(Some(workspace_root.clone()));
    }
    Ok(None)
}

fn validate_scope(
    state: &AppState,
    scope: &PluginControlScope,
) -> Result<PluginControlScope, ApiError> {
    let scope = scope
        .normalized()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    match scope.scope_type {
        PluginControlScopeType::Global => {}
        PluginControlScopeType::Workspace => {
            let scope_id = scope.scope_id.as_deref().unwrap_or_default();
            let registered = state.store.list_projects()?.into_iter().any(|project| {
                project
                    .workspace_root
                    .as_deref()
                    .is_some_and(|root| opentopia_core::normalize_workspace_key(root) == scope_id)
            });
            if !registered {
                return Err(ApiError::bad_request(
                    "workspace scope is not registered as a project",
                ));
            }
        }
        PluginControlScopeType::Thread => {
            let thread_id = Uuid::parse_str(scope.scope_id.as_deref().unwrap_or_default())
                .map_err(|_| ApiError::bad_request("thread scopeId must be a UUID"))?;
            ensure_thread(state, thread_id)?;
        }
    }
    Ok(scope)
}

fn validate_activation_scope(
    state: &AppState,
    scope: &PluginActivationScope,
) -> Result<PluginActivationScope, ApiError> {
    let scope = scope
        .normalized()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if scope.scope_type == PluginActivationScopeType::Workspace {
        let scope_id = scope.scope_id.as_deref().unwrap_or_default();
        let registered = state.store.list_projects()?.into_iter().any(|project| {
            project
                .workspace_root
                .as_deref()
                .is_some_and(|root| opentopia_core::normalize_workspace_key(root) == scope_id)
        });
        if !registered {
            return Err(ApiError::bad_request(
                "workspace scope is not registered as a project",
            ));
        }
    }
    Ok(scope)
}

fn context_for_activation_scope(
    state: &AppState,
    scope: &PluginActivationScope,
) -> Result<PluginContextQuery, ApiError> {
    match scope.scope_type {
        PluginActivationScopeType::Global => Ok(PluginContextQuery::default()),
        PluginActivationScopeType::Workspace => {
            let scope_id = scope.scope_id.as_deref().unwrap_or_default();
            let workspace_root = state
                .store
                .list_projects()?
                .into_iter()
                .filter_map(|project| project.workspace_root)
                .find(|root| opentopia_core::normalize_workspace_key(root) == scope_id)
                .ok_or_else(|| ApiError::bad_request("workspace scope is not registered"))?;
            Ok(PluginContextQuery {
                workspace_root: Some(workspace_root),
                thread_id: None,
            })
        }
    }
}

fn context_for_scope(
    state: &AppState,
    scope: &PluginControlScope,
) -> Result<PluginContextQuery, ApiError> {
    match scope.scope_type {
        PluginControlScopeType::Global => Ok(PluginContextQuery::default()),
        PluginControlScopeType::Workspace => {
            let scope_id = scope.scope_id.as_deref().unwrap_or_default();
            let workspace_root = state
                .store
                .list_projects()?
                .into_iter()
                .filter_map(|project| project.workspace_root)
                .find(|root| opentopia_core::normalize_workspace_key(root) == scope_id)
                .ok_or_else(|| ApiError::bad_request("workspace scope is not registered"))?;
            Ok(PluginContextQuery {
                workspace_root: Some(workspace_root),
                thread_id: None,
            })
        }
        PluginControlScopeType::Thread => {
            let thread_id = Uuid::parse_str(scope.scope_id.as_deref().unwrap_or_default())
                .map_err(|_| ApiError::bad_request("thread scopeId must be a UUID"))?;
            let thread = ensure_thread(state, thread_id)?;
            Ok(PluginContextQuery {
                workspace_root: Some(thread.workspace_root),
                thread_id: Some(thread_id),
            })
        }
    }
}

fn scope_from_query(
    state: &AppState,
    query: &PluginScopedQuery,
) -> Result<PluginControlScope, ApiError> {
    validate_scope(
        state,
        &PluginControlScope {
            scope_type: query.scope_type,
            scope_id: query.scope_id.clone(),
        },
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginContextQuery {
    workspace_root: Option<PathBuf>,
    thread_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginScopedQuery {
    scope_type: PluginControlScopeType,
    scope_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginActivationRequest {
    scope: PluginActivationScope,
    enabled: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginActivationResponse {
    activation: PluginActivationRecord,
    effective_enabled: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginDetailResponse {
    plugin: PluginDescriptor,
    manifest: PluginControlManifest,
    activations: Vec<PluginActivationRecord>,
    effective_enabled: bool,
    contributions: Vec<PluginContribution>,
    health: Vec<PluginRuntimeHealthRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginSettingsPatchRequest {
    scope: PluginControlScope,
    #[serde(default)]
    settings: Map<String, Value>,
    #[serde(default)]
    secret_bindings: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginSettingsResponse {
    schema: Option<Value>,
    settings: PluginSettingsRecord,
    secret_bindings: Vec<PluginSecretBindingRecord>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PluginPermissionsResponse {
    requests: Vec<opentopia_core::PluginPermissionRequest>,
    grants: Vec<PluginPermissionGrantRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginPermissionRequestBody {
    scope: PluginControlScope,
    permission: String,
    #[serde(default = "empty_json_object")]
    constraint: Value,
    granted: bool,
}

fn empty_json_object() -> Value {
    Value::Object(Map::new())
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadCapabilitiesResponse {
    thread_id: Uuid,
    experience_mode: ExperienceMode,
    prompt_profile_id: String,
    capability_projection: CapabilityProjection,
    workspace_root: PathBuf,
    generated_at: DateTime<Utc>,
    snapshot: CapabilityActivationSnapshot,
    plugins: Vec<ThreadPluginCapabilities>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ThreadPluginCapabilities {
    plugin_id: String,
    plugin_name: String,
    enabled: bool,
    contributions: Vec<PluginContribution>,
    granted_permissions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_runtime::effective_granted_permissions;
    use opentopia_core::{CapabilityUnavailableReason, ContributionKind};
    use serde_json::json;
    use std::fs;

    struct TestWorkspace(PathBuf);

    impl TestWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("opentopia-plugin-projection-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("create test workspace");
            Self(root)
        }

        fn install_projection_plugin(&self, profile_id: &str) {
            let root = self.0.join(".opentopia/plugins/projection-pack");
            fs::create_dir_all(root.join(".codex-plugin")).expect("create manifest directory");
            fs::create_dir_all(root.join("agents")).expect("create agent directory");
            fs::create_dir_all(root.join("apps")).expect("create app directory");
            let manifest = json!({
                "name": "projection-pack",
                "version": "1.0.0",
                "opentopia": {
                    "apiVersion": "1",
                    "requires": {
                        "hostCapabilities": [
                            "previewer.v1",
                            "agentProfile.v1",
                            "scmConnector.v1",
                            "appView.v1"
                        ]
                    },
                    "permissions": {
                        "filesystem": ["workspace:read"],
                        "network": [],
                        "secrets": [],
                        "desktop": []
                    },
                    "contributes": {
                        "previewers": [{
                            "id": "report-preview",
                            "extensions": ["report"],
                            "runtime": "mcp:projection"
                        }],
                        "agentProfiles": [{
                            "id": profile_id,
                            "path": format!("./agents/{profile_id}.toml")
                        }],
                        "scmConnectors": [{
                            "id": "code-host",
                            "displayName": "Code Host",
                            "remoteMatchers": [{
                                "matcherId": "code.example.test",
                                "schemes": ["https", "ssh"],
                                "host": {"type": "exact", "value": "code.example.test"},
                                "path": {"type": "any"}
                            }]
                        }],
                        "apps": [{
                            "id": "dashboard",
                            "entry": "apps/dashboard.html",
                            "allowedChannels": ["refresh"]
                        }]
                    }
                }
            });
            fs::write(
                root.join(".codex-plugin/plugin.json"),
                serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
            )
            .expect("write manifest");
            fs::write(
                root.join(format!("agents/{profile_id}.toml")),
                format!(
                    "name = \"{profile_id}\"\ndescription = \"Projection test profile\"\ndeveloper_instructions = \"Stay inside the projection test.\"\n"
                ),
            )
            .expect("write profile");
            fs::write(
                root.join("apps/dashboard.html"),
                "<!doctype html><title>Projection dashboard</title>",
            )
            .expect("write app entry");
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn set_all_permissions(
        store: &SqliteSessionStore,
        plugin: &PluginDescriptor,
        status: PluginPermissionGrantStatus,
    ) {
        let manifest = inspect_plugin_control_manifest(plugin).expect("inspect plugin manifest");
        for request in &manifest.permission_requests {
            store
                .set_manifest_plugin_permission_grant(
                    &plugin.id,
                    &manifest,
                    &PluginControlScope::global(),
                    &request.permission,
                    &Value::Null,
                    status,
                )
                .expect("update manifest permission");
        }
    }

    #[test]
    fn narrower_revocation_wins_when_projecting_thread_permissions() {
        let workspace = PathBuf::from("C:/work/demo");
        let thread_id = Uuid::new_v4();
        let record = |scope, status| PluginPermissionGrantRecord {
            plugin_id: "plugin".to_string(),
            scope,
            permission: "network:api.example.com".to_string(),
            constraint: Value::Object(Map::new()),
            status,
            granted_at: None,
            updated_at: Utc::now(),
        };
        let records = vec![
            record(
                PluginControlScope::global(),
                PluginPermissionGrantStatus::Granted,
            ),
            record(
                PluginControlScope::thread(thread_id),
                PluginPermissionGrantStatus::Revoked,
            ),
        ];
        assert!(
            effective_granted_permissions(&records, Some(&workspace), Some(thread_id)).is_empty()
        );
    }

    #[test]
    fn thread_permission_projection_ignores_other_threads() {
        let workspace = PathBuf::from("C:/work/demo");
        let thread_id = Uuid::new_v4();
        let records = vec![PluginPermissionGrantRecord {
            plugin_id: "plugin".to_string(),
            scope: PluginControlScope::thread(Uuid::new_v4()),
            permission: "desktop:window:capture".to_string(),
            constraint: Value::Object(Map::new()),
            status: PluginPermissionGrantStatus::Granted,
            granted_at: Some(Utc::now()),
            updated_at: Utc::now(),
        }];
        assert!(
            effective_granted_permissions(&records, Some(&workspace), Some(thread_id)).is_empty()
        );
    }

    #[test]
    fn default_bundled_plugins_activate_browser_and_office_while_computer_stays_opt_in() {
        let workspace = TestWorkspace::new();
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let thread = store
            .create_thread(None, workspace.0.clone())
            .expect("create thread");
        let plugins = discover_plugins(Some(&workspace.0));
        let spreadsheet = plugins
            .iter()
            .find(|plugin| plugin.name == "spreadsheet" && plugin.source == PluginSource::Bundled)
            .expect("spreadsheet bundled plugin");
        let pdf = plugins
            .iter()
            .find(|plugin| plugin.name == "pdf" && plugin.source == PluginSource::Bundled)
            .expect("PDF bundled plugin");
        let documents = plugins
            .iter()
            .find(|plugin| plugin.name == "documents" && plugin.source == PluginSource::Bundled)
            .expect("Documents bundled plugin");
        let browser = plugins
            .iter()
            .find(|plugin| {
                plugin.name == "browser-automation" && plugin.source == PluginSource::Bundled
            })
            .expect("browser bundled plugin");
        let computer = plugins
            .iter()
            .find(|plugin| plugin.name == "computer-use" && plugin.source == PluginSource::Bundled)
            .expect("computer bundled plugin");

        assert!(spreadsheet.default_enabled);
        assert!(pdf.default_enabled);
        assert!(documents.default_enabled);
        assert!(browser.default_enabled);
        assert!(!computer.default_enabled);

        let outcome = load_plugin_outcome_for_thread(&store, &thread).expect("plugin load outcome");
        let snapshot = outcome.capability_snapshot();
        for plugin in [spreadsheet, pdf, documents, browser] {
            assert!(snapshot.unavailable.iter().any(|item| {
                item.contribution.contribution.plugin_id == plugin.id
                    && matches!(
                        item.reason,
                        CapabilityUnavailableReason::MissingPermissions(_)
                    )
            }));
            assert!(!snapshot
                .active
                .iter()
                .any(|item| item.contribution.plugin_id == plugin.id));
        }
        for plugin in [computer] {
            assert!(snapshot.unavailable.iter().any(|item| {
                item.contribution.contribution.plugin_id == plugin.id
                    && matches!(item.reason, CapabilityUnavailableReason::Disabled)
            }));
        }

        ensure_default_bundled_plugin_permissions(&store)
            .expect("bootstrap default bundled plugin permissions");
        let outcome =
            load_plugin_outcome_for_thread(&store, &thread).expect("authorized plugin outcome");
        let snapshot = outcome.capability_snapshot();
        let spreadsheet_kinds = snapshot
            .active
            .iter()
            .filter(|item| item.contribution.plugin_id == spreadsheet.id)
            .map(|item| item.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            spreadsheet_kinds,
            BTreeSet::from([ContributionKind::NativeTool, ContributionKind::Skill])
        );

        let pdf_kinds = snapshot
            .active
            .iter()
            .filter(|item| item.contribution.plugin_id == pdf.id)
            .map(|item| item.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(pdf_kinds, BTreeSet::from([ContributionKind::NativeTool]));
        let document_kinds = snapshot
            .active
            .iter()
            .filter(|item| item.contribution.plugin_id == documents.id)
            .map(|item| item.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            document_kinds,
            BTreeSet::from([ContributionKind::NativeTool])
        );
        let browser_kinds = snapshot
            .active
            .iter()
            .filter(|item| item.contribution.plugin_id == browser.id)
            .map(|item| item.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            browser_kinds,
            BTreeSet::from([ContributionKind::Skill, ContributionKind::NativeTool])
        );
        assert!(!snapshot
            .active
            .iter()
            .any(|item| item.contribution.plugin_id == computer.id));
    }

    #[test]
    fn default_permission_bootstrap_preserves_explicit_revocation() {
        let workspace = TestWorkspace::new();
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let thread = store
            .create_thread(None, workspace.0.clone())
            .expect("create thread");
        let spreadsheet = discover_plugins(Some(&workspace.0))
            .into_iter()
            .find(|plugin| plugin.name == "spreadsheet")
            .expect("spreadsheet bundled plugin");

        set_all_permissions(&store, &spreadsheet, PluginPermissionGrantStatus::Revoked);
        ensure_default_bundled_plugin_permissions(&store).expect("preserve spreadsheet revocation");

        let outcome =
            load_plugin_outcome_for_thread(&store, &thread).expect("revoked plugin outcome");
        let snapshot = outcome.capability_snapshot();
        assert!(!snapshot
            .active
            .iter()
            .any(|item| item.contribution.plugin_id == spreadsheet.id));
        assert!(snapshot.unavailable.iter().any(|item| {
            item.contribution.contribution.plugin_id == spreadsheet.id
                && matches!(
                    item.reason,
                    CapabilityUnavailableReason::MissingPermissions(_)
                )
        }));
    }

    #[test]
    fn permission_revocation_removes_app_preview_profile_and_scm_projections() {
        let workspace = TestWorkspace::new();
        let profile_id = format!("projection-reviewer-{}", Uuid::new_v4().simple());
        workspace.install_projection_plugin(&profile_id);
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let thread = store
            .create_thread(None, workspace.0.clone())
            .expect("create thread");
        let plugin = discover_plugins(Some(&workspace.0))
            .into_iter()
            .find(|plugin| plugin.name == "projection-pack")
            .expect("projection plugin");
        store
            .set_plugin_activation(&plugin.id, &PluginActivationScope::global(), true)
            .expect("activate plugin");
        set_all_permissions(&store, &plugin, PluginPermissionGrantStatus::Granted);

        let outcome =
            load_plugin_outcome_for_thread(&store, &thread).expect("project plugin outcome");
        let active = outcome.active_contributions().cloned().collect::<Vec<_>>();
        let projected_kinds = active
            .iter()
            .filter(|contribution| contribution.plugin_id == plugin.id)
            .map(|contribution| contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            projected_kinds,
            BTreeSet::from([
                ContributionKind::App,
                ContributionKind::Previewer,
                ContributionKind::AgentProfile,
                ContributionKind::ScmConnector,
            ])
        );
        let profiles = crate::load_agent_profiles_for_thread(&store, &thread)
            .expect("load active plugin profiles");
        assert_eq!(
            profiles
                .get(&profile_id)
                .and_then(|profile| profile.source_plugin_id.as_deref()),
            Some(plugin.id.as_str())
        );

        set_all_permissions(&store, &plugin, PluginPermissionGrantStatus::Revoked);

        let outcome =
            load_plugin_outcome_for_thread(&store, &thread).expect("project revoked outcome");
        let active = outcome.active_contributions().cloned().collect::<Vec<_>>();
        assert!(!active
            .iter()
            .any(|contribution| contribution.plugin_id == plugin.id));
        let profiles =
            crate::load_agent_profiles_for_thread(&store, &thread).expect("reload plugin profiles");
        assert!(profiles.get(&profile_id).is_none());
        let snapshot = outcome.capability_snapshot();
        let unavailable_kinds = snapshot
            .unavailable
            .iter()
            .filter(|item| item.contribution.contribution.plugin_id == plugin.id)
            .inspect(|item| {
                assert!(matches!(
                    item.reason,
                    CapabilityUnavailableReason::MissingPermissions(_)
                ));
            })
            .map(|item| item.contribution.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(unavailable_kinds, projected_kinds);
    }
}
