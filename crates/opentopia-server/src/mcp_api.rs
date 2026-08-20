use crate::connection_operation_runtime::StoreConnectionOperationInvocationGate;
use crate::{
    current_settings, ensure_thread, load_bound_agent_context, publish_payload, AgentEventPayload,
    ApiError, AppState, BasicPolicyEngine, DeleteResponse, McpCallResult, McpExtensionHost,
    McpServerConfig, McpServerStatus, McpToolDescriptor, ModelContentPart, PolicyDecision,
    PolicyEngine, SessionStore, ThreadMcpServer, ToolCall, ToolPermissionDescriptor, ToolResult,
};
use axum::extract::{Path, State};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use opentopia_core::{
    CapabilityDiscoveryKindV1, ConnectionAccountV1, ConnectionAuthContextV1,
    ConnectionAuthVerificationV1, ConnectionOperationInvocationGate, ConnectionOwnerTypeV1,
    ConnectionRuntimeBindingV1, ConnectionStatusV1, ConnectionV1, ExecutionConnectionOperationV1,
    ExperienceMode, IntegrationAuthSchemeV1, IntegrationDefinitionV1, IntegrationKindV1,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/mcp/servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id",
            patch(update_mcp_server).delete(delete_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id/restart",
            post(restart_mcp_server),
        )
        .route("/api/mcp/servers/:server_id/tools", get(list_mcp_tools))
        .route("/api/mcp/servers/:server_id/call-tool", post(call_mcp_tool))
        .route("/api/threads/:thread_id/mcp", get(list_thread_mcp_servers))
        .route(
            "/api/threads/:thread_id/mcp/:server_id",
            put(set_thread_mcp_server),
        )
}

pub(super) async fn ensure_mcp_server_status(
    host: &McpExtensionHost,
    server: &McpServerConfig,
) -> McpServerStatus {
    match host.ensure_server(server.clone()).await {
        Ok(status) => status,
        Err(err) => {
            warn!(?err, server_id = %server.server_id, "failed to apply MCP server configuration");
            host.status_for_config(server).await
        }
    }
}

async fn list_mcp_servers(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerView>>, ApiError> {
    let servers = state.store.list_mcp_servers()?;
    let mut views = Vec::with_capacity(servers.len());
    for server in servers {
        let status = state.mcp_host.status_for_config(&server).await;
        views.push(McpServerView { server, status });
    }
    Ok(Json(views))
}

async fn create_mcp_server(
    State(state): State<AppState>,
    Json(request): Json<McpServerRequest>,
) -> Result<Json<McpServerView>, ApiError> {
    let server = request.into_config()?;
    let server = state.store.insert_mcp_server(server)?;
    if let Err(error) = sync_legacy_mcp_connection(&state, &server) {
        let _ = state.store.delete_mcp_server(server.server_id);
        let _ = state.store.delete_integration_definition(server.server_id);
        return Err(error);
    }
    let status = ensure_mcp_server_status(&state.mcp_host, &server).await;
    Ok(Json(McpServerView { status, server }))
}

async fn update_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(request): Json<McpServerPatchRequest>,
) -> Result<Json<McpServerView>, ApiError> {
    let mut server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    request.apply(&mut server)?;
    server.refresh_updated_at();
    let server = state
        .store
        .update_mcp_server(server)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    sync_legacy_mcp_connection(&state, &server)?;
    let status = ensure_mcp_server_status(&state.mcp_host, &server).await;
    Ok(Json(McpServerView { status, server }))
}

/// Projects the legacy MCP CRUD path into the account-level Connections control plane.
/// The legacy response contract stays unchanged while both APIs observe the same runtime.
fn sync_legacy_mcp_connection(state: &AppState, server: &McpServerConfig) -> Result<(), ApiError> {
    let existing_connection = state.store.get_connection(server.server_id)?;
    let has_credential_ref = existing_connection
        .as_ref()
        .and_then(|connection| connection.auth_context.credential_ref.as_ref())
        .is_some();
    let verification = if server.env_keys.is_empty() {
        ConnectionAuthVerificationV1::NotRequired
    } else {
        ConnectionAuthVerificationV1::LegacyUnverified
    };
    let auth_scheme = legacy_mcp_auth_scheme(&server.env_keys, has_credential_ref);

    let definition = match state.store.get_integration_definition(server.server_id)? {
        Some(mut definition) => {
            let expected_revision = definition.revision;
            definition.name = server.name.clone();
            definition.auth_scheme = auth_scheme;
            definition.enabled = server.enabled;
            definition.touch();
            state
                .store
                .update_integration_definition(&definition, expected_revision)
                .map_err(crate::connections_api::connection_error)?
        }
        None => {
            let mut definition = IntegrationDefinitionV1::new(
                format!("legacy-mcp-{}", server.server_id),
                server.name.clone(),
                IntegrationKindV1::Mcp,
                auth_scheme,
                CapabilityDiscoveryKindV1::McpToolsList,
            );
            definition.id = server.server_id;
            definition.description = Some("Managed by the legacy MCP server API.".to_string());
            definition.enabled = server.enabled;
            definition.created_at = server.created_at;
            definition.updated_at = server.updated_at;
            state
                .store
                .insert_integration_definition(&definition)
                .map_err(crate::connections_api::connection_error)?
        }
    };

    match existing_connection {
        Some(mut connection) => {
            let expected_revision = connection.revision;
            connection.enabled = server.enabled;
            connection.status = if server.enabled {
                ConnectionStatusV1::Configured
            } else {
                ConnectionStatusV1::Disabled
            };
            if connection.auth_context.credential_ref.is_none() {
                connection.auth_context.verification = verification;
            }
            connection.touch();
            state
                .store
                .update_connection(&connection, expected_revision)
                .map_err(crate::connections_api::connection_error)?;
        }
        None => {
            let mut connection = ConnectionV1::new(
                definition.id,
                server.name.clone(),
                ConnectionOwnerTypeV1::Personal,
                "local".to_string(),
                ConnectionRuntimeBindingV1::McpServer {
                    server_id: server.server_id,
                },
                ConnectionAuthContextV1 {
                    credential_ref: None,
                    verification,
                    account: ConnectionAccountV1 {
                        display_name: Some(server.name.clone()),
                        ..ConnectionAccountV1::default()
                    },
                    granted_scopes: Vec::new(),
                    expires_at: None,
                },
            );
            connection.id = server.server_id;
            connection.enabled = server.enabled;
            connection.status = if server.enabled {
                ConnectionStatusV1::Configured
            } else {
                ConnectionStatusV1::Disabled
            };
            connection.created_at = server.created_at;
            connection.updated_at = server.updated_at;
            state
                .store
                .insert_connection(&connection)
                .map_err(crate::connections_api::connection_error)?;
        }
    }
    Ok(())
}

fn legacy_mcp_auth_scheme(
    env_keys: &[String],
    has_credential_ref: bool,
) -> IntegrationAuthSchemeV1 {
    if env_keys.is_empty() && !has_credential_ref {
        IntegrationAuthSchemeV1::None
    } else {
        IntegrationAuthSchemeV1::External
    }
}

async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    state.mcp_host.forget_server(server_id).await.ok();
    let deleted = state.store.delete_mcp_server(server_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "MCP server not found: {server_id}"
        )));
    }
    Ok(Json(DeleteResponse { deleted }))
}

async fn restart_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<McpServerStatus>, ApiError> {
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    let status = state.mcp_host.restart_server(server).await?;
    Ok(Json(status))
}

async fn list_mcp_tools(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<Vec<McpToolDescriptor>>, ApiError> {
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    state.mcp_host.ensure_server(server).await?;
    Ok(Json(state.mcp_host.list_tools(server_id).await?))
}

async fn call_mcp_tool(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(request): Json<McpToolCallRequest>,
) -> Result<Json<McpCallResult>, ApiError> {
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    let thread_id = request.thread_id;
    let thread = state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    let (bound_instance, bound_template) = load_bound_agent_context(&state, &thread)?;
    let structured_operation = match (bound_instance.as_ref(), bound_template.as_ref()) {
        (Some(instance), Some(template)) if !template.spec.connection_bindings.is_empty() => {
            let mut matches = instance
                .execution_context
                .connection_operations
                .iter()
                .filter(|operation| {
                    operation.mcp_server_id == server_id
                        && operation.provider_tool_name == request.tool_name
                });
            let operation = matches.next().cloned().ok_or_else(|| {
                ApiError::forbidden("MCP operation is outside the bound Agent authority")
            })?;
            if matches.next().is_some() {
                return Err(ApiError::conflict(
                    "bound Agent contains an ambiguous Connection operation route",
                ));
            }
            Some(operation)
        }
        (Some(instance), Some(_)) => {
            if !instance
                .execution_context
                .capabilities
                .allows_mcp_server(&server_id.to_string())
            {
                return Err(ApiError::forbidden(
                    "MCP server is outside the bound Agent authority",
                ));
            }
            None
        }
        (None, None) if thread.experience_mode == ExperienceMode::Flow => {
            return Err(ApiError::forbidden(
                "unbound Flow threads cannot invoke MCP tools",
            ));
        }
        (None, None) => None,
        _ => {
            return Err(ApiError::conflict(
                "bound Agent execution context is incomplete",
            ));
        }
    };
    let enabled_for_thread = if structured_operation.is_some() {
        true
    } else {
        state
            .store
            .list_thread_mcp_servers(thread_id)?
            .into_iter()
            .any(|binding| binding.server_id == server_id && binding.enabled)
    };
    if !server.enabled || !enabled_for_thread {
        return Err(ApiError::bad_request(
            "MCP server is not enabled for this thread",
        ));
    }
    if let Some(operation) = structured_operation.as_ref() {
        // Reject revoked/stale structured authority before starting the MCP
        // runtime or discovering its catalog. The same gate runs again just
        // before tools/call to cover changes during approval or policy review.
        StoreConnectionOperationInvocationGate::new(state.store.clone())
            .authorize(operation)
            .await
            .map_err(|_| ApiError::forbidden("Connection operation is no longer authorized"))?;
    }
    state.mcp_host.ensure_server(server.clone()).await?;

    let tools = state.mcp_host.list_tools(server_id).await?;
    let mut descriptor = tools
        .iter()
        .find(|t| t.tool_name == request.tool_name)
        .cloned()
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "tool not found on server {}: {}",
                server_id, request.tool_name
            ))
        })?;
    if let Some(operation) = structured_operation.as_ref() {
        descriptor.public_name = operation.model_tool_name.clone();
    }

    let settings = current_settings(&state);
    let policy = Arc::new(BasicPolicyEngine::new(
        thread.workspace_root,
        settings.permission_mode,
    ));
    let permission = ToolPermissionDescriptor::from(&descriptor);
    match policy.inspect_mcp_tool_call(&permission) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny { reason } => return Err(ApiError::bad_request(reason)),
        PolicyDecision::Ask { reason } => return Err(ApiError::bad_request(reason)),
    }
    if let Some(operation) = structured_operation.as_ref() {
        StoreConnectionOperationInvocationGate::new(state.store.clone())
            .authorize(operation)
            .await
            .map_err(|_| ApiError::forbidden("Connection operation is no longer authorized"))?;
    }

    let call = ToolCall::new(&descriptor.public_name, request.arguments.clone());
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallStarted { call: call.clone() },
    );

    let invocation = call_routed_mcp_tool(
        &state,
        structured_operation.as_ref(),
        &descriptor,
        request.arguments,
    )
    .await;
    let result = match invocation {
        Ok(result) => result,
        Err(err) => {
            let tool_result = ToolResult {
                call_id: call.id,
                output: err.to_string(),
                content: vec![ModelContentPart::text(err.to_string())],
                metadata: json!({
                    "success": false,
                    "error": err.to_string(),
                    "publicName": descriptor.public_name,
                    "toolName": descriptor.tool_name,
                    "serverId": descriptor.server_id,
                    "operationId": structured_operation.as_ref().map(|operation| &operation.operation_id),
                    "modelToolName": structured_operation.as_ref().map(|operation| &operation.model_tool_name),
                }),
            };
            publish_payload(
                &state,
                thread_id,
                None,
                AgentEventPayload::ToolCallFinished {
                    result: tool_result,
                },
            );
            return Err(ApiError::from(err));
        }
    };

    let tool_result = ToolResult {
        call_id: call.id,
        output: result.output.clone(),
        content: result
            .structured_content
            .clone()
            .map(ModelContentPart::json)
            .into_iter()
            .collect(),
        metadata: json!({
            "isError": result.is_error,
            "publicName": descriptor.public_name,
            "toolName": descriptor.tool_name,
            "serverId": descriptor.server_id,
            "operationId": structured_operation.as_ref().map(|operation| &operation.operation_id),
            "modelToolName": structured_operation.as_ref().map(|operation| &operation.model_tool_name),
        }),
    };
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: tool_result,
        },
    );

    Ok(Json(result))
}

async fn call_routed_mcp_tool(
    state: &AppState,
    structured_operation: Option<&ExecutionConnectionOperationV1>,
    descriptor: &McpToolDescriptor,
    arguments: Value,
) -> Result<McpCallResult, opentopia_core::mcp_host::McpHostError> {
    if let Some(operation) = structured_operation {
        return state
            .mcp_host
            .call_server_tool(
                operation.mcp_server_id,
                &operation.provider_tool_name,
                &operation.pinned_operation_fingerprint,
                arguments,
            )
            .await;
    }
    state
        .mcp_host
        .call_tool(&descriptor.public_name, arguments)
        .await
}

async fn list_thread_mcp_servers(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<ThreadMcpServerView>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let bindings = state.store.list_thread_mcp_servers(thread_id)?;
    let bindings_by_server = bindings
        .into_iter()
        .map(|binding| (binding.server_id, binding))
        .collect::<HashMap<_, _>>();
    let servers = state.store.list_mcp_servers()?;
    Ok(Json(
        servers
            .into_iter()
            .map(|server| {
                let binding = bindings_by_server.get(&server.server_id).cloned();
                let enabled = server.enabled && binding.as_ref().is_some_and(|item| item.enabled);
                ThreadMcpServerView {
                    enabled,
                    binding,
                    server,
                }
            })
            .collect(),
    ))
}

async fn set_thread_mcp_server(
    State(state): State<AppState>,
    Path((thread_id, server_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ThreadMcpServerRequest>,
) -> Result<Json<ThreadMcpServer>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    let binding = state
        .store
        .set_thread_mcp_server(thread_id, server_id, request.enabled)?;
    if request.enabled && server.enabled {
        let _ = ensure_mcp_server_status(&state.mcp_host, &server).await;
    }
    Ok(Json(binding))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpToolCallRequest {
    tool_name: String,
    arguments: Value,
    thread_id: Uuid,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerView {
    pub(crate) server: McpServerConfig,
    pub(crate) status: McpServerStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerRequest {
    name: String,
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    env_keys: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    enabled: Option<bool>,
}

impl McpServerRequest {
    fn into_config(self) -> Result<McpServerConfig, ApiError> {
        let name = self.name.trim();
        let command = self.command.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request("MCP server name cannot be empty"));
        }
        if command.is_empty() {
            return Err(ApiError::bad_request("MCP command cannot be empty"));
        }
        let mut config = McpServerConfig::new(name.to_string(), command.to_string());
        config.args = self.args.unwrap_or_default();
        config.cwd = self.cwd;
        config.env_keys = self.env_keys.unwrap_or_default();
        config.timeout_ms = self.timeout_ms.unwrap_or(30_000).clamp(1_000, 300_000);
        config.enabled = self.enabled.unwrap_or(true);
        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerPatchRequest {
    name: Option<String>,
    command: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    clear_cwd: Option<bool>,
    env_keys: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    enabled: Option<bool>,
}

impl McpServerPatchRequest {
    fn apply(self, config: &mut McpServerConfig) -> Result<(), ApiError> {
        if let Some(name) = self.name {
            let name = name.trim();
            if name.is_empty() {
                return Err(ApiError::bad_request("MCP server name cannot be empty"));
            }
            config.name = name.to_string();
        }
        if let Some(command) = self.command {
            let command = command.trim();
            if command.is_empty() {
                return Err(ApiError::bad_request("MCP command cannot be empty"));
            }
            config.command = command.to_string();
        }
        if let Some(args) = self.args {
            config.args = args;
        }
        if self.clear_cwd.unwrap_or(false) {
            config.cwd = None;
        } else if let Some(cwd) = self.cwd {
            config.cwd = Some(cwd);
        }
        if let Some(env_keys) = self.env_keys {
            config.env_keys = env_keys;
        }
        if let Some(timeout_ms) = self.timeout_ms {
            config.timeout_ms = timeout_ms.clamp(1_000, 300_000);
        }
        if let Some(enabled) = self.enabled {
            config.enabled = enabled;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadMcpServerRequest {
    enabled: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadMcpServerView {
    pub(crate) server: McpServerConfig,
    pub(crate) binding: Option<ThreadMcpServer>,
    pub(crate) enabled: bool,
}

#[cfg(test)]
mod connection_projection_tests {
    use super::*;

    #[test]
    fn credential_reference_keeps_legacy_definition_external_without_env_keys() {
        assert_eq!(
            legacy_mcp_auth_scheme(&[], true),
            IntegrationAuthSchemeV1::External
        );
        assert_eq!(
            legacy_mcp_auth_scheme(&[], false),
            IntegrationAuthSchemeV1::None
        );
    }
}
