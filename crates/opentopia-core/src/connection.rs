use crate::connection_capability_fingerprint::{
    canonicalize_connection_capability_json, connection_capability_operation_fingerprint,
    normalize_permission_labels, normalize_public_mcp_annotations,
};
use crate::mcp::McpToolDescriptor;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const INTEGRATION_DEFINITION_SCHEMA_VERSION: u32 = 1;
pub const CONNECTION_SCHEMA_VERSION: u32 = 1;
pub const CONNECTION_CAPABILITY_REVISION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKindV1 {
    Mcp,
    OauthApi,
    Database,
    LocalApp,
}

impl IntegrationKindV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::OauthApi => "oauth_api",
            Self::Database => "database",
            Self::LocalApp => "local_app",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationAuthSchemeV1 {
    None,
    ApiKey,
    Oauth2,
    External,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDiscoveryKindV1 {
    McpToolsList,
    Static,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationDefinitionV1 {
    pub schema_version: u32,
    pub id: Uuid,
    pub revision: u32,
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: IntegrationKindV1,
    pub auth_scheme: IntegrationAuthSchemeV1,
    pub capability_discovery: CapabilityDiscoveryKindV1,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IntegrationDefinitionV1 {
    pub fn new(
        key: String,
        name: String,
        kind: IntegrationKindV1,
        auth_scheme: IntegrationAuthSchemeV1,
        capability_discovery: CapabilityDiscoveryKindV1,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: INTEGRATION_DEFINITION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            revision: 1,
            key,
            name,
            description: None,
            kind,
            auth_scheme,
            capability_discovery,
            enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionOwnerTypeV1 {
    Personal,
    OrgShared,
    ServiceAccount,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatusV1 {
    Configured,
    Ready,
    Degraded,
    ReauthRequired,
    Disabled,
}

impl ConnectionStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::ReauthRequired => "reauth_required",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionRuntimeBindingV1 {
    McpServer {
        #[serde(rename = "serverId")]
        server_id: Uuid,
    },
}

impl ConnectionRuntimeBindingV1 {
    pub fn mcp_server_id(&self) -> Uuid {
        match self {
            Self::McpServer { server_id } => *server_id,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionAccountV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionAuthContextV1 {
    /// Opaque reference resolved by a credential provider. Never a token or password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    pub verification: ConnectionAuthVerificationV1,
    #[serde(default)]
    pub account: ConnectionAccountV1,
    #[serde(default)]
    pub granted_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl Default for ConnectionAuthVerificationV1 {
    fn default() -> Self {
        Self::Unverified
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionAuthVerificationV1 {
    NotRequired,
    Unverified,
    LegacyUnverified,
    Verified,
}

pub fn connection_auth_is_runtime_ready(
    definition: &IntegrationDefinitionV1,
    connection: &ConnectionV1,
    now: DateTime<Utc>,
) -> bool {
    let verification_ready = matches!(
        connection.auth_context.verification,
        ConnectionAuthVerificationV1::Verified | ConnectionAuthVerificationV1::NotRequired
    ) || (connection.auth_context.verification
        == ConnectionAuthVerificationV1::LegacyUnverified
        && connection.auth_context.credential_ref.is_none()
        && definition.key.starts_with("legacy-mcp-"));
    verification_ready
        && connection
            .auth_context
            .expires_at
            .as_ref()
            .is_none_or(|expires_at| expires_at > &now)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionV1 {
    pub schema_version: u32,
    pub id: Uuid,
    pub revision: u32,
    pub integration_definition_id: Uuid,
    pub name: String,
    pub owner_type: ConnectionOwnerTypeV1,
    pub environment: String,
    pub enabled: bool,
    pub status: ConnectionStatusV1,
    pub runtime_binding: ConnectionRuntimeBindingV1,
    pub auth_context: ConnectionAuthContextV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_capability_revision: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tested_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConnectionV1 {
    pub fn new(
        integration_definition_id: Uuid,
        name: String,
        owner_type: ConnectionOwnerTypeV1,
        environment: String,
        runtime_binding: ConnectionRuntimeBindingV1,
        auth_context: ConnectionAuthContextV1,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: CONNECTION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            revision: 1,
            integration_definition_id,
            name,
            owner_type,
            environment,
            enabled: true,
            status: ConnectionStatusV1::Configured,
            runtime_binding,
            auth_context,
            active_capability_revision: None,
            last_tested_at: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionCapabilitySourceV1 {
    McpToolsList,
    Static,
}

impl ConnectionCapabilitySourceV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::McpToolsList => "mcp_tools_list",
            Self::Static => "static",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionCapabilityKindV1 {
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCapabilityProviderMetadataV1 {
    pub server_id: Uuid,
    pub public_name: String,
    pub tool_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCapabilityV1 {
    pub capability_id: String,
    pub kind: ConnectionCapabilityKindV1,
    pub name: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
    /// Only standard, public MCP behavior hints are projected here.
    pub annotations: Value,
    pub provider_metadata: ConnectionCapabilityProviderMetadataV1,
    pub permission_labels: Vec<String>,
}

impl ConnectionCapabilityV1 {
    pub fn from_mcp_tool(connection_id: Uuid, tool: &McpToolDescriptor) -> Self {
        Self {
            capability_id: format!("connection:{connection_id}:tool:{}", tool.tool_name),
            kind: ConnectionCapabilityKindV1::Tool,
            name: tool.tool_name.clone(),
            display_name: tool.tool_name.clone(),
            description: tool.description.clone(),
            input_schema: canonicalize_connection_capability_json(&tool.input_schema),
            annotations: normalize_public_mcp_annotations(&tool.annotations),
            provider_metadata: ConnectionCapabilityProviderMetadataV1 {
                server_id: tool.server_id,
                public_name: tool.public_name.clone(),
                tool_name: tool.tool_name.clone(),
            },
            permission_labels: normalize_permission_labels(&tool.permission_labels),
        }
    }
}

pub fn mcp_operation_fingerprint_from_capability(capability: &ConnectionCapabilityV1) -> String {
    connection_capability_operation_fingerprint(
        capability.provider_metadata.server_id.to_string(),
        &capability.provider_metadata.public_name,
        &capability.provider_metadata.tool_name,
        capability.description.as_deref(),
        &capability.input_schema,
        &normalize_public_mcp_annotations(&capability.annotations),
        &normalize_permission_labels(&capability.permission_labels),
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDiscoverySupportV1 {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCapabilityDiscoveryCoverageV1 {
    pub tools: CapabilityDiscoverySupportV1,
    pub resources: CapabilityDiscoverySupportV1,
    pub prompts: CapabilityDiscoverySupportV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCapabilityRevisionV1 {
    pub schema_version: u32,
    pub id: Uuid,
    pub connection_id: Uuid,
    pub revision: u32,
    pub source: ConnectionCapabilitySourceV1,
    pub content_hash: String,
    pub discovery_coverage: ConnectionCapabilityDiscoveryCoverageV1,
    pub capabilities: Vec<ConnectionCapabilityV1>,
    pub discovered_at: DateTime<Utc>,
}

impl ConnectionCapabilityRevisionV1 {
    pub fn from_mcp_tools(
        connection_id: Uuid,
        revision: u32,
        tools: &[McpToolDescriptor],
    ) -> anyhow::Result<Self> {
        let mut capabilities = tools
            .iter()
            .map(|tool| ConnectionCapabilityV1::from_mcp_tool(connection_id, tool))
            .collect::<Vec<_>>();
        capabilities.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        let encoded = serde_json::to_vec(&capabilities)?;
        let digest = Sha256::digest(encoded);
        let content_hash = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Ok(Self {
            schema_version: CONNECTION_CAPABILITY_REVISION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            connection_id,
            revision,
            source: ConnectionCapabilitySourceV1::McpToolsList,
            content_hash: format!("sha256:{content_hash}"),
            discovery_coverage: ConnectionCapabilityDiscoveryCoverageV1 {
                tools: CapabilityDiscoverySupportV1::Supported,
                resources: CapabilityDiscoverySupportV1::Unsupported,
                prompts: CapabilityDiscoverySupportV1::Unsupported,
            },
            capabilities,
            discovered_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mcp_capability_projection_drops_arbitrary_provider_metadata() {
        let tool = McpToolDescriptor {
            public_name: "crm__customers_get".to_string(),
            server_id: Uuid::new_v4(),
            tool_name: "customers_get".to_string(),
            description: Some("Read a customer".to_string()),
            input_schema: json!({"type": "object"}),
            annotations: json!({
                "readOnlyHint": true,
                "secretLookingField": "must-not-be-persisted"
            }),
            meta: json!({"token": "must-not-be-persisted"}),
            permission_labels: vec!["Read".to_string()],
        };

        let capability = ConnectionCapabilityV1::from_mcp_tool(Uuid::new_v4(), &tool);

        assert_eq!(capability.annotations, json!({"readOnlyHint": true}));
        let encoded = serde_json::to_string(&capability).expect("serialize capability");
        assert!(!encoded.contains("must-not-be-persisted"));
        assert_eq!(capability.permission_labels, vec!["read"]);
        assert_eq!(
            crate::mcp_operation_fingerprint(&tool),
            mcp_operation_fingerprint_from_capability(&capability)
        );
    }

    #[test]
    fn capability_hash_is_stable_across_tool_order() {
        let server_id = Uuid::new_v4();
        let tool = |name: &str| McpToolDescriptor {
            public_name: format!("crm__{name}"),
            server_id,
            tool_name: name.to_string(),
            description: None,
            input_schema: json!({"properties": {}, "type": "object"}),
            annotations: json!({}),
            meta: json!({}),
            permission_labels: vec!["unknown".to_string()],
        };
        let first = ConnectionCapabilityRevisionV1::from_mcp_tools(
            Uuid::new_v4(),
            1,
            &[tool("a"), tool("b")],
        )
        .expect("first revision");
        let second = ConnectionCapabilityRevisionV1::from_mcp_tools(
            first.connection_id,
            2,
            &[tool("b"), tool("a")],
        )
        .expect("second revision");

        assert_eq!(first.content_hash, second.content_hash);
    }
}
