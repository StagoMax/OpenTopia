use crate::{CapabilityProjection, ExecutionConnectionOperationV1};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Frozen authority for external Connection operations.
///
/// This discriminator is independent from collaboration snapshots because the
/// same immutable authority is also owned by persisted Flow runs. In
/// particular, `structured { operations: [] }` is an explicit empty grant and
/// must never fall back to mutable legacy thread MCP bindings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeConnectionAuthorityV1 {
    DenyAll,
    LegacyMcp,
    Structured {
        #[serde(default)]
        operations: Vec<ExecutionConnectionOperationV1>,
    },
}

impl RuntimeConnectionAuthorityV1 {
    /// Compatibility inference for data written before typed Connection
    /// authority existed. Only an explicit legacy MCP projection can opt in.
    pub fn inferred_from_projection(projection: &CapabilityProjection) -> Self {
        if projection.allow_all_mcp_servers || !projection.mcp_servers.is_empty() {
            Self::LegacyMcp
        } else {
            Self::DenyAll
        }
    }

    /// Narrows frozen operations to the effective capability ceiling without
    /// ever changing authority mode or consulting live thread state.
    pub fn attenuate(&self, projection: &CapabilityProjection) -> Self {
        match self {
            Self::DenyAll => Self::DenyAll,
            Self::LegacyMcp => Self::LegacyMcp,
            Self::Structured { operations } => Self::Structured {
                operations: operations
                    .iter()
                    .filter(|operation| {
                        projection.allows_tool(&operation.model_tool_name)
                            && projection.allows_mcp_server(&operation.mcp_server_id.to_string())
                    })
                    .cloned()
                    .collect(),
            },
        }
    }
}

/// Live authorization boundary for one frozen Connection operation.
///
/// Implementations are owned by the server so the core tool runtime never
/// receives a broad database or secret-store handle. The check is repeated
/// immediately before every external call.
#[async_trait]
pub trait ConnectionOperationInvocationGate: Send + Sync {
    async fn authorize(&self, operation: &ExecutionConnectionOperationV1) -> anyhow::Result<()>;
}

/// Cloneable runtime capability shared by direct model tools and indirect
/// consumers such as attachment inspection.
#[derive(Clone)]
pub struct ConnectionOperationRuntimeRoute {
    operation: ExecutionConnectionOperationV1,
    gate: Arc<dyn ConnectionOperationInvocationGate>,
}

impl ConnectionOperationRuntimeRoute {
    pub fn new(
        operation: ExecutionConnectionOperationV1,
        gate: Arc<dyn ConnectionOperationInvocationGate>,
    ) -> Self {
        Self { operation, gate }
    }

    pub fn operation(&self) -> &ExecutionConnectionOperationV1 {
        &self.operation
    }

    pub async fn authorize(&self) -> anyhow::Result<()> {
        self.gate.authorize(&self.operation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn operation() -> ExecutionConnectionOperationV1 {
        ExecutionConnectionOperationV1 {
            connection_id: Uuid::new_v4(),
            capability_revision: 1,
            operation_id: "connection:test:tool:lookup".to_string(),
            mcp_server_id: Uuid::new_v4(),
            provider_tool_name: "lookup".to_string(),
            model_tool_name: "lookup__account".to_string(),
            pinned_operation_fingerprint: "sha256:test".to_string(),
        }
    }

    #[test]
    fn structured_attenuation_never_changes_mode_or_expands_operations() {
        let operation = operation();
        let authority = RuntimeConnectionAuthorityV1::Structured {
            operations: vec![operation.clone()],
        };
        let mut projection = CapabilityProjection::deny_all();
        projection.tools.insert(operation.model_tool_name.clone());
        projection
            .mcp_servers
            .insert(operation.mcp_server_id.to_string());

        assert_eq!(authority.attenuate(&projection), authority);
        projection.tools.clear();
        assert_eq!(
            authority.attenuate(&projection),
            RuntimeConnectionAuthorityV1::Structured {
                operations: Vec::new()
            }
        );
    }

    #[test]
    fn compatibility_inference_requires_explicit_legacy_mcp_projection() {
        assert_eq!(
            RuntimeConnectionAuthorityV1::inferred_from_projection(
                &CapabilityProjection::deny_all()
            ),
            RuntimeConnectionAuthorityV1::DenyAll
        );
        let mut legacy = CapabilityProjection::deny_all();
        legacy.mcp_servers.insert(Uuid::new_v4().to_string());
        assert_eq!(
            RuntimeConnectionAuthorityV1::inferred_from_projection(&legacy),
            RuntimeConnectionAuthorityV1::LegacyMcp
        );
    }
}
