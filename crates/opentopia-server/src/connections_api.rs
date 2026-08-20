use crate::flows_api::ensure_enterprise;
use crate::{ApiError, AppState};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use opentopia_core::{
    CapabilityDiscoveryKindV1, ConnectionAccountV1, ConnectionAuthContextV1,
    ConnectionAuthVerificationV1, ConnectionCapabilityRevisionV1, ConnectionOwnerTypeV1,
    ConnectionRuntimeBindingV1, ConnectionStatusV1, ConnectionStoreError, ConnectionV1,
    IntegrationAuthSchemeV1, IntegrationDefinitionV1, IntegrationKindV1, McpLifecycleStatus,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/integration-definitions",
            get(list_integration_definitions).post(create_integration_definition),
        )
        .route(
            "/api/integration-definitions/:definition_id",
            get(get_integration_definition).patch(update_integration_definition),
        )
        .route(
            "/api/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/api/connections/:connection_id",
            get(get_connection).patch(update_connection),
        )
        .route(
            "/api/connections/:connection_id/test",
            post(test_connection),
        )
        .route(
            "/api/connections/:connection_id/capabilities/refresh",
            post(refresh_connection_capabilities),
        )
        .route(
            "/api/connections/:connection_id/capability-revisions",
            get(list_connection_capability_revisions),
        )
        .route(
            "/api/connections/:connection_id/capability-revisions/:revision",
            get(get_connection_capability_revision),
        )
}

async fn list_integration_definitions(
    State(state): State<AppState>,
) -> Result<Json<Vec<IntegrationDefinitionV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(state.store.list_integration_definitions()?))
}

async fn get_integration_definition(
    State(state): State<AppState>,
    Path(definition_id): Path<Uuid>,
) -> Result<Json<IntegrationDefinitionV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(load_integration(&state, definition_id)?))
}

async fn create_integration_definition(
    State(state): State<AppState>,
    Json(request): Json<CreateIntegrationDefinitionRequest>,
) -> Result<Json<IntegrationDefinitionV1>, ApiError> {
    ensure_enterprise(&state)?;
    let key = validated_key(&request.key)?;
    let name = required_text(&request.name, "Integration name")?;
    validate_discovery(request.kind, request.capability_discovery)?;
    let mut definition = IntegrationDefinitionV1::new(
        key,
        name,
        request.kind,
        request.auth_scheme,
        request.capability_discovery,
    );
    definition.description = optional_text(request.description);
    definition.enabled = request.enabled.unwrap_or(true);
    Ok(Json(
        state.store.insert_integration_definition(&definition)?,
    ))
}

async fn update_integration_definition(
    State(state): State<AppState>,
    Path(definition_id): Path<Uuid>,
    Json(request): Json<UpdateIntegrationDefinitionRequest>,
) -> Result<Json<IntegrationDefinitionV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut definition = load_integration(&state, definition_id)?;
    if let Some(name) = request.name {
        definition.name = required_text(&name, "Integration name")?;
    }
    if request.clear_description.unwrap_or(false) {
        definition.description = None;
    } else if let Some(description) = request.description {
        definition.description = optional_text(Some(description));
    }
    if let Some(enabled) = request.enabled {
        definition.enabled = enabled;
    }
    definition.touch();
    state
        .store
        .update_integration_definition(&definition, request.expected_revision)
        .map(Json)
        .map_err(connection_error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionListQuery {
    integration_definition_id: Option<Uuid>,
    status: Option<ConnectionStatusV1>,
}

async fn list_connections(
    State(state): State<AppState>,
    Query(query): Query<ConnectionListQuery>,
) -> Result<Json<Vec<ConnectionV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(state.store.list_connections(
        query.integration_definition_id,
        query.status,
    )?))
}

async fn get_connection(
    State(state): State<AppState>,
    Path(connection_id): Path<Uuid>,
) -> Result<Json<ConnectionV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(load_connection(&state, connection_id)?))
}

async fn create_connection(
    State(state): State<AppState>,
    Json(request): Json<CreateConnectionRequest>,
) -> Result<Json<ConnectionV1>, ApiError> {
    ensure_enterprise(&state)?;
    let definition = load_integration(&state, request.integration_definition_id)?;
    validate_connection_definition(&definition)?;
    let runtime_binding = request.runtime_binding.into_domain();
    validate_runtime_binding(&state, &runtime_binding)?;
    let auth_context = request.auth_context.into_domain(definition.auth_scheme)?;
    let mut connection = ConnectionV1::new(
        definition.id,
        required_text(&request.name, "Connection name")?,
        request.owner_type,
        required_text(&request.environment, "Connection environment")?,
        runtime_binding,
        auth_context,
    );
    connection.enabled = request.enabled.unwrap_or(true);
    if !connection.enabled {
        connection.status = ConnectionStatusV1::Disabled;
    }
    state
        .store
        .insert_connection(&connection)
        .map(Json)
        .map_err(connection_error)
}

async fn update_connection(
    State(state): State<AppState>,
    Path(connection_id): Path<Uuid>,
    Json(request): Json<UpdateConnectionRequest>,
) -> Result<Json<ConnectionV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut connection = load_connection(&state, connection_id)?;
    let definition = load_integration(&state, connection.integration_definition_id)?;
    if let Some(name) = request.name {
        connection.name = required_text(&name, "Connection name")?;
    }
    if let Some(owner_type) = request.owner_type {
        connection.owner_type = owner_type;
    }
    if let Some(environment) = request.environment {
        connection.environment = required_text(&environment, "Connection environment")?;
    }
    if let Some(runtime_binding) = request.runtime_binding.map(|binding| binding.into_domain()) {
        validate_runtime_binding(&state, &runtime_binding)?;
        connection.runtime_binding = runtime_binding;
    }
    if let Some(auth_context) = request.auth_context {
        connection.auth_context = auth_context.into_domain(definition.auth_scheme)?;
        if connection.enabled {
            connection.status = ConnectionStatusV1::Configured;
        }
    }
    if let Some(enabled) = request.enabled {
        connection.enabled = enabled;
        connection.status = if enabled {
            ConnectionStatusV1::Configured
        } else {
            ConnectionStatusV1::Disabled
        };
    }
    connection.touch();
    state
        .store
        .update_connection(&connection, request.expected_revision)
        .map(Json)
        .map_err(connection_error)
}

async fn test_connection(
    State(state): State<AppState>,
    Path(connection_id): Path<Uuid>,
) -> Result<Json<TestConnectionResponse>, ApiError> {
    ensure_enterprise(&state)?;
    let mut connection = load_connection(&state, connection_id)?;
    let definition = load_integration(&state, connection.integration_definition_id)?;
    let checked_at = Utc::now();

    if !definition.enabled || !connection.enabled {
        let message = if !definition.enabled {
            "Integration definition is disabled."
        } else {
            "Connection is disabled."
        };
        return Ok(Json(TestConnectionResponse {
            health: ConnectionHealthView {
                ok: false,
                runtime_status: McpLifecycleStatus::Disabled,
                auth_status: connection.auth_context.verification,
                message: message.to_string(),
                checked_at,
                tools_count: 0,
            },
            connection,
        }));
    }

    let server = load_connection_mcp_server(&state, &connection)?;
    let runtime = state.mcp_host.ensure_server(server).await;
    let (ok, runtime_status, message, tools_count) = match runtime {
        Ok(status) => (
            matches!(status.status, McpLifecycleStatus::Ready),
            status.status,
            status.message,
            status.tools_count,
        ),
        Err(error) => (false, McpLifecycleStatus::Error, error.to_string(), 0),
    };
    let expected_revision = connection.revision;
    connection.touch();
    connection.status = if ok {
        ConnectionStatusV1::Ready
    } else {
        ConnectionStatusV1::Degraded
    };
    connection.last_tested_at = Some(checked_at);
    connection.last_error = (!ok).then(|| message.clone());
    let connection = state
        .store
        .update_connection(&connection, expected_revision)
        .map_err(connection_error)?;
    Ok(Json(TestConnectionResponse {
        health: ConnectionHealthView {
            ok,
            runtime_status,
            auth_status: connection.auth_context.verification,
            message,
            checked_at,
            tools_count,
        },
        connection,
    }))
}

async fn refresh_connection_capabilities(
    State(state): State<AppState>,
    Path(connection_id): Path<Uuid>,
) -> Result<Json<RefreshConnectionCapabilitiesResponse>, ApiError> {
    ensure_enterprise(&state)?;
    let mut connection = load_connection(&state, connection_id)?;
    let definition = load_integration(&state, connection.integration_definition_id)?;
    validate_connection_definition(&definition)?;
    if !definition.enabled || !connection.enabled {
        return Err(ApiError::bad_request(
            "Connection and Integration definition must be enabled before capability discovery",
        ));
    }
    let server = load_connection_mcp_server(&state, &connection)?;
    state
        .mcp_host
        // Explicit refresh must perform a real tools/list even when the MCP
        // provider did not emit tools/list_changed. Restart is the safe v1
        // invalidation boundary until the host exposes force_refresh_tools.
        .restart_server(server)
        .await
        .map_err(|error| ApiError::bad_gateway(error.to_string()))?;
    let server_id = connection.runtime_binding.mcp_server_id();
    let tools = state
        .mcp_host
        .list_tools(server_id)
        .await
        .map_err(|error| ApiError::bad_gateway(error.to_string()))?;

    let revisions = state
        .store
        .list_connection_capability_revisions(connection.id)?;
    let previous = revisions.first().cloned();
    let next_revision = previous.as_ref().map_or(1, |item| item.revision + 1);
    let candidate =
        ConnectionCapabilityRevisionV1::from_mcp_tools(connection.id, next_revision, &tools)?;
    let diff = capability_diff(previous.as_ref(), &candidate);
    let expected_revision = connection.revision;
    connection.touch();
    connection.status = ConnectionStatusV1::Ready;
    connection.last_tested_at = Some(Utc::now());
    connection.last_error = None;

    if let Some(previous) = previous.filter(|item| item.content_hash == candidate.content_hash) {
        connection.active_capability_revision = Some(previous.revision);
        let connection = state
            .store
            .update_connection(&connection, expected_revision)
            .map_err(connection_error)?;
        return Ok(Json(RefreshConnectionCapabilitiesResponse {
            connection,
            capability_revision: previous,
            changed: false,
            diff,
        }));
    }

    connection.active_capability_revision = Some(candidate.revision);
    let (connection, capability_revision) = state
        .store
        .publish_connection_capability_revision(&connection, expected_revision, &candidate)
        .map_err(connection_error)?;
    Ok(Json(RefreshConnectionCapabilitiesResponse {
        connection,
        capability_revision,
        changed: true,
        diff,
    }))
}

async fn list_connection_capability_revisions(
    State(state): State<AppState>,
    Path(connection_id): Path<Uuid>,
) -> Result<Json<Vec<ConnectionCapabilityRevisionV1>>, ApiError> {
    ensure_enterprise(&state)?;
    load_connection(&state, connection_id)?;
    Ok(Json(
        state
            .store
            .list_connection_capability_revisions(connection_id)?,
    ))
}

async fn get_connection_capability_revision(
    State(state): State<AppState>,
    Path((connection_id, revision)): Path<(Uuid, u32)>,
) -> Result<Json<ConnectionCapabilityRevisionV1>, ApiError> {
    ensure_enterprise(&state)?;
    load_connection(&state, connection_id)?;
    let revision = state
        .store
        .get_connection_capability_revision(connection_id, revision)?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Connection capability revision not found: {connection_id}@{revision}"
            ))
        })?;
    Ok(Json(revision))
}

fn load_integration(
    state: &AppState,
    definition_id: Uuid,
) -> Result<IntegrationDefinitionV1, ApiError> {
    state
        .store
        .get_integration_definition(definition_id)?
        .ok_or_else(|| {
            ApiError::not_found(format!("Integration definition not found: {definition_id}"))
        })
}

fn load_connection(state: &AppState, connection_id: Uuid) -> Result<ConnectionV1, ApiError> {
    state
        .store
        .get_connection(connection_id)?
        .ok_or_else(|| ApiError::not_found(format!("Connection not found: {connection_id}")))
}

fn load_connection_mcp_server(
    state: &AppState,
    connection: &ConnectionV1,
) -> Result<opentopia_core::McpServerConfig, ApiError> {
    let server_id = connection.runtime_binding.mcp_server_id();
    state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))
}

fn validate_connection_definition(definition: &IntegrationDefinitionV1) -> Result<(), ApiError> {
    if definition.kind != IntegrationKindV1::Mcp
        || definition.capability_discovery != CapabilityDiscoveryKindV1::McpToolsList
    {
        return Err(ApiError::bad_request(
            "This release only supports MCP connections discovered through tools/list",
        ));
    }
    Ok(())
}

fn validate_runtime_binding(
    state: &AppState,
    binding: &ConnectionRuntimeBindingV1,
) -> Result<(), ApiError> {
    let server_id = binding.mcp_server_id();
    if state.store.get_mcp_server(server_id)?.is_none() {
        return Err(ApiError::not_found(format!(
            "MCP server not found: {server_id}; create a dedicated MCP server runtime first"
        )));
    }
    Ok(())
}

fn validate_discovery(
    kind: IntegrationKindV1,
    discovery: CapabilityDiscoveryKindV1,
) -> Result<(), ApiError> {
    if (kind == IntegrationKindV1::Mcp) != (discovery == CapabilityDiscoveryKindV1::McpToolsList) {
        return Err(ApiError::bad_request(
            "mcp_tools_list discovery is only valid for MCP Integration definitions",
        ));
    }
    Ok(())
}

fn validated_key(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 96
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
    {
        return Err(ApiError::bad_request(
            "Integration key must be 1-96 ASCII letters, digits, dots, dashes, or underscores",
        ));
    }
    Ok(value)
}

fn required_text(value: &str, field: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{field} cannot be empty")));
    }
    if value.len() > 256 {
        return Err(ApiError::bad_request(format!(
            "{field} cannot exceed 256 bytes"
        )));
    }
    Ok(value.to_string())
}

fn optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn validated_credential_ref(value: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    let allowed = ["vault://", "keychain://", "env://", "plugin://"];
    if value.len() > 512
        || value.contains(char::is_whitespace)
        || value.contains(['?', '#'])
        || !allowed.iter().any(|prefix| {
            value
                .strip_prefix(prefix)
                .is_some_and(|reference| !reference.is_empty())
        })
    {
        return Err(ApiError::bad_request(
            "credentialRef must be an opaque vault://, keychain://, env://, or plugin:// reference",
        ));
    }
    Ok(Some(value.to_string()))
}

fn capability_diff(
    previous: Option<&ConnectionCapabilityRevisionV1>,
    current: &ConnectionCapabilityRevisionV1,
) -> ConnectionCapabilityDiffView {
    let previous_by_id = previous
        .map(|revision| {
            revision
                .capabilities
                .iter()
                .map(|capability| (capability.capability_id.as_str(), capability))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let current_by_id = current
        .capabilities
        .iter()
        .map(|capability| (capability.capability_id.as_str(), capability))
        .collect::<BTreeMap<_, _>>();
    let previous_ids = previous_by_id.keys().copied().collect::<BTreeSet<_>>();
    let current_ids = current_by_id.keys().copied().collect::<BTreeSet<_>>();
    let added_capability_ids = current_ids
        .difference(&previous_ids)
        .map(|id| (*id).to_string())
        .collect();
    let removed_capability_ids = previous_ids
        .difference(&current_ids)
        .map(|id| (*id).to_string())
        .collect();
    let changed_capability_ids = previous_ids
        .intersection(&current_ids)
        .filter(|id| previous_by_id.get(**id) != current_by_id.get(**id))
        .map(|id| (*id).to_string())
        .collect();
    ConnectionCapabilityDiffView {
        added_capability_ids,
        removed_capability_ids,
        changed_capability_ids,
    }
}

pub(crate) fn connection_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<ConnectionStoreError>() {
        return match error {
            ConnectionStoreError::IntegrationDefinitionNotFound(_)
            | ConnectionStoreError::ConnectionNotFound(_)
            | ConnectionStoreError::CapabilityRevisionNotFound { .. } => {
                ApiError::not_found(error.to_string())
            }
            ConnectionStoreError::IntegrationDefinitionRevisionConflict(_)
            | ConnectionStoreError::DuplicateIntegrationKey(_)
            | ConnectionStoreError::ConnectionRevisionConflict(_)
            | ConnectionStoreError::McpRuntimeAlreadyBound(_) => {
                ApiError::conflict(error.to_string())
            }
        };
    }
    ApiError::from(error)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateIntegrationDefinitionRequest {
    key: String,
    name: String,
    description: Option<String>,
    kind: IntegrationKindV1,
    auth_scheme: IntegrationAuthSchemeV1,
    capability_discovery: CapabilityDiscoveryKindV1,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateIntegrationDefinitionRequest {
    expected_revision: u32,
    name: Option<String>,
    description: Option<String>,
    clear_description: Option<bool>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateConnectionRequest {
    integration_definition_id: Uuid,
    name: String,
    owner_type: ConnectionOwnerTypeV1,
    environment: String,
    enabled: Option<bool>,
    runtime_binding: ConnectionRuntimeBindingRequest,
    #[serde(default)]
    auth_context: ConnectionAuthContextRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateConnectionRequest {
    expected_revision: u32,
    name: Option<String>,
    owner_type: Option<ConnectionOwnerTypeV1>,
    environment: Option<String>,
    enabled: Option<bool>,
    runtime_binding: Option<ConnectionRuntimeBindingRequest>,
    auth_context: Option<ConnectionAuthContextRequest>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectionAuthContextRequest {
    credential_ref: Option<String>,
    #[serde(default)]
    account: ConnectionAccountRequest,
    #[serde(default)]
    granted_scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl ConnectionAuthContextRequest {
    fn into_domain(
        self,
        auth_scheme: IntegrationAuthSchemeV1,
    ) -> Result<ConnectionAuthContextV1, ApiError> {
        let credential_ref = validated_credential_ref(self.credential_ref)?;
        if auth_scheme == IntegrationAuthSchemeV1::None && credential_ref.is_some() {
            return Err(ApiError::bad_request(
                "A no-auth Integration cannot store a credentialRef",
            ));
        }
        let mut scopes = self
            .granted_scopes
            .into_iter()
            .map(|scope| scope.trim().to_string())
            .filter(|scope| !scope.is_empty())
            .collect::<Vec<_>>();
        scopes.sort();
        scopes.dedup();
        Ok(ConnectionAuthContextV1 {
            credential_ref,
            verification: if auth_scheme == IntegrationAuthSchemeV1::None {
                ConnectionAuthVerificationV1::NotRequired
            } else {
                ConnectionAuthVerificationV1::Unverified
            },
            account: self.account.into_domain(),
            granted_scopes: scopes,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConnectionRuntimeBindingRequest {
    McpServer {
        #[serde(rename = "serverId")]
        server_id: Uuid,
    },
}

impl ConnectionRuntimeBindingRequest {
    fn into_domain(self) -> ConnectionRuntimeBindingV1 {
        match self {
            Self::McpServer { server_id } => ConnectionRuntimeBindingV1::McpServer { server_id },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectionAccountRequest {
    display_name: Option<String>,
    external_account_id: Option<String>,
    tenant_id: Option<String>,
    tenant_name: Option<String>,
    workspace_id: Option<String>,
    workspace_name: Option<String>,
}

impl ConnectionAccountRequest {
    fn into_domain(self) -> ConnectionAccountV1 {
        ConnectionAccountV1 {
            display_name: optional_text(self.display_name),
            external_account_id: optional_text(self.external_account_id),
            tenant_id: optional_text(self.tenant_id),
            tenant_name: optional_text(self.tenant_name),
            workspace_id: optional_text(self.workspace_id),
            workspace_name: optional_text(self.workspace_name),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionHealthView {
    pub(crate) ok: bool,
    pub(crate) runtime_status: McpLifecycleStatus,
    pub(crate) auth_status: ConnectionAuthVerificationV1,
    pub(crate) message: String,
    pub(crate) checked_at: DateTime<Utc>,
    pub(crate) tools_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestConnectionResponse {
    pub(crate) connection: ConnectionV1,
    pub(crate) health: ConnectionHealthView,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionCapabilityDiffView {
    pub(crate) added_capability_ids: Vec<String>,
    pub(crate) removed_capability_ids: Vec<String>,
    pub(crate) changed_capability_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefreshConnectionCapabilitiesResponse {
    pub(crate) connection: ConnectionV1,
    pub(crate) capability_revision: ConnectionCapabilityRevisionV1,
    pub(crate) changed: bool,
    pub(crate) diff: ConnectionCapabilityDiffView,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_input_accepts_references_and_rejects_secret_values() {
        assert_eq!(
            validated_credential_ref(Some("vault://connections/crm".to_string()))
                .expect("valid reference")
                .as_deref(),
            Some("vault://connections/crm")
        );
        assert!(validated_credential_ref(Some("raw-secret-token".to_string())).is_err());
        assert!(validated_credential_ref(Some("vault://item?token=secret".to_string())).is_err());
    }

    #[test]
    fn discovery_contract_rejects_non_mcp_tools_list() {
        assert!(validate_discovery(
            IntegrationKindV1::Database,
            CapabilityDiscoveryKindV1::McpToolsList
        )
        .is_err());
        assert!(validate_discovery(
            IntegrationKindV1::Mcp,
            CapabilityDiscoveryKindV1::McpToolsList
        )
        .is_ok());
    }

    #[test]
    fn connection_requests_reject_unknown_secret_bearing_fields() {
        let definition_id = Uuid::new_v4();
        let server_id = Uuid::new_v4();
        let top_level = serde_json::json!({
            "integrationDefinitionId": definition_id,
            "name": "CRM",
            "ownerType": "personal",
            "environment": "production",
            "runtimeBinding": {"kind": "mcp_server", "serverId": server_id},
            "token": "must-not-be-accepted"
        });
        assert!(serde_json::from_value::<CreateConnectionRequest>(top_level).is_err());

        let nested_auth = serde_json::json!({
            "integrationDefinitionId": definition_id,
            "name": "CRM",
            "ownerType": "personal",
            "environment": "production",
            "runtimeBinding": {"kind": "mcp_server", "serverId": server_id},
            "authContext": {"password": "must-not-be-accepted"}
        });
        assert!(serde_json::from_value::<CreateConnectionRequest>(nested_auth).is_err());

        let nested_runtime = serde_json::json!({
            "integrationDefinitionId": definition_id,
            "name": "CRM",
            "ownerType": "personal",
            "environment": "production",
            "runtimeBinding": {
                "kind": "mcp_server",
                "serverId": server_id,
                "apiKey": "must-not-be-accepted"
            }
        });
        assert!(serde_json::from_value::<CreateConnectionRequest>(nested_runtime).is_err());
    }
}
