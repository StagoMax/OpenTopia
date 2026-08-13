use super::{ensure_thread, load_bound_agent_context, sync_plugin_mcp_configs, ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use opentopia_core::{
    discover_plugins, inspect_plugin_control_manifest, permission_requested,
    validate_plugin_settings, CapabilityActivationRequest, CapabilityActivationScope,
    CapabilityActivationSnapshot, CapabilityProjection, CapabilityRegistry, ExperienceMode,
    ExperienceSurfaceProfile, PluginActivation, PluginActivationRecord, PluginContribution,
    PluginContributionRecord, PluginControlManifest, PluginControlScope, PluginControlScopeType,
    PluginDescriptor, PluginPermission, PluginPermissionGrantRecord, PluginPermissionGrantStatus,
    PluginRuntimeHealthRecord, PluginSecretBindingRecord, PluginSettingsRecord, PluginSource,
    SessionStore, SqliteSessionStore, Thread,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path as FsPath, PathBuf};
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
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

async fn get_plugin_detail(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Query(query): Query<PluginContextQuery>,
) -> Result<Json<PluginDetailResponse>, ApiError> {
    let (plugin, manifest) = prepare_plugin(&state, &plugin_id, &query)?;
    let effective_enabled = state.store.plugin_effectively_enabled(
        &plugin.id,
        plugin.default_enabled,
        query.workspace_root.as_deref(),
        query.thread_id,
    )?;
    Ok(Json(PluginDetailResponse {
        plugin: plugin.clone(),
        manifest,
        activations: state.store.list_plugin_activations(&plugin.id)?,
        effective_enabled,
        contributions: state.store.list_plugin_contributions(&plugin.id)?,
        health: state.store.list_plugin_runtime_health(&plugin.id)?,
    }))
}

async fn put_plugin_activation(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    Json(request): Json<PluginActivationRequest>,
) -> Result<Json<PluginActivationResponse>, ApiError> {
    let scope = validate_scope(&state, &request.scope)?;
    let query = context_for_scope(&state, &scope)?;
    let (plugin, _) = prepare_plugin(&state, &plugin_id, &query)?;
    let servers = sync_plugin_mcp_configs(&state, &plugin).await?;
    let activation = state
        .store
        .set_plugin_activation(&plugin.id, &scope, request.enabled)?;
    if scope.scope_type == PluginControlScopeType::Thread {
        let thread_id = Uuid::parse_str(scope.scope_id.as_deref().unwrap_or_default())
            .map_err(|_| ApiError::bad_request("thread scopeId must be a UUID"))?;
        if !plugin.native_capabilities.is_empty() {
            state
                .store
                .set_thread_plugin_activation(thread_id, &plugin.name, request.enabled)?;
        }
        for server in servers {
            state
                .store
                .set_thread_mcp_server(thread_id, server.server_id, request.enabled)?;
        }
    }
    let effective_enabled = state.store.plugin_effectively_enabled(
        &plugin.id,
        plugin.default_enabled,
        query.workspace_root.as_deref(),
        query.thread_id,
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
) -> Result<Json<Vec<PluginContributionRecord>>, ApiError> {
    let (plugin, _) = prepare_plugin(&state, &plugin_id, &query)?;
    Ok(Json(state.store.list_plugin_contributions(&plugin.id)?))
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
    let mut plugins = Vec::new();
    for plugin in discover_plugins(Some(&thread.workspace_root)) {
        let manifest = inspect_plugin_control_manifest(&plugin)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        state
            .store
            .replace_plugin_contributions(&plugin.id, &manifest.contributions)?;
        let configured_enabled = state.store.plugin_effectively_enabled(
            &plugin.id,
            plugin.default_enabled,
            Some(&thread.workspace_root),
            Some(thread_id),
        )?;
        let enabled = configured_enabled
            && (effective_capabilities.allows_plugin(&plugin.id)
                || effective_capabilities.allows_plugin(&plugin.name));
        let grants = state.store.list_plugin_permission_grants(&plugin.id)?;
        let granted_permissions =
            effective_granted_permissions(&grants, &thread.workspace_root, thread_id);
        plugins.push(ThreadPluginCapabilities {
            plugin_id: plugin.id.clone(),
            plugin_name: plugin.name,
            enabled,
            contributions: enabled
                .then(|| state.store.list_plugin_contributions(&plugin.id))
                .transpose()?
                .unwrap_or_default(),
            granted_permissions,
        });
    }
    plugins.sort_by(|left, right| left.plugin_name.cmp(&right.plugin_name));
    let snapshot = capability_snapshot_for_thread(&state.store, &thread)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
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

pub(crate) fn capability_snapshot_for_thread(
    store: &SqliteSessionStore,
    thread: &Thread,
) -> anyhow::Result<CapabilityActivationSnapshot> {
    let mut registry = CapabilityRegistry::new();
    let mut activations = Vec::new();
    for plugin in discover_plugins(Some(&thread.workspace_root)) {
        let grants = store.list_plugin_permission_grants(&plugin.id)?;
        let granted_permissions =
            effective_granted_permissions(&grants, &thread.workspace_root, thread.id);
        let activation_records = store.list_plugin_activations(&plugin.id)?;
        activations.push(capability_activation(
            &plugin,
            &activation_records,
            &granted_permissions,
            &thread.workspace_root,
            thread.id,
        ));
        registry.register_plugin(plugin.capability_registration())?;
    }
    Ok(registry.activation_snapshot(CapabilityActivationRequest {
        scope: CapabilityActivationScope {
            workspace_id: Some(opentopia_core::normalize_workspace_key(
                &thread.workspace_root,
            )),
            thread_id: Some(thread.id.to_string()),
        },
        host_capabilities: host_capabilities(),
        plugins: activations,
    }))
}

pub(crate) fn active_contributions_for_thread(
    store: &SqliteSessionStore,
    thread: &Thread,
) -> anyhow::Result<Vec<PluginContribution>> {
    Ok(capability_snapshot_for_thread(store, thread)?
        .active
        .into_iter()
        .map(|active| active.contribution)
        .collect())
}

pub(crate) fn ensure_default_bundled_plugin_permissions(
    store: &SqliteSessionStore,
) -> anyhow::Result<()> {
    for plugin in discover_plugins(None)
        .into_iter()
        .filter(|plugin| plugin.source == PluginSource::Bundled && plugin.default_enabled)
    {
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

fn capability_activation(
    plugin: &PluginDescriptor,
    records: &[PluginActivationRecord],
    granted_permissions: &[String],
    workspace_root: &FsPath,
    thread_id: Uuid,
) -> PluginActivation {
    let workspace_id = opentopia_core::normalize_workspace_key(workspace_root);
    let thread_id = thread_id.to_string();
    let enabled_at = |scope_type, scope_id: Option<&str>| {
        records
            .iter()
            .find(|record| {
                record.scope.scope_type == scope_type
                    && record.scope.scope_id.as_deref() == scope_id
            })
            .map(|record| record.enabled)
    };
    let granted = granted_permissions.iter().collect::<BTreeSet<_>>();
    let permission_objects = plugin
        .capability_manifest
        .permissions
        .requirements()
        .into_iter()
        .filter(|permission| granted.contains(&permission_key(permission)))
        .collect();
    PluginActivation {
        plugin_id: plugin.id.clone(),
        global_enabled: enabled_at(PluginControlScopeType::Global, None),
        workspace_enabled: enabled_at(
            PluginControlScopeType::Workspace,
            Some(workspace_id.as_str()),
        ),
        thread_enabled: enabled_at(PluginControlScopeType::Thread, Some(thread_id.as_str())),
        granted_permissions: permission_objects,
    }
}

fn permission_key(permission: &PluginPermission) -> String {
    let category = match permission.kind {
        opentopia_core::PluginPermissionKind::Filesystem => "filesystem",
        opentopia_core::PluginPermissionKind::Network => "network",
        opentopia_core::PluginPermissionKind::Secret => "secrets",
        opentopia_core::PluginPermissionKind::Desktop => "desktop",
    };
    format!("{category}:{}", permission.value)
}

fn host_capabilities() -> Vec<String> {
    [
        "workspace.files.v1",
        "artifact.runtime.v1",
        "artifact.preview.v1",
        "nativeTool.pdf.v1",
        "nativeTool.document.v1",
        "nativeTool.spreadsheet.v1",
        "browser.runtime.v1",
        "policy.network.v1",
        "nativeTool.browser.v1",
        "computer.driver.v1",
        "policy.approval.v1",
        "nativeTool.computer.v1",
        "localGit.read.v1",
        "localGit.mutate.v1",
        "previewer.v1",
        "contextLoader.v1",
        "agentProfile.v1",
        "appView.v1",
        "scmConnector.v1",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn prepare_plugin(
    state: &AppState,
    plugin_id: &str,
    query: &PluginContextQuery,
) -> Result<(PluginDescriptor, PluginControlManifest), ApiError> {
    let workspace_root = resolve_context(state, query)?;
    let plugin = discover_plugins(workspace_root.as_deref())
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| ApiError::not_found("plugin is not available in this context"))?;
    let manifest = inspect_plugin_control_manifest(&plugin)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .store
        .replace_plugin_contributions(&plugin.id, &manifest.contributions)?;
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

fn effective_granted_permissions(
    records: &[PluginPermissionGrantRecord],
    workspace_root: &FsPath,
    thread_id: Uuid,
) -> Vec<String> {
    let workspace_id = opentopia_core::normalize_workspace_key(workspace_root);
    let thread_id = thread_id.to_string();
    let relevant = |record: &&PluginPermissionGrantRecord| match record.scope.scope_type {
        PluginControlScopeType::Global => true,
        PluginControlScopeType::Workspace => {
            record.scope.scope_id.as_deref() == Some(&workspace_id)
        }
        PluginControlScopeType::Thread => record.scope.scope_id.as_deref() == Some(&thread_id),
    };
    let permissions = records
        .iter()
        .filter(relevant)
        .map(|record| record.permission.clone())
        .collect::<BTreeSet<_>>();
    permissions
        .into_iter()
        .filter(|permission| {
            let matching = records
                .iter()
                .filter(relevant)
                .filter(|record| record.permission == *permission)
                .collect::<Vec<_>>();
            matching
                .iter()
                .any(|record| record.status == PluginPermissionGrantStatus::Granted)
                && !matching
                    .iter()
                    .any(|record| record.status == PluginPermissionGrantStatus::Revoked)
        })
        .collect()
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
    scope: PluginControlScope,
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginActivationResponse {
    activation: PluginActivationRecord,
    effective_enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginDetailResponse {
    plugin: PluginDescriptor,
    manifest: PluginControlManifest,
    activations: Vec<PluginActivationRecord>,
    effective_enabled: bool,
    contributions: Vec<PluginContributionRecord>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSettingsResponse {
    schema: Option<Value>,
    settings: PluginSettingsRecord,
    secret_bindings: Vec<PluginSecretBindingRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginPermissionsResponse {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadCapabilitiesResponse {
    thread_id: Uuid,
    experience_mode: ExperienceMode,
    prompt_profile_id: String,
    capability_projection: CapabilityProjection,
    workspace_root: PathBuf,
    generated_at: DateTime<Utc>,
    snapshot: CapabilityActivationSnapshot,
    plugins: Vec<ThreadPluginCapabilities>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadPluginCapabilities {
    plugin_id: String,
    plugin_name: String,
    enabled: bool,
    contributions: Vec<PluginContributionRecord>,
    granted_permissions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(effective_granted_permissions(&records, &workspace, thread_id).is_empty());
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
        assert!(effective_granted_permissions(&records, &workspace, thread_id).is_empty());
    }

    #[test]
    fn bundled_plugins_keep_privileged_surfaces_off_and_activate_authorized_office_tools() {
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
        assert!(!browser.default_enabled);
        assert!(!computer.default_enabled);

        let snapshot =
            capability_snapshot_for_thread(&store, &thread).expect("capability snapshot");
        for plugin in [spreadsheet, pdf, documents] {
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
        for plugin in [browser, computer] {
            assert!(snapshot.unavailable.iter().any(|item| {
                item.contribution.contribution.plugin_id == plugin.id
                    && matches!(item.reason, CapabilityUnavailableReason::Disabled)
            }));
        }

        ensure_default_bundled_plugin_permissions(&store)
            .expect("bootstrap default bundled plugin permissions");
        let snapshot = capability_snapshot_for_thread(&store, &thread)
            .expect("authorized capability snapshot");
        let spreadsheet_kinds = snapshot
            .active
            .iter()
            .filter(|item| item.contribution.plugin_id == spreadsheet.id)
            .map(|item| item.contribution.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            spreadsheet_kinds,
            BTreeSet::from([ContributionKind::NativeTool])
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

        let snapshot =
            capability_snapshot_for_thread(&store, &thread).expect("revoked capability snapshot");
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
            .set_plugin_activation(&plugin.id, &PluginControlScope::global(), true)
            .expect("activate plugin");
        set_all_permissions(&store, &plugin, PluginPermissionGrantStatus::Granted);

        let active =
            active_contributions_for_thread(&store, &thread).expect("project active contributions");
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

        let active = active_contributions_for_thread(&store, &thread)
            .expect("project revoked contributions");
        assert!(!active
            .iter()
            .any(|contribution| contribution.plugin_id == plugin.id));
        let profiles =
            crate::load_agent_profiles_for_thread(&store, &thread).expect("reload plugin profiles");
        assert!(profiles.get(&profile_id).is_none());
        let snapshot =
            capability_snapshot_for_thread(&store, &thread).expect("revoked capability snapshot");
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
