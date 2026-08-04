use crate::model::ExperienceMode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub const ENTERPRISE_SCHEMA_VERSION_V1: u16 = 1;

/// A deterministic, fail-closed view of the capabilities available to one
/// Agent execution. `allow_all_*` is explicit so a missing field never means
/// unrestricted access when an ExecutionContext is deserialized.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CapabilityProjection {
    pub allow_all_tools: bool,
    pub tools: BTreeSet<String>,
    pub allow_all_skills: bool,
    pub skills: BTreeSet<String>,
    pub allow_all_plugins: bool,
    pub plugins: BTreeSet<String>,
    pub allow_all_mcp_servers: bool,
    pub mcp_servers: BTreeSet<String>,
    pub allow_all_workspace_roots: bool,
    pub workspace_roots: BTreeSet<PathBuf>,
}

impl Default for CapabilityProjection {
    fn default() -> Self {
        Self::deny_all()
    }
}

impl CapabilityProjection {
    pub fn deny_all() -> Self {
        Self {
            allow_all_tools: false,
            tools: BTreeSet::new(),
            allow_all_skills: false,
            skills: BTreeSet::new(),
            allow_all_plugins: false,
            plugins: BTreeSet::new(),
            allow_all_mcp_servers: false,
            mcp_servers: BTreeSet::new(),
            allow_all_workspace_roots: false,
            workspace_roots: BTreeSet::new(),
        }
    }

    pub fn unrestricted() -> Self {
        Self {
            allow_all_tools: true,
            allow_all_skills: true,
            allow_all_plugins: true,
            allow_all_mcp_servers: true,
            allow_all_workspace_roots: true,
            ..Self::deny_all()
        }
    }

    pub fn only_tools<I, S>(tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut projection = Self::unrestricted();
        projection.allow_all_tools = false;
        projection.tools = tools.into_iter().map(Into::into).collect();
        projection
    }

    /// Repeated projections only narrow. There is no operation that can widen
    /// a previously applied ExecutionContext.
    pub fn intersect(&self, other: &Self) -> Self {
        let (allow_all_tools, tools) = intersect_scope(
            self.allow_all_tools,
            &self.tools,
            other.allow_all_tools,
            &other.tools,
        );
        let (allow_all_skills, skills) = intersect_scope(
            self.allow_all_skills,
            &self.skills,
            other.allow_all_skills,
            &other.skills,
        );
        let (allow_all_plugins, plugins) = intersect_scope(
            self.allow_all_plugins,
            &self.plugins,
            other.allow_all_plugins,
            &other.plugins,
        );
        let (allow_all_mcp_servers, mcp_servers) = intersect_scope(
            self.allow_all_mcp_servers,
            &self.mcp_servers,
            other.allow_all_mcp_servers,
            &other.mcp_servers,
        );
        let (allow_all_workspace_roots, workspace_roots) = intersect_scope(
            self.allow_all_workspace_roots,
            &self.workspace_roots,
            other.allow_all_workspace_roots,
            &other.workspace_roots,
        );
        Self {
            allow_all_tools,
            tools,
            allow_all_skills,
            skills,
            allow_all_plugins,
            plugins,
            allow_all_mcp_servers,
            mcp_servers,
            allow_all_workspace_roots,
            workspace_roots,
        }
    }

    pub fn allows_tool(&self, name: &str) -> bool {
        self.allow_all_tools || self.tools.contains(name)
    }

    pub fn allows_skill(&self, id: &str) -> bool {
        self.allow_all_skills || self.skills.contains(id)
    }

    pub fn allows_plugin(&self, id_or_name: &str) -> bool {
        self.allow_all_plugins || self.plugins.contains(id_or_name)
    }

    pub fn allows_mcp_server(&self, server_id: &str) -> bool {
        self.allow_all_mcp_servers || self.mcp_servers.contains(server_id)
    }

    pub fn allows_workspace_root(&self, root: &Path) -> bool {
        self.allow_all_workspace_roots || self.workspace_roots.contains(root)
    }
}

fn intersect_scope<T: Clone + Ord>(
    left_all: bool,
    left: &BTreeSet<T>,
    right_all: bool,
    right: &BTreeSet<T>,
) -> (bool, BTreeSet<T>) {
    match (left_all, right_all) {
        (true, true) => (true, BTreeSet::new()),
        (true, false) => (false, right.clone()),
        (false, true) => (false, left.clone()),
        (false, false) => (false, left.intersection(right).cloned().collect()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperienceSurfaceProfile {
    pub mode: ExperienceMode,
    pub prompt_profile_id: String,
    pub enterprise_only: bool,
    pub capabilities: CapabilityProjection,
}

impl ExperienceSurfaceProfile {
    pub fn for_mode(mode: ExperienceMode) -> Self {
        match mode {
            ExperienceMode::Code => Self {
                mode,
                prompt_profile_id: "code.v1".to_string(),
                enterprise_only: false,
                capabilities: CapabilityProjection::unrestricted(),
            },
            ExperienceMode::Work => Self {
                mode,
                prompt_profile_id: "work.v1".to_string(),
                enterprise_only: false,
                capabilities: CapabilityProjection::unrestricted(),
            },
            ExperienceMode::Flow => {
                let mut capabilities = CapabilityProjection::only_tools([
                    "list_files",
                    "read_file",
                    "read_files",
                    "search",
                    "git_diff",
                    "list_skills",
                    "read_skill",
                    "complete_task",
                ]);
                // Phase 0 is a design-only baseline. Flow-specific tools arrive
                // in Phase 2; external plugins and MCP are opt-in through a
                // future Agent template rather than inherited from Code/Work.
                capabilities.allow_all_plugins = false;
                capabilities.plugins.clear();
                capabilities.allow_all_mcp_servers = false;
                capabilities.mcp_servers.clear();
                Self {
                    mode,
                    prompt_profile_id: "flow.v1".to_string(),
                    enterprise_only: true,
                    capabilities,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    File,
    Network,
    Database,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResourceGrantV1 {
    pub binding_id: String,
    pub kind: ResourceKind,
    /// A workspace-relative root, network origin, or logical database route.
    pub resource: String,
    pub can_read: bool,
    pub can_write: bool,
    pub max_data_classification: DataClassification,
}

#[derive(Debug, Clone)]
pub struct ExecutionIdentityRoute {
    pub grant: ExecutionResourceGrantV1,
    /// Server-side reference only. It is intentionally absent from the model-
    /// visible grant and from serialized ExecutionContext snapshots.
    credential_ref: String,
}

impl ExecutionIdentityRoute {
    pub fn new(grant: ExecutionResourceGrantV1, credential_ref: impl Into<String>) -> Self {
        Self {
            grant,
            credential_ref: credential_ref.into(),
        }
    }

    pub fn credential_ref(&self) -> &str {
        &self.credential_ref
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionIdentityRouter {
    routes: BTreeMap<String, ExecutionIdentityRoute>,
}

impl ExecutionIdentityRouter {
    pub fn new(routes: impl IntoIterator<Item = ExecutionIdentityRoute>) -> Self {
        Self {
            routes: routes
                .into_iter()
                .map(|route| (route.grant.binding_id.clone(), route))
                .collect(),
        }
    }

    pub fn resolve(
        &self,
        binding_id: &str,
        kind: ResourceKind,
        write: bool,
        classification: DataClassification,
    ) -> Result<&ExecutionIdentityRoute, ExecutionBoundaryError> {
        let route = self
            .routes
            .get(binding_id)
            .ok_or_else(|| ExecutionBoundaryError::UnknownBinding(binding_id.to_string()))?;
        if route.grant.kind != kind {
            return Err(ExecutionBoundaryError::KindMismatch {
                binding_id: binding_id.to_string(),
                expected: route.grant.kind,
                actual: kind,
            });
        }
        let operation_allowed = if write {
            route.grant.can_write
        } else {
            route.grant.can_read
        };
        if !operation_allowed {
            return Err(ExecutionBoundaryError::OperationDenied {
                binding_id: binding_id.to_string(),
                operation: if write { "write" } else { "read" },
            });
        }
        if classification > route.grant.max_data_classification {
            return Err(ExecutionBoundaryError::ClassificationDenied {
                binding_id: binding_id.to_string(),
                requested: classification,
                maximum: route.grant.max_data_classification,
            });
        }
        Ok(route)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExecutionBoundaryError {
    #[error("unknown execution binding: {0}")]
    UnknownBinding(String),
    #[error("execution binding {binding_id} is for {expected:?}, not {actual:?}")]
    KindMismatch {
        binding_id: String,
        expected: ResourceKind,
        actual: ResourceKind,
    },
    #[error("execution binding {binding_id} does not allow {operation}")]
    OperationDenied {
        binding_id: String,
        operation: &'static str,
    },
    #[error("execution binding {binding_id} allows at most {maximum:?}, not {requested:?}")]
    ClassificationDenied {
        binding_id: String,
        requested: DataClassification,
        maximum: DataClassification,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentDefinitionV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub name: String,
    pub instructions: String,
    pub capabilities: CapabilityProjection,
    pub state_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseExecutionContextV1 {
    pub schema_version: u16,
    pub agent_id: Uuid,
    pub thread_id: Uuid,
    pub mode: ExperienceMode,
    pub capabilities: CapabilityProjection,
    pub resource_grants: Vec<ExecutionResourceGrantV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowDefinitionV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub name: String,
    pub version: u32,
    pub graph: GraphDefinitionV1,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphDefinitionV1 {
    pub schema_version: u16,
    pub nodes: Vec<GraphNodeV1>,
    pub edges: Vec<GraphEdgeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeV1 {
    pub id: String,
    pub kind: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeV1 {
    pub from: String,
    pub to: String,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRecordV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub run_id: Uuid,
    pub source: String,
    pub content_hash: String,
    pub classification: DataClassification,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuditEventV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub actor_id: String,
    pub action: String,
    pub resource: String,
    pub outcome: String,
    pub evidence_ids: Vec<Uuid>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projections_are_fail_closed_and_only_narrow() {
        let missing_fields: CapabilityProjection =
            serde_json::from_value(serde_json::json!({})).expect("deserialize projection");
        assert!(!missing_fields.allows_tool("shell"));

        let first = CapabilityProjection::only_tools(["read_file", "shell"]);
        let second = CapabilityProjection::only_tools(["read_file", "write_file"]);
        let effective = first.intersect(&second);
        assert!(effective.allows_tool("read_file"));
        assert!(!effective.allows_tool("shell"));
        assert!(!effective.allows_tool("write_file"));
    }

    #[test]
    fn flow_profile_is_enterprise_only_and_excludes_external_capabilities() {
        let profile = ExperienceSurfaceProfile::for_mode(ExperienceMode::Flow);
        assert!(profile.enterprise_only);
        assert!(profile.capabilities.allows_tool("read_file"));
        assert!(!profile.capabilities.allows_tool("shell"));
        assert!(!profile.capabilities.allows_plugin("browser-automation"));
        assert!(!profile.capabilities.allows_mcp_server("server-1"));
    }

    #[test]
    fn identity_router_never_falls_back_to_another_binding() {
        let router = ExecutionIdentityRouter::new([ExecutionIdentityRoute::new(
            ExecutionResourceGrantV1 {
                binding_id: "finance-ro".to_string(),
                kind: ResourceKind::Database,
                resource: "finance".to_string(),
                can_read: true,
                can_write: false,
                max_data_classification: DataClassification::Confidential,
            },
            "vault://finance/reader",
        )]);

        let route = router
            .resolve(
                "finance-ro",
                ResourceKind::Database,
                false,
                DataClassification::Confidential,
            )
            .expect("resolve allowed route");
        assert_eq!(route.credential_ref(), "vault://finance/reader");
        assert!(matches!(
            router.resolve(
                "finance-ro",
                ResourceKind::Database,
                true,
                DataClassification::Internal,
            ),
            Err(ExecutionBoundaryError::OperationDenied { .. })
        ));
        assert!(matches!(
            router.resolve(
                "missing",
                ResourceKind::Database,
                false,
                DataClassification::Public,
            ),
            Err(ExecutionBoundaryError::UnknownBinding(_))
        ));
    }
}
