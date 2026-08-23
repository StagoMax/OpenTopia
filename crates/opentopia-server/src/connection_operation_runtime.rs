use async_trait::async_trait;
use opentopia_core::collaboration::RuntimeConnectionAuthorityV1;
use opentopia_core::{
    connection_auth_is_runtime_ready, connection_model_tool_name,
    mcp_operation_fingerprint_from_capability, AgentInstanceV1, AgentTemplateVersionV1,
    CapabilityProjection, ConnectionCapabilityKindV1, ConnectionOperationInvocationGate,
    ConnectionStatusV1, ExecutionConnectionOperationV1, ExperienceMode, SqliteSessionStore,
};
use std::sync::Arc;

/// A frozen Connection operation can become unavailable while its Agent or
/// Workflow snapshot remains otherwise valid. This is a mutable business
/// precondition (revocation, reauthentication, capability review), not an
/// internal server failure. Keeping it typed lets API surfaces return a
/// recoverable conflict without hiding unexpected store/runtime errors.
#[derive(Debug)]
pub(crate) struct ConnectionOperationUnavailable {
    message: String,
}

impl ConnectionOperationUnavailable {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConnectionOperationUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConnectionOperationUnavailable {}

fn require_available(condition: bool, message: impl Into<String>) -> anyhow::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ConnectionOperationUnavailable::new(message).into())
    }
}

/// Store-backed, per-invocation guard for a frozen Connection operation.
///
/// The Agent catalog is only a disclosure optimization. This guard reloads all
/// mutable authority immediately before every external call, so disabling an
/// Integration/Connection, requiring reauthentication, changing the active
/// capability revision, or removing an operation takes effect for an already
/// running Turn.
pub(crate) struct StoreConnectionOperationInvocationGate {
    store: Arc<SqliteSessionStore>,
}

impl StoreConnectionOperationInvocationGate {
    pub(crate) fn new(store: Arc<SqliteSessionStore>) -> Self {
        Self { store }
    }
}

/// Resolves the one Connection authority mode consumed by root, continuation,
/// and child runtime setup. Structured template shape wins even when it grants
/// zero operations, preventing a fallback to mutable thread MCP bindings.
pub(super) fn connection_authority_for_context(
    mode: ExperienceMode,
    instance: Option<&AgentInstanceV1>,
    template: Option<&AgentTemplateVersionV1>,
    unbound_projection: &CapabilityProjection,
) -> RuntimeConnectionAuthorityV1 {
    if template.is_some_and(|template| !template.spec.connection_bindings.is_empty()) {
        return RuntimeConnectionAuthorityV1::Structured {
            operations: instance
                .map(|instance| instance.execution_context.connection_operations.clone())
                .unwrap_or_default(),
        };
    }
    if let Some(instance) = instance {
        let projection = &instance.execution_context.capabilities;
        return if projection.allow_all_mcp_servers || !projection.mcp_servers.is_empty() {
            RuntimeConnectionAuthorityV1::LegacyMcp
        } else {
            RuntimeConnectionAuthorityV1::DenyAll
        };
    }
    if mode == ExperienceMode::Flow {
        return RuntimeConnectionAuthorityV1::DenyAll;
    }
    if unbound_projection.allow_all_mcp_servers || !unbound_projection.mcp_servers.is_empty() {
        RuntimeConnectionAuthorityV1::LegacyMcp
    } else {
        RuntimeConnectionAuthorityV1::DenyAll
    }
}

#[async_trait]
impl ConnectionOperationInvocationGate for StoreConnectionOperationInvocationGate {
    async fn authorize(&self, operation: &ExecutionConnectionOperationV1) -> anyhow::Result<()> {
        let connection = self
            .store
            .get_connection(operation.connection_id)?
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new(format!(
                    "Connection {} no longer exists",
                    operation.connection_id
                ))
            })?;
        require_available(connection.enabled, "Connection is disabled")?;
        require_available(
            connection.status == ConnectionStatusV1::Ready,
            "Connection is not ready",
        )?;
        require_available(
            connection.runtime_binding.mcp_server_id() == operation.mcp_server_id,
            "Connection runtime no longer matches the frozen operation",
        )?;

        let definition = self
            .store
            .get_integration_definition(connection.integration_definition_id)?
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new(
                    "Connection Integration definition no longer exists",
                )
            })?;
        require_available(definition.enabled, "Connection Integration is disabled")?;
        require_available(
            connection_auth_is_runtime_ready(&definition, &connection, chrono::Utc::now()),
            "Connection account requires reauthentication",
        )?;

        let pinned = self
            .store
            .get_connection_capability_revision(
                operation.connection_id,
                operation.capability_revision,
            )?
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new(
                    "reviewed Connection capability revision no longer exists",
                )
            })?;
        let active_revision = connection.active_capability_revision.ok_or_else(|| {
            ConnectionOperationUnavailable::new("Connection has no active capability revision")
        })?;
        let active = self
            .store
            .get_connection_capability_revision(operation.connection_id, active_revision)?
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new(
                    "active Connection capability revision no longer exists",
                )
            })?;

        let pinned_capability = pinned
            .capabilities
            .iter()
            .find(|capability| capability.capability_id == operation.operation_id)
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new(
                    "granted operation was removed from its reviewed revision",
                )
            })?;
        let active_capability = active
            .capabilities
            .iter()
            .find(|capability| capability.capability_id == operation.operation_id)
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new("granted operation is no longer available")
            })?;
        for capability in [pinned_capability, active_capability] {
            require_available(
                capability.kind == ConnectionCapabilityKindV1::Tool,
                "granted capability is not an invocable tool",
            )?;
            require_available(
                capability.provider_metadata.server_id == operation.mcp_server_id
                    && capability.provider_metadata.tool_name == operation.provider_tool_name,
                "Connection operation route changed after review",
            )?;
            require_available(
                mcp_operation_fingerprint_from_capability(capability)
                    == operation.pinned_operation_fingerprint,
                "Connection operation descriptor changed after review",
            )?;
        }
        require_available(
            connection_model_tool_name(operation.connection_id, &operation.provider_tool_name)
                == operation.model_tool_name,
            "Connection model tool alias does not match the frozen operation",
        )?;

        let runtime = self
            .store
            .get_mcp_server(operation.mcp_server_id)?
            .ok_or_else(|| {
                ConnectionOperationUnavailable::new("Connection MCP runtime no longer exists")
            })?;
        require_available(runtime.enabled, "Connection MCP runtime is disabled")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::{
        CapabilityDiscoveryKindV1, ConnectionAccountV1, ConnectionAuthContextV1,
        ConnectionAuthVerificationV1, ConnectionCapabilityRevisionV1, ConnectionOwnerTypeV1,
        ConnectionRuntimeBindingV1, ConnectionV1, IntegrationAuthSchemeV1, IntegrationDefinitionV1,
        IntegrationKindV1, McpServerConfig, McpToolDescriptor,
    };
    use serde_json::json;
    use uuid::Uuid;

    fn tool(server_id: Uuid, description: &str) -> McpToolDescriptor {
        McpToolDescriptor {
            public_name: "crm__customers_get".to_string(),
            server_id,
            tool_name: "customers_get".to_string(),
            description: Some(description.to_string()),
            input_schema: json!({"type": "object"}),
            annotations: json!({"readOnlyHint": true}),
            meta: json!({}),
            permission_labels: vec!["read".to_string()],
        }
    }

    fn publish(
        store: &SqliteSessionStore,
        connection: &ConnectionV1,
        revision: u32,
        tools: &[McpToolDescriptor],
    ) -> ConnectionV1 {
        let capabilities =
            ConnectionCapabilityRevisionV1::from_mcp_tools(connection.id, revision, tools)
                .expect("capabilities");
        let mut next = connection.clone();
        next.touch();
        next.status = ConnectionStatusV1::Ready;
        next.active_capability_revision = Some(revision);
        store
            .publish_connection_capability_revision(&next, connection.revision, &capabilities)
            .expect("publish capabilities")
            .0
    }

    fn fixture() -> (
        Arc<SqliteSessionStore>,
        ConnectionV1,
        ExecutionConnectionOperationV1,
        McpToolDescriptor,
    ) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("store"));
        let server = store
            .insert_mcp_server(McpServerConfig::new(
                "CRM runtime".to_string(),
                "mock".to_string(),
            ))
            .expect("runtime");
        let definition = store
            .insert_integration_definition(&IntegrationDefinitionV1::new(
                "crm".to_string(),
                "CRM".to_string(),
                IntegrationKindV1::Mcp,
                IntegrationAuthSchemeV1::None,
                CapabilityDiscoveryKindV1::McpToolsList,
            ))
            .expect("definition");
        let connection = store
            .insert_connection(&ConnectionV1::new(
                definition.id,
                "CRM production".to_string(),
                ConnectionOwnerTypeV1::OrgShared,
                "production".to_string(),
                ConnectionRuntimeBindingV1::McpServer {
                    server_id: server.server_id,
                },
                ConnectionAuthContextV1 {
                    credential_ref: None,
                    verification: ConnectionAuthVerificationV1::NotRequired,
                    account: ConnectionAccountV1::default(),
                    granted_scopes: Vec::new(),
                    expires_at: None,
                },
            ))
            .expect("connection");
        let descriptor = tool(server.server_id, "Read customer");
        let connection = publish(&store, &connection, 1, std::slice::from_ref(&descriptor));
        let capability = store
            .get_connection_capability_revision(connection.id, 1)
            .expect("load revision")
            .expect("revision")
            .capabilities
            .into_iter()
            .next()
            .expect("capability");
        let operation = ExecutionConnectionOperationV1 {
            connection_id: connection.id,
            capability_revision: 1,
            operation_id: capability.capability_id.clone(),
            mcp_server_id: server.server_id,
            provider_tool_name: capability.provider_metadata.tool_name.clone(),
            model_tool_name: connection_model_tool_name(
                connection.id,
                &capability.provider_metadata.tool_name,
            ),
            pinned_operation_fingerprint: mcp_operation_fingerprint_from_capability(&capability),
        };
        (store, connection, operation, descriptor)
    }

    #[test]
    fn unbound_flow_denies_legacy_mcp_while_work_and_code_keep_compatibility() {
        let unrestricted = CapabilityProjection::unrestricted();
        assert_eq!(
            connection_authority_for_context(ExperienceMode::Flow, None, None, &unrestricted),
            RuntimeConnectionAuthorityV1::DenyAll
        );
        assert_eq!(
            connection_authority_for_context(ExperienceMode::Code, None, None, &unrestricted),
            RuntimeConnectionAuthorityV1::LegacyMcp
        );
    }

    #[tokio::test]
    async fn live_gate_revokes_an_already_frozen_operation_when_connection_is_disabled() {
        let (store, connection, operation, _) = fixture();
        let gate = StoreConnectionOperationInvocationGate::new(store.clone());
        gate.authorize(&operation)
            .await
            .expect("ready Connection should authorize");

        let mut disabled = connection.clone();
        disabled.touch();
        disabled.enabled = false;
        disabled.status = ConnectionStatusV1::Disabled;
        store
            .update_connection(&disabled, connection.revision)
            .expect("disable Connection");

        let err = gate
            .authorize(&operation)
            .await
            .expect_err("live disable must revoke an in-flight snapshot");
        assert!(err.to_string().contains("disabled"));
        assert!(err
            .downcast_ref::<ConnectionOperationUnavailable>()
            .is_some());
    }

    #[tokio::test]
    async fn live_gate_rejects_an_expired_verified_account() {
        let (store, connection, operation, _) = fixture();
        let mut expired = connection.clone();
        expired.touch();
        expired.auth_context.verification = ConnectionAuthVerificationV1::Verified;
        expired.auth_context.expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(1));
        store
            .update_connection(&expired, connection.revision)
            .expect("expire Connection account");

        let error = StoreConnectionOperationInvocationGate::new(store)
            .authorize(&operation)
            .await
            .expect_err("expired credential must fail closed");
        assert!(error.to_string().contains("reauthentication"));
    }

    #[tokio::test]
    async fn live_gate_rejects_changed_and_removed_active_operations() {
        let (store, connection, operation, descriptor) = fixture();
        let gate = StoreConnectionOperationInvocationGate::new(store.clone());
        let changed = tool(descriptor.server_id, "Changed after approval");
        let connection = publish(&store, &connection, 2, &[changed]);
        let changed_err = gate
            .authorize(&operation)
            .await
            .expect_err("changed operation must fail closed");
        assert!(changed_err.to_string().contains("changed after review"));

        let _connection = publish(&store, &connection, 3, &[]);
        let removed_err = gate
            .authorize(&operation)
            .await
            .expect_err("removed operation must fail closed");
        assert!(removed_err.to_string().contains("no longer available"));
    }
}
