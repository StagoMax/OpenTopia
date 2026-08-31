use crate::enterprise_connection_grants::{
    connection_bindings_are_subset, diff_connection_bindings, intersect_connection_bindings,
    resolved_bindings_match, validate_connection_bindings_shape, ConnectionBindingV1,
    ConnectionGrantChangeKind, ConnectionGrantShapeError, ExecutionConnectionOperationV1,
    ResolvedConnectionBindingV1,
};
use crate::model::ExperienceMode;
use crate::model_context::content_fingerprint;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub const ENTERPRISE_SCHEMA_VERSION_V1: u16 = 1;
pub const MAX_AGENT_DELEGATION_DEPTH: u16 = 16;

/// A deterministic, fail-closed view of the capabilities available to one
/// Agent execution. `allow_all_*` is explicit so a missing field never means
/// unrestricted access when an ExecutionContext is deserialized.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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
        let (allow_all_workspace_roots, workspace_roots) = intersect_workspace_scope(
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

    /// Produces the smallest catalog ceiling that can host either projection.
    /// Individual workflow nodes must still execute against their own narrower
    /// projection; this operation is not an authorization boundary.
    pub fn union(&self, other: &Self) -> Self {
        let (allow_all_tools, tools) = union_scope(
            self.allow_all_tools,
            &self.tools,
            other.allow_all_tools,
            &other.tools,
        );
        let (allow_all_skills, skills) = union_scope(
            self.allow_all_skills,
            &self.skills,
            other.allow_all_skills,
            &other.skills,
        );
        let (allow_all_plugins, plugins) = union_scope(
            self.allow_all_plugins,
            &self.plugins,
            other.allow_all_plugins,
            &other.plugins,
        );
        let (allow_all_mcp_servers, mcp_servers) = union_scope(
            self.allow_all_mcp_servers,
            &self.mcp_servers,
            other.allow_all_mcp_servers,
            &other.mcp_servers,
        );
        let (allow_all_workspace_roots, workspace_roots) = union_workspace_scope(
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
        self.allow_all_workspace_roots
            || self
                .workspace_roots
                .iter()
                .any(|allowed| workspace_roots_equal(allowed, root))
    }

    /// Returns true when every capability in this projection is permitted by
    /// `ceiling`. Workspace paths use canonical identity rather than their
    /// serialized spelling, which is essential on Windows where the same path
    /// may appear with or without a verbatim (`\\?\`) prefix.
    pub fn is_subset_of(&self, ceiling: &Self) -> bool {
        scope_is_subset(
            self.allow_all_tools,
            &self.tools,
            ceiling.allow_all_tools,
            &ceiling.tools,
        ) && scope_is_subset(
            self.allow_all_skills,
            &self.skills,
            ceiling.allow_all_skills,
            &ceiling.skills,
        ) && scope_is_subset(
            self.allow_all_plugins,
            &self.plugins,
            ceiling.allow_all_plugins,
            &ceiling.plugins,
        ) && scope_is_subset(
            self.allow_all_mcp_servers,
            &self.mcp_servers,
            ceiling.allow_all_mcp_servers,
            &ceiling.mcp_servers,
        ) && workspace_scope_is_subset(
            self.allow_all_workspace_roots,
            &self.workspace_roots,
            ceiling.allow_all_workspace_roots,
            &ceiling.workspace_roots,
        )
    }
}

fn scope_is_subset<T: Ord>(
    requested_all: bool,
    requested: &BTreeSet<T>,
    ceiling_all: bool,
    ceiling: &BTreeSet<T>,
) -> bool {
    if requested_all {
        ceiling_all
    } else {
        ceiling_all || requested.is_subset(ceiling)
    }
}

fn workspace_scope_is_subset(
    requested_all: bool,
    requested: &BTreeSet<PathBuf>,
    ceiling_all: bool,
    ceiling: &BTreeSet<PathBuf>,
) -> bool {
    if requested_all {
        ceiling_all
    } else {
        ceiling_all
            || requested.iter().all(|root| {
                ceiling
                    .iter()
                    .any(|allowed| workspace_roots_equal(root, allowed))
            })
    }
}

fn workspace_root_identity(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let value = path.to_string_lossy().replace('/', "\\");
        let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else if let Some(rest) = value.strip_prefix(r"\\?\") {
            rest.to_string()
        } else if let Some(rest) = value.strip_prefix(r"\??\") {
            rest.to_string()
        } else {
            value
        };
        return PathBuf::from(value.to_lowercase());
    }
    #[cfg(not(windows))]
    path
}

fn workspace_roots_equal(left: &Path, right: &Path) -> bool {
    workspace_root_identity(left) == workspace_root_identity(right)
}

fn intersect_workspace_scope(
    left_all: bool,
    left: &BTreeSet<PathBuf>,
    right_all: bool,
    right: &BTreeSet<PathBuf>,
) -> (bool, BTreeSet<PathBuf>) {
    match (left_all, right_all) {
        (true, true) => (true, BTreeSet::new()),
        (true, false) => (false, normalized_workspace_roots(right)),
        (false, true) => (false, normalized_workspace_roots(left)),
        (false, false) => (
            false,
            left.iter()
                .filter(|candidate| {
                    right
                        .iter()
                        .any(|other| workspace_roots_equal(candidate, other))
                })
                .map(|path| workspace_root_identity(path))
                .collect(),
        ),
    }
}

fn union_workspace_scope(
    left_all: bool,
    left: &BTreeSet<PathBuf>,
    right_all: bool,
    right: &BTreeSet<PathBuf>,
) -> (bool, BTreeSet<PathBuf>) {
    if left_all || right_all {
        (true, BTreeSet::new())
    } else {
        (
            false,
            left.iter()
                .chain(right.iter())
                .map(|path| workspace_root_identity(path))
                .collect(),
        )
    }
}

fn normalized_workspace_roots(roots: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    roots
        .iter()
        .map(|path| workspace_root_identity(path))
        .collect()
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

fn union_scope<T: Clone + Ord>(
    left_all: bool,
    left: &BTreeSet<T>,
    right_all: bool,
    right: &BTreeSet<T>,
) -> (bool, BTreeSet<T>) {
    if left_all || right_all {
        (true, BTreeSet::new())
    } else {
        (false, left.union(right).cloned().collect())
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
    pub fn flow_control_tools() -> BTreeSet<String> {
        [
            "agent_search",
            "agent_create",
            "flow_search",
            "flow_create",
            "flow_update",
            "flow_inspect",
            "flow_validate",
            "flow_simulate",
            "flow_publish",
            "flow_run",
            "flow_status",
            "flow_pause",
            "flow_resume",
            "flow_cancel",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// Minimal authority for a Flow session that is not bound to an Agent.
    ///
    /// The workspace root is retained as the execution identity required by
    /// the policy boundary, while the tool surface remains limited to Flow
    /// control operations. Callers may add another explicit built-in such as
    /// `library_search`; external tools, plugins, and MCP stay denied.
    pub fn flow_runtime_baseline(workspace_root: PathBuf) -> CapabilityProjection {
        let mut capabilities = CapabilityProjection::deny_all();
        capabilities.tools = Self::flow_control_tools();
        capabilities.workspace_roots.insert(workspace_root);
        capabilities
    }

    /// Builds the catalog ceiling used by the Flow harness itself. Individual
    /// nodes are still attenuated to their own effective projection before
    /// execution, so this union cannot grant a node additional authority.
    pub fn flow_harness_capabilities(
        workspace_root: PathBuf,
        node_ceiling: &CapabilityProjection,
    ) -> CapabilityProjection {
        Self::flow_runtime_baseline(workspace_root).union(node_ceiling)
    }

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
            ExperienceMode::Flow => Self {
                mode,
                prompt_profile_id: "flow.v1".to_string(),
                enterprise_only: true,
                // Flow coordinates work that can span every Work and Code
                // capability. ExecutionContext projections may still narrow
                // this unrestricted surface for a bound Agent or Flow run.
                capabilities: CapabilityProjection::unrestricted(),
            },
        }
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    File,
    Network,
    Database,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnterpriseExecutionContextV1 {
    pub schema_version: u16,
    pub agent_id: Uuid,
    pub thread_id: Uuid,
    pub mode: ExperienceMode,
    pub template_id: String,
    pub template_version: u32,
    pub parent_agent_id: Option<Uuid>,
    pub delegation_chain: Vec<Uuid>,
    pub capabilities: CapabilityProjection,
    pub resource_grants: Vec<ExecutionResourceGrantV1>,
    pub model_policy: AgentModelPolicyV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_bindings: Vec<ConnectionBindingV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_operations: Vec<ExecutionConnectionOperationV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_binding: Option<AgentKnowledgeBindingV1>,
}

/// Knowledge retrieval backend selected by an Agent template. The provider is
/// part of the immutable Agent version; provider-owned databases and indexes
/// remain deployment configuration rather than template identity.
#[derive(
    Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "kebab-case")]
pub enum KnowledgeLibraryProviderV1 {
    #[default]
    Sag,
    GraphRag,
}

impl KnowledgeLibraryProviderV1 {
    pub fn is_sag(&self) -> bool {
        *self == Self::Sag
    }
}

/// Immutable, server-enforced knowledge scope available to an Agent. Provider
/// selection and any provider-specific namespace restriction are intentionally
/// absent from the model-facing tool schema, so a tool call cannot widen the
/// template's knowledge boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentKnowledgeBindingV1 {
    #[serde(default, skip_serializing_if = "KnowledgeLibraryProviderV1::is_sag")]
    pub provider: KnowledgeLibraryProviderV1,
    #[serde(default)]
    pub namespaces: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelBindingV1 {
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentModelPolicyV1 {
    pub allow_all_models: bool,
    pub allowed_models: BTreeSet<AgentModelBindingV1>,
}

impl Default for AgentModelPolicyV1 {
    fn default() -> Self {
        Self::deny_all()
    }
}

impl AgentModelPolicyV1 {
    pub fn deny_all() -> Self {
        Self {
            allow_all_models: false,
            allowed_models: BTreeSet::new(),
        }
    }

    pub fn unrestricted() -> Self {
        Self {
            allow_all_models: true,
            allowed_models: BTreeSet::new(),
        }
    }

    pub fn only(bindings: impl IntoIterator<Item = AgentModelBindingV1>) -> AgentModelPolicyV1 {
        Self {
            allow_all_models: false,
            allowed_models: bindings.into_iter().collect(),
        }
    }

    pub fn allows(&self, provider_id: &str, model_id: &str) -> bool {
        self.allow_all_models
            || self.allowed_models.contains(&AgentModelBindingV1 {
                provider_id: provider_id.to_string(),
                model_id: model_id.to_string(),
            })
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let (allow_all_models, allowed_models) = intersect_scope(
            self.allow_all_models,
            &self.allowed_models,
            other.allow_all_models,
            &other.allowed_models,
        );
        Self {
            allow_all_models,
            allowed_models,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRiskClassV1 {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentBudgetV1 {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_duration_seconds: u64,
}

impl Default for AgentBudgetV1 {
    fn default() -> Self {
        Self {
            max_turns: 20,
            max_tool_calls: 40,
            max_duration_seconds: 900,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplateSpecV1 {
    pub description: String,
    pub instructions: String,
    pub capabilities: CapabilityProjection,
    pub resource_grants: Vec<ExecutionResourceGrantV1>,
    pub model_policy: AgentModelPolicyV1,
    pub state_schema: Value,
    pub output_schema: Value,
    pub allow_all_delegates: bool,
    pub delegate_template_ids: BTreeSet<String>,
    pub budget: AgentBudgetV1,
    pub risk_class: AgentRiskClassV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_bindings: Vec<ConnectionBindingV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_binding: Option<AgentKnowledgeBindingV1>,
}

impl AgentTemplateSpecV1 {
    /// Knowledge access is a declarative Agent binding, not a second manual
    /// permission toggle. Persist the derived tool capability so compilation,
    /// diffs, and runtime attenuation all observe the same authority.
    pub fn apply_derived_capabilities(&mut self) {
        if self.knowledge_binding.is_some() && !self.capabilities.allow_all_tools {
            self.capabilities.tools.insert("library_search".to_string());
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTemplateStatusV1 {
    Draft,
    Published,
}

impl AgentTemplateStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplateVersionV1 {
    pub schema_version: u16,
    pub template_id: String,
    pub version: u32,
    pub name: String,
    pub owner: String,
    pub spec: AgentTemplateSpecV1,
    pub status: AgentTemplateStatusV1,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
    pub published_by: Option<String>,
}

impl AgentTemplateVersionV1 {
    pub fn new_draft(
        template_id: impl Into<String>,
        version: u32,
        name: impl Into<String>,
        owner: impl Into<String>,
        mut spec: AgentTemplateSpecV1,
    ) -> Result<Self, AgentTemplateError> {
        spec.apply_derived_capabilities();
        let mut template = Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            template_id: template_id.into(),
            version,
            name: name.into(),
            owner: owner.into(),
            spec,
            status: AgentTemplateStatusV1::Draft,
            content_hash: String::new(),
            created_at: Utc::now(),
            published_at: None,
            published_by: None,
        };
        template.validate()?;
        template.content_hash = template.calculate_content_hash();
        Ok(template)
    }

    pub fn validate(&self) -> Result<(), AgentTemplateError> {
        if self.schema_version != ENTERPRISE_SCHEMA_VERSION_V1 {
            return Err(AgentTemplateError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if !valid_template_id(&self.template_id) {
            return Err(AgentTemplateError::InvalidTemplateId);
        }
        if self.version == 0 {
            return Err(AgentTemplateError::InvalidVersion);
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err(AgentTemplateError::InvalidName);
        }
        if self.owner.trim().is_empty() || self.owner.chars().count() > 160 {
            return Err(AgentTemplateError::InvalidOwner);
        }
        if self.spec.instructions.trim().is_empty()
            || self.spec.instructions.chars().count() > 40_000
        {
            return Err(AgentTemplateError::InvalidInstructions);
        }
        if self.spec.description.chars().count() > 4_000 {
            return Err(AgentTemplateError::InvalidDescription);
        }
        validate_projection_shape(&self.spec.capabilities)?;
        validate_connection_bindings_shape(&self.spec.connection_bindings)?;
        validate_knowledge_binding_shape(
            self.spec.knowledge_binding.as_ref(),
            &self.spec.capabilities,
        )?;
        if !self.spec.connection_bindings.is_empty()
            && (self.spec.capabilities.allow_all_mcp_servers
                || !self.spec.capabilities.mcp_servers.is_empty())
        {
            return Err(AgentTemplateError::AmbiguousConnectionGrantPolicy);
        }
        validate_model_policy_shape(&self.spec.model_policy)?;
        validate_schema_shape(&self.spec.state_schema, "stateSchema")?;
        validate_schema_shape(&self.spec.output_schema, "outputSchema")?;
        if self.spec.allow_all_delegates && !self.spec.delegate_template_ids.is_empty() {
            return Err(AgentTemplateError::AmbiguousDelegatePolicy);
        }
        if self
            .spec
            .delegate_template_ids
            .iter()
            .any(|id| !valid_template_id(id))
        {
            return Err(AgentTemplateError::InvalidDelegateTemplateId);
        }
        if self.spec.model_policy.allowed_models.iter().any(|binding| {
            binding.provider_id.trim().is_empty() || binding.model_id.trim().is_empty()
        }) {
            return Err(AgentTemplateError::InvalidModelBinding);
        }
        if self.spec.budget.max_turns == 0
            || self.spec.budget.max_tool_calls == 0
            || self.spec.budget.max_duration_seconds == 0
        {
            return Err(AgentTemplateError::InvalidBudget);
        }
        let mut binding_ids = BTreeSet::new();
        for grant in &self.spec.resource_grants {
            if grant.binding_id.trim().is_empty() || grant.resource.trim().is_empty() {
                return Err(AgentTemplateError::InvalidResourceGrant(
                    grant.binding_id.clone(),
                ));
            }
            if !grant.can_read && !grant.can_write {
                return Err(AgentTemplateError::InvalidResourceGrant(
                    grant.binding_id.clone(),
                ));
            }
            if !binding_ids.insert(grant.binding_id.clone()) {
                return Err(AgentTemplateError::DuplicateResourceGrant(
                    grant.binding_id.clone(),
                ));
            }
        }
        Ok(())
    }

    pub fn publish(
        &self,
        approved_by: &str,
        previous_published: Option<&Self>,
        approve_capability_expansion: bool,
    ) -> Result<(Self, AgentTemplateDiffV1), AgentTemplateError> {
        self.validate()?;
        if self.content_hash != self.calculate_content_hash() {
            return Err(AgentTemplateError::ContentHashMismatch);
        }
        if self.status != AgentTemplateStatusV1::Draft {
            return Err(AgentTemplateError::VersionIsImmutable);
        }
        if approved_by.trim() != self.owner {
            return Err(AgentTemplateError::OwnerApprovalRequired);
        }
        if let Some(previous) = previous_published {
            if previous.template_id != self.template_id || previous.version >= self.version {
                return Err(AgentTemplateError::InvalidPreviousVersion);
            }
        }
        let diff = AgentTemplateDiffV1::between(previous_published, self);
        if diff.widens_capabilities && !approve_capability_expansion {
            return Err(AgentTemplateError::CapabilityExpansionApprovalRequired);
        }
        let mut published = self.clone();
        published.status = AgentTemplateStatusV1::Published;
        published.published_at = Some(Utc::now());
        published.published_by = Some(approved_by.to_string());
        Ok((published, diff))
    }

    pub fn calculate_content_hash(&self) -> String {
        let immutable_content = serde_json::to_vec(&(
            self.schema_version,
            &self.template_id,
            self.version,
            &self.name,
            &self.owner,
            &self.spec,
        ))
        .unwrap_or_default();
        content_fingerprint(&immutable_content)
    }

    pub fn validate_state(&self, state: &Value) -> Result<(), AgentTemplateError> {
        validate_value_against_schema(&self.spec.state_schema, state, "$state")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityChangeKindV1 {
    Added,
    Removed,
    Expanded,
    Reduced,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityChangeV1 {
    pub scope: String,
    pub value: String,
    pub kind: CapabilityChangeKindV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplateDiffV1 {
    pub from_version: Option<u32>,
    pub to_version: u32,
    pub changes: Vec<CapabilityChangeV1>,
    pub widens_capabilities: bool,
}

impl AgentTemplateDiffV1 {
    pub fn between(
        previous: Option<&AgentTemplateVersionV1>,
        next: &AgentTemplateVersionV1,
    ) -> Self {
        let mut changes = Vec::new();
        match previous {
            Some(previous) => {
                diff_projection(
                    &previous.spec.capabilities,
                    &next.spec.capabilities,
                    &mut changes,
                );
                diff_models(
                    &previous.spec.model_policy,
                    &next.spec.model_policy,
                    &mut changes,
                );
                diff_resource_grants(
                    &previous.spec.resource_grants,
                    &next.spec.resource_grants,
                    &mut changes,
                );
                diff_delegates(&previous.spec, &next.spec, &mut changes);
                connection_grant_changes(
                    &previous.spec.connection_bindings,
                    &next.spec.connection_bindings,
                    &mut changes,
                );
                knowledge_binding_changes(
                    previous.spec.knowledge_binding.as_ref(),
                    next.spec.knowledge_binding.as_ref(),
                    &mut changes,
                );
            }
            None => {
                projection_additions(&next.spec.capabilities, &mut changes);
                model_additions(&next.spec.model_policy, &mut changes);
                for grant in &next.spec.resource_grants {
                    changes.push(CapabilityChangeV1 {
                        scope: "resource".to_string(),
                        value: grant.binding_id.clone(),
                        kind: CapabilityChangeKindV1::Added,
                    });
                }
                if next.spec.allow_all_delegates {
                    changes.push(CapabilityChangeV1 {
                        scope: "delegate".to_string(),
                        value: "*".to_string(),
                        kind: CapabilityChangeKindV1::Expanded,
                    });
                } else {
                    for id in &next.spec.delegate_template_ids {
                        changes.push(CapabilityChangeV1 {
                            scope: "delegate".to_string(),
                            value: id.clone(),
                            kind: CapabilityChangeKindV1::Added,
                        });
                    }
                }
                connection_grant_changes(&[], &next.spec.connection_bindings, &mut changes);
                knowledge_binding_changes(None, next.spec.knowledge_binding.as_ref(), &mut changes);
            }
        }
        let widens_capabilities = changes.iter().any(|change| {
            matches!(
                change.kind,
                CapabilityChangeKindV1::Added | CapabilityChangeKindV1::Expanded
            )
        });
        Self {
            from_version: previous.map(|template| template.version),
            to_version: next.version,
            changes,
            widens_capabilities,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentInstanceStatusV1 {
    Active,
    Suspended,
    Completed,
    Revoked,
}

impl AgentInstanceStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Completed => "completed",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstanceV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub template_id: String,
    pub template_version: u32,
    pub thread_id: Uuid,
    pub parent_instance_id: Option<Uuid>,
    pub delegation_depth: u16,
    pub execution_context: EnterpriseExecutionContextV1,
    pub state: Value,
    pub state_revision: u64,
    pub status: AgentInstanceStatusV1,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AgentInstanceV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate(
        template: &AgentTemplateVersionV1,
        thread_id: Uuid,
        mode: ExperienceMode,
        mode_capabilities: &CapabilityProjection,
        parent: Option<&AgentInstanceV1>,
        parent_template: Option<&AgentTemplateVersionV1>,
        requested_capabilities: Option<&CapabilityProjection>,
        requested_resource_grants: Option<&[ExecutionResourceGrantV1]>,
        requested_model_policy: Option<&AgentModelPolicyV1>,
        initial_state: Value,
    ) -> Result<Self, AgentTemplateError> {
        Self::instantiate_with_connections(
            template,
            thread_id,
            mode,
            mode_capabilities,
            parent,
            parent_template,
            requested_capabilities,
            requested_resource_grants,
            requested_model_policy,
            None,
            &[],
            initial_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_with_connections(
        template: &AgentTemplateVersionV1,
        thread_id: Uuid,
        mode: ExperienceMode,
        mode_capabilities: &CapabilityProjection,
        parent: Option<&AgentInstanceV1>,
        parent_template: Option<&AgentTemplateVersionV1>,
        requested_capabilities: Option<&CapabilityProjection>,
        requested_resource_grants: Option<&[ExecutionResourceGrantV1]>,
        requested_model_policy: Option<&AgentModelPolicyV1>,
        requested_connection_bindings: Option<&[ConnectionBindingV1]>,
        resolved_connection_bindings: &[ResolvedConnectionBindingV1],
        initial_state: Value,
    ) -> Result<Self, AgentTemplateError> {
        template.validate()?;
        if template.content_hash != template.calculate_content_hash() {
            return Err(AgentTemplateError::ContentHashMismatch);
        }
        if template.status != AgentTemplateStatusV1::Published {
            return Err(AgentTemplateError::TemplateNotPublished);
        }
        validate_value_against_schema(&template.spec.state_schema, &initial_state, "$state")?;

        if !resolved_bindings_match(
            &template.spec.connection_bindings,
            resolved_connection_bindings,
        ) {
            return Err(AgentTemplateError::InvalidResolvedConnectionBindings);
        }
        let template_capabilities = capabilities_with_resolved_connections(
            &template.spec.capabilities,
            resolved_connection_bindings,
        );
        let mut capabilities = mode_capabilities.intersect(&template_capabilities);
        let mut grants = template.spec.resource_grants.clone();
        let mut model_policy = template.spec.model_policy.clone();
        let mut connection_bindings = template.spec.connection_bindings.clone();
        let (parent_instance_id, parent_agent_id, delegation_depth, delegation_chain) =
            if let Some(parent) = parent {
                if parent.thread_id != thread_id || parent.execution_context.mode != mode {
                    return Err(AgentTemplateError::DelegationContextMismatch);
                }
                if parent.status != AgentInstanceStatusV1::Active {
                    return Err(AgentTemplateError::ParentInstanceNotActive);
                }
                let parent_template = parent_template
                    .filter(|parent_template| {
                        parent_template.template_id == parent.template_id
                            && parent_template.version == parent.template_version
                    })
                    .ok_or(AgentTemplateError::ParentTemplateMismatch)?;
                parent_template.validate()?;
                if parent_template.status != AgentTemplateStatusV1::Published
                    || parent_template.content_hash != parent_template.calculate_content_hash()
                {
                    return Err(AgentTemplateError::ParentTemplateMismatch);
                }
                if !parent_template.spec.allow_all_delegates
                    && !parent_template
                        .spec
                        .delegate_template_ids
                        .contains(&template.template_id)
                {
                    return Err(AgentTemplateError::DelegateTemplateDenied(
                        template.template_id.clone(),
                    ));
                }
                let depth = parent
                    .delegation_depth
                    .checked_add(1)
                    .ok_or(AgentTemplateError::DelegationDepthExceeded)?;
                if depth > MAX_AGENT_DELEGATION_DEPTH {
                    return Err(AgentTemplateError::DelegationDepthExceeded);
                }
                capabilities = capabilities.intersect(&parent.execution_context.capabilities);
                grants =
                    intersect_resource_grants(&grants, &parent.execution_context.resource_grants);
                model_policy = model_policy.intersect(&parent.execution_context.model_policy);
                connection_bindings = intersect_connection_bindings(
                    &connection_bindings,
                    &parent.execution_context.connection_bindings,
                );
                let mut chain = parent.execution_context.delegation_chain.clone();
                chain.push(parent.id);
                (Some(parent.id), Some(parent.id), depth, chain)
            } else {
                (None, None, 0, Vec::new())
            };

        if let Some(requested) = requested_capabilities {
            capabilities = capabilities.intersect(requested);
        }
        if let Some(requested) = requested_resource_grants {
            grants = intersect_resource_grants(&grants, requested);
        }
        if let Some(requested) = requested_model_policy {
            model_policy = model_policy.intersect(requested);
        }
        if let Some(requested) = requested_connection_bindings {
            validate_connection_bindings_shape(requested)?;
            connection_bindings = intersect_connection_bindings(&connection_bindings, requested);
        }

        let effective_connection_ids = connection_bindings
            .iter()
            .map(|binding| binding.connection_id)
            .collect::<BTreeSet<_>>();
        let effective_operation_ids = connection_bindings
            .iter()
            .map(|binding| {
                (
                    binding.connection_id,
                    binding
                        .operation_grants
                        .iter()
                        .map(|grant| grant.operation_id.as_str())
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut connection_operations = resolved_connection_bindings
            .iter()
            .filter(|binding| effective_connection_ids.contains(&binding.binding.connection_id))
            .flat_map(|binding| binding.operations.iter())
            .filter(|(_, operation)| {
                effective_operation_ids
                    .get(&operation.connection_id)
                    .is_some_and(|operation_ids| {
                        operation_ids.contains(operation.operation_id.as_str())
                    })
            })
            .map(|(_, operation)| operation.clone())
            .collect::<Vec<_>>();
        connection_operations.sort_by(|left, right| {
            (left.connection_id, left.operation_id.as_str())
                .cmp(&(right.connection_id, right.operation_id.as_str()))
        });
        capabilities = capabilities.intersect(&capabilities_with_connection_operations(
            &template.spec.capabilities,
            &connection_operations,
        ));

        let now = Utc::now();
        let id = Uuid::new_v4();
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id,
            template_id: template.template_id.clone(),
            template_version: template.version,
            thread_id,
            parent_instance_id,
            delegation_depth,
            execution_context: EnterpriseExecutionContextV1 {
                schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
                agent_id: id,
                thread_id,
                mode,
                template_id: template.template_id.clone(),
                template_version: template.version,
                parent_agent_id,
                delegation_chain,
                capabilities,
                resource_grants: grants,
                model_policy,
                connection_bindings,
                connection_operations,
                knowledge_binding: template.spec.knowledge_binding.clone(),
            },
            state: initial_state,
            state_revision: 1,
            status: AgentInstanceStatusV1::Active,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn validate_execution_boundary(
        &self,
        template: &AgentTemplateVersionV1,
        mode_capabilities: &CapabilityProjection,
    ) -> Result<(), AgentTemplateError> {
        self.validate_execution_boundary_with_connections(template, mode_capabilities, &[])
    }

    pub fn validate_execution_boundary_with_connections(
        &self,
        template: &AgentTemplateVersionV1,
        mode_capabilities: &CapabilityProjection,
        resolved_connection_bindings: &[ResolvedConnectionBindingV1],
    ) -> Result<(), AgentTemplateError> {
        template.validate()?;
        if template.status != AgentTemplateStatusV1::Published
            || template.content_hash != template.calculate_content_hash()
        {
            return Err(AgentTemplateError::ContentHashMismatch);
        }
        if self.schema_version != ENTERPRISE_SCHEMA_VERSION_V1
            || self.template_id != template.template_id
            || self.template_version != template.version
            || self.execution_context.schema_version != ENTERPRISE_SCHEMA_VERSION_V1
            || self.execution_context.agent_id != self.id
            || self.execution_context.thread_id != self.thread_id
            || self.execution_context.template_id != self.template_id
            || self.execution_context.template_version != self.template_version
        {
            return Err(AgentTemplateError::InvalidInstanceContext);
        }
        let template_boundary =
            mode_capabilities.intersect(&capabilities_with_resolved_connections(
                &template.spec.capabilities,
                resolved_connection_bindings,
            ));
        if !self
            .execution_context
            .capabilities
            .is_subset_of(&template_boundary)
        {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        if self
            .execution_context
            .model_policy
            .intersect(&template.spec.model_policy)
            != self.execution_context.model_policy
        {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        if !resource_grants_are_subset(
            &self.execution_context.resource_grants,
            &template.spec.resource_grants,
        ) {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        if validate_connection_bindings_shape(&self.execution_context.connection_bindings).is_err()
            || !resolved_bindings_match(
                &template.spec.connection_bindings,
                resolved_connection_bindings,
            )
            || !connection_bindings_are_subset(
                &self.execution_context.connection_bindings,
                &template.spec.connection_bindings,
            )
        {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        let effective_connection_ids = self
            .execution_context
            .connection_bindings
            .iter()
            .map(|binding| binding.connection_id)
            .collect::<BTreeSet<_>>();
        let effective_operation_ids = self
            .execution_context
            .connection_bindings
            .iter()
            .map(|binding| {
                (
                    binding.connection_id,
                    binding
                        .operation_grants
                        .iter()
                        .map(|grant| grant.operation_id.as_str())
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut expected_operations = resolved_connection_bindings
            .iter()
            .filter(|binding| effective_connection_ids.contains(&binding.binding.connection_id))
            .flat_map(|binding| binding.operations.iter())
            .filter(|(_, operation)| {
                effective_operation_ids
                    .get(&operation.connection_id)
                    .is_some_and(|operation_ids| {
                        operation_ids.contains(operation.operation_id.as_str())
                    })
            })
            .map(|(_, operation)| operation.clone())
            .collect::<Vec<_>>();
        expected_operations.sort_by(|left, right| {
            (left.connection_id, left.operation_id.as_str())
                .cmp(&(right.connection_id, right.operation_id.as_str()))
        });
        if self.execution_context.connection_operations != expected_operations {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        if self.execution_context.knowledge_binding != template.spec.knowledge_binding {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        let effective_template_boundary =
            mode_capabilities.intersect(&capabilities_with_connection_operations(
                &template.spec.capabilities,
                &expected_operations,
            ));
        if !self
            .execution_context
            .capabilities
            .is_subset_of(&effective_template_boundary)
        {
            return Err(AgentTemplateError::InstanceCapabilityViolation);
        }
        if self.delegation_depth as usize != self.execution_context.delegation_chain.len()
            || self.delegation_depth > MAX_AGENT_DELEGATION_DEPTH
            || (self.parent_instance_id.is_none()
                && (self.delegation_depth != 0 || self.execution_context.parent_agent_id.is_some()))
            || (self.parent_instance_id.is_some()
                && self.execution_context.parent_agent_id != self.parent_instance_id)
        {
            return Err(AgentTemplateError::InvalidInstanceContext);
        }
        template.validate_state(&self.state)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentTemplateError {
    #[error(transparent)]
    InvalidConnectionGrantShape(#[from] ConnectionGrantShapeError),
    #[error("resolved Connection grants do not match the Agent template")]
    InvalidResolvedConnectionBindings,
    #[error("connectionBindings cannot be combined with legacy mcpServers grants")]
    AmbiguousConnectionGrantPolicy,
    #[error("unsupported Agent template schema version: {0}")]
    UnsupportedSchemaVersion(u16),
    #[error(
        "templateId must be a lowercase slug containing only letters, numbers, '.', '_' or '-'"
    )]
    InvalidTemplateId,
    #[error("Agent template version must be greater than zero")]
    InvalidVersion,
    #[error("Agent template name must contain between 1 and 120 characters")]
    InvalidName,
    #[error("Agent template owner must contain between 1 and 160 characters")]
    InvalidOwner,
    #[error("Agent template instructions must contain between 1 and 40000 characters")]
    InvalidInstructions,
    #[error("Agent template description cannot exceed 4000 characters")]
    InvalidDescription,
    #[error("SAG knowledge binding requires between 1 and 12 namespaces")]
    InvalidKnowledgeNamespaceCount,
    #[error("invalid SAG knowledge namespace: {0}")]
    InvalidKnowledgeNamespace(String),
    #[error("Graph RAG knowledge binding does not accept SAG namespaces")]
    GraphRagNamespacesUnsupported,
    #[error("knowledge binding requires the derived library_search tool capability")]
    MissingLibrarySearchCapability,
    #[error("{0} must be a JSON Schema object or boolean")]
    InvalidSchema(String),
    #[error("capability projection contains entries under an allow-all scope: {0}")]
    AmbiguousCapabilityScope(String),
    #[error("model policy contains entries while allowAllModels is true")]
    AmbiguousModelPolicy,
    #[error("delegateTemplateIds must be empty while allowAllDelegates is true")]
    AmbiguousDelegatePolicy,
    #[error("delegateTemplateIds contains an invalid template ID")]
    InvalidDelegateTemplateId,
    #[error("model policy bindings require non-empty providerId and modelId")]
    InvalidModelBinding,
    #[error("Agent budget limits must be greater than zero")]
    InvalidBudget,
    #[error("invalid resource grant: {0}")]
    InvalidResourceGrant(String),
    #[error("duplicate resource bindingId: {0}")]
    DuplicateResourceGrant(String),
    #[error("published Agent template versions are immutable")]
    VersionIsImmutable,
    #[error("Agent template content hash does not match its immutable fields")]
    ContentHashMismatch,
    #[error("Agent template publication requires approval by its owner")]
    OwnerApprovalRequired,
    #[error("the previous published Agent template version is invalid")]
    InvalidPreviousVersion,
    #[error("publishing a capability expansion requires explicit approval")]
    CapabilityExpansionApprovalRequired,
    #[error("Agent instances can only use published templates")]
    TemplateNotPublished,
    #[error("parent Agent instance is not active")]
    ParentInstanceNotActive,
    #[error("parent Agent template does not match the instance")]
    ParentTemplateMismatch,
    #[error("parent and child Agent instances must share a thread and mode")]
    DelegationContextMismatch,
    #[error("parent Agent template does not allow delegate template: {0}")]
    DelegateTemplateDenied(String),
    #[error("maximum Agent delegation depth exceeded")]
    DelegationDepthExceeded,
    #[error("Agent instance execution context is internally inconsistent")]
    InvalidInstanceContext,
    #[error("Agent instance execution context exceeds its template or mode boundary")]
    InstanceCapabilityViolation,
    #[error("state schema validation failed at {path}: {message}")]
    StateSchemaViolation { path: String, message: String },
}

fn valid_template_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn validate_knowledge_binding_shape(
    binding: Option<&AgentKnowledgeBindingV1>,
    capabilities: &CapabilityProjection,
) -> Result<(), AgentTemplateError> {
    let Some(binding) = binding else {
        return Ok(());
    };
    match binding.provider {
        KnowledgeLibraryProviderV1::Sag => {
            if binding.namespaces.is_empty() || binding.namespaces.len() > 12 {
                return Err(AgentTemplateError::InvalidKnowledgeNamespaceCount);
            }
        }
        KnowledgeLibraryProviderV1::GraphRag if !binding.namespaces.is_empty() => {
            return Err(AgentTemplateError::GraphRagNamespacesUnsupported);
        }
        KnowledgeLibraryProviderV1::GraphRag => {}
    }
    for namespace in &binding.namespaces {
        if namespace.is_empty()
            || namespace.len() > 120
            || !namespace.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-' | b'/')
            })
        {
            return Err(AgentTemplateError::InvalidKnowledgeNamespace(
                namespace.clone(),
            ));
        }
    }
    if !capabilities.allows_tool("library_search") {
        return Err(AgentTemplateError::MissingLibrarySearchCapability);
    }
    Ok(())
}

fn validate_schema_shape(schema: &Value, field: &str) -> Result<(), AgentTemplateError> {
    if schema.is_object() || schema.is_boolean() {
        Ok(())
    } else {
        Err(AgentTemplateError::InvalidSchema(field.to_string()))
    }
}

fn validate_projection_shape(projection: &CapabilityProjection) -> Result<(), AgentTemplateError> {
    let scopes = [
        (
            projection.allow_all_tools,
            !projection.tools.is_empty(),
            "tools",
        ),
        (
            projection.allow_all_skills,
            !projection.skills.is_empty(),
            "skills",
        ),
        (
            projection.allow_all_plugins,
            !projection.plugins.is_empty(),
            "plugins",
        ),
        (
            projection.allow_all_mcp_servers,
            !projection.mcp_servers.is_empty(),
            "mcpServers",
        ),
        (
            projection.allow_all_workspace_roots,
            !projection.workspace_roots.is_empty(),
            "workspaceRoots",
        ),
    ];
    if let Some((_, _, scope)) = scopes
        .into_iter()
        .find(|(allow_all, has_entries, _)| *allow_all && *has_entries)
    {
        return Err(AgentTemplateError::AmbiguousCapabilityScope(
            scope.to_string(),
        ));
    }
    Ok(())
}

fn validate_model_policy_shape(policy: &AgentModelPolicyV1) -> Result<(), AgentTemplateError> {
    if policy.allow_all_models && !policy.allowed_models.is_empty() {
        Err(AgentTemplateError::AmbiguousModelPolicy)
    } else {
        Ok(())
    }
}

pub(crate) fn capabilities_with_resolved_connections(
    base: &CapabilityProjection,
    resolved: &[ResolvedConnectionBindingV1],
) -> CapabilityProjection {
    let mut capabilities = base.clone();
    for binding in resolved {
        if !capabilities.allow_all_mcp_servers {
            capabilities.mcp_servers.extend(
                binding
                    .mcp_server_ids()
                    .map(|server_id| server_id.to_string()),
            );
        }
        if !capabilities.allow_all_tools {
            capabilities
                .tools
                .extend(binding.model_tool_names().cloned());
        }
    }
    capabilities
}

pub(crate) fn capabilities_with_connection_operations(
    base: &CapabilityProjection,
    operations: &[ExecutionConnectionOperationV1],
) -> CapabilityProjection {
    let mut capabilities = base.clone();
    for operation in operations {
        if !capabilities.allow_all_mcp_servers {
            capabilities
                .mcp_servers
                .insert(operation.mcp_server_id.to_string());
        }
        if !capabilities.allow_all_tools {
            capabilities.tools.insert(operation.model_tool_name.clone());
        }
    }
    capabilities
}

fn intersect_resource_grants(
    left: &[ExecutionResourceGrantV1],
    right: &[ExecutionResourceGrantV1],
) -> Vec<ExecutionResourceGrantV1> {
    left.iter()
        .filter_map(|left_grant| {
            let right_grant = right.iter().find(|right_grant| {
                right_grant.binding_id == left_grant.binding_id
                    && right_grant.kind == left_grant.kind
                    && right_grant.resource == left_grant.resource
            })?;
            Some(ExecutionResourceGrantV1 {
                binding_id: left_grant.binding_id.clone(),
                kind: left_grant.kind,
                resource: left_grant.resource.clone(),
                can_read: left_grant.can_read && right_grant.can_read,
                can_write: left_grant.can_write && right_grant.can_write,
                max_data_classification: std::cmp::min(
                    left_grant.max_data_classification,
                    right_grant.max_data_classification,
                ),
            })
        })
        .filter(|grant| grant.can_read || grant.can_write)
        .collect()
}

fn resource_grants_are_subset(
    candidate: &[ExecutionResourceGrantV1],
    boundary: &[ExecutionResourceGrantV1],
) -> bool {
    candidate.iter().all(|grant| {
        boundary.iter().any(|allowed| {
            allowed.binding_id == grant.binding_id
                && allowed.kind == grant.kind
                && allowed.resource == grant.resource
                && (!grant.can_read || allowed.can_read)
                && (!grant.can_write || allowed.can_write)
                && grant.max_data_classification <= allowed.max_data_classification
        })
    })
}

fn validate_value_against_schema(
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), AgentTemplateError> {
    if schema == &Value::Bool(true) {
        return Ok(());
    }
    if schema == &Value::Bool(false) {
        return Err(AgentTemplateError::StateSchemaViolation {
            path: path.to_string(),
            message: "schema rejects every value".to_string(),
        });
    }
    let object = schema
        .as_object()
        .ok_or_else(|| AgentTemplateError::InvalidSchema(path.to_string()))?;
    if let Some(expected) = object.get("type") {
        let matches = match expected {
            Value::String(expected) => value_matches_type(value, expected),
            Value::Array(expected) => expected
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| value_matches_type(value, expected)),
            _ => false,
        };
        if !matches {
            return Err(AgentTemplateError::StateSchemaViolation {
                path: path.to_string(),
                message: format!("expected type {expected}"),
            });
        }
    }
    if let Some(allowed) = object.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            return Err(AgentTemplateError::StateSchemaViolation {
                path: path.to_string(),
                message: "value is not in enum".to_string(),
            });
        }
    }
    if let Some(value_object) = value.as_object() {
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !value_object.contains_key(field) {
                    return Err(AgentTemplateError::StateSchemaViolation {
                        path: path.to_string(),
                        message: format!("missing required property `{field}`"),
                    });
                }
            }
        }
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(properties) = properties {
            for (field, field_value) in value_object {
                if let Some(field_schema) = properties.get(field) {
                    validate_value_against_schema(
                        field_schema,
                        field_value,
                        &format!("{path}.{field}"),
                    )?;
                } else if object.get("additionalProperties") == Some(&Value::Bool(false)) {
                    return Err(AgentTemplateError::StateSchemaViolation {
                        path: path.to_string(),
                        message: format!("additional property `{field}` is not allowed"),
                    });
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (object.get("items"), value.as_array()) {
        for (index, item) in values.iter().enumerate() {
            validate_value_against_schema(items, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

/// Validates a runtime value against the supported JSON Schema subset used by
/// Agent and Workflow contracts. This keeps state, output, and external
/// delivery boundaries on one validator instead of drifting into adapters.
pub fn validate_json_schema_value(
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), AgentTemplateError> {
    validate_value_against_schema(schema, value, path)
}

fn value_matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "string" => value.is_string(),
        _ => false,
    }
}

fn diff_projection(
    previous: &CapabilityProjection,
    next: &CapabilityProjection,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    diff_scope(
        "tool",
        previous.allow_all_tools,
        &previous.tools,
        next.allow_all_tools,
        &next.tools,
        changes,
    );
    diff_scope(
        "skill",
        previous.allow_all_skills,
        &previous.skills,
        next.allow_all_skills,
        &next.skills,
        changes,
    );
    diff_scope(
        "plugin",
        previous.allow_all_plugins,
        &previous.plugins,
        next.allow_all_plugins,
        &next.plugins,
        changes,
    );
    diff_scope(
        "mcp_server",
        previous.allow_all_mcp_servers,
        &previous.mcp_servers,
        next.allow_all_mcp_servers,
        &next.mcp_servers,
        changes,
    );
    let previous_roots = previous
        .workspace_roots
        .iter()
        .map(|path| workspace_root_identity(path).display().to_string())
        .collect::<BTreeSet<_>>();
    let next_roots = next
        .workspace_roots
        .iter()
        .map(|path| workspace_root_identity(path).display().to_string())
        .collect::<BTreeSet<_>>();
    diff_scope(
        "workspace_root",
        previous.allow_all_workspace_roots,
        &previous_roots,
        next.allow_all_workspace_roots,
        &next_roots,
        changes,
    );
}

fn projection_additions(projection: &CapabilityProjection, changes: &mut Vec<CapabilityChangeV1>) {
    add_scope(
        "tool",
        projection.allow_all_tools,
        &projection.tools,
        changes,
    );
    add_scope(
        "skill",
        projection.allow_all_skills,
        &projection.skills,
        changes,
    );
    add_scope(
        "plugin",
        projection.allow_all_plugins,
        &projection.plugins,
        changes,
    );
    add_scope(
        "mcp_server",
        projection.allow_all_mcp_servers,
        &projection.mcp_servers,
        changes,
    );
    let roots = projection
        .workspace_roots
        .iter()
        .map(|path| workspace_root_identity(path).display().to_string())
        .collect::<BTreeSet<_>>();
    add_scope(
        "workspace_root",
        projection.allow_all_workspace_roots,
        &roots,
        changes,
    );
}

fn connection_grant_changes(
    previous: &[ConnectionBindingV1],
    next: &[ConnectionBindingV1],
    changes: &mut Vec<CapabilityChangeV1>,
) {
    changes.extend(
        diff_connection_bindings(previous, next)
            .into_iter()
            .map(|change| CapabilityChangeV1 {
                scope: if change.operation_id.is_some() {
                    "connection_operation".to_string()
                } else {
                    "connection".to_string()
                },
                value: change
                    .operation_id
                    .unwrap_or_else(|| change.connection_id.to_string()),
                kind: match change.kind {
                    ConnectionGrantChangeKind::Added => CapabilityChangeKindV1::Added,
                    ConnectionGrantChangeKind::Removed => CapabilityChangeKindV1::Removed,
                },
            }),
    );
}

fn knowledge_binding_changes(
    previous: Option<&AgentKnowledgeBindingV1>,
    next: Option<&AgentKnowledgeBindingV1>,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    let previous_provider = previous.map(|binding| binding.provider);
    let next_provider = next.map(|binding| binding.provider);
    if previous_provider != next_provider {
        if let Some(provider) = next_provider {
            changes.push(CapabilityChangeV1 {
                scope: "knowledge_provider".to_string(),
                value: knowledge_provider_id(provider).to_string(),
                kind: CapabilityChangeKindV1::Added,
            });
        }
        if let Some(provider) = previous_provider {
            changes.push(CapabilityChangeV1 {
                scope: "knowledge_provider".to_string(),
                value: knowledge_provider_id(provider).to_string(),
                kind: CapabilityChangeKindV1::Removed,
            });
        }
    }
    let empty = BTreeSet::new();
    diff_scope(
        "knowledge_namespace",
        false,
        previous
            .map(|binding| &binding.namespaces)
            .unwrap_or(&empty),
        false,
        next.map(|binding| &binding.namespaces).unwrap_or(&empty),
        changes,
    );
}

fn knowledge_provider_id(provider: KnowledgeLibraryProviderV1) -> &'static str {
    match provider {
        KnowledgeLibraryProviderV1::Sag => "sag",
        KnowledgeLibraryProviderV1::GraphRag => "graph-rag",
    }
}

fn diff_scope(
    scope: &str,
    previous_all: bool,
    previous: &BTreeSet<String>,
    next_all: bool,
    next: &BTreeSet<String>,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    if previous_all != next_all {
        changes.push(CapabilityChangeV1 {
            scope: scope.to_string(),
            value: "*".to_string(),
            kind: if next_all {
                CapabilityChangeKindV1::Expanded
            } else {
                CapabilityChangeKindV1::Reduced
            },
        });
        if previous_all || next_all {
            return;
        }
    }
    for value in next.difference(previous) {
        changes.push(CapabilityChangeV1 {
            scope: scope.to_string(),
            value: value.clone(),
            kind: CapabilityChangeKindV1::Added,
        });
    }
    for value in previous.difference(next) {
        changes.push(CapabilityChangeV1 {
            scope: scope.to_string(),
            value: value.clone(),
            kind: CapabilityChangeKindV1::Removed,
        });
    }
}

fn add_scope(
    scope: &str,
    allow_all: bool,
    values: &BTreeSet<String>,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    if allow_all {
        changes.push(CapabilityChangeV1 {
            scope: scope.to_string(),
            value: "*".to_string(),
            kind: CapabilityChangeKindV1::Expanded,
        });
    } else {
        for value in values {
            changes.push(CapabilityChangeV1 {
                scope: scope.to_string(),
                value: value.clone(),
                kind: CapabilityChangeKindV1::Added,
            });
        }
    }
}

fn diff_models(
    previous: &AgentModelPolicyV1,
    next: &AgentModelPolicyV1,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    let previous_values = previous
        .allowed_models
        .iter()
        .map(|binding| format!("{}:{}", binding.provider_id, binding.model_id))
        .collect::<BTreeSet<_>>();
    let next_values = next
        .allowed_models
        .iter()
        .map(|binding| format!("{}:{}", binding.provider_id, binding.model_id))
        .collect::<BTreeSet<_>>();
    diff_scope(
        "model",
        previous.allow_all_models,
        &previous_values,
        next.allow_all_models,
        &next_values,
        changes,
    );
}

fn model_additions(policy: &AgentModelPolicyV1, changes: &mut Vec<CapabilityChangeV1>) {
    let values = policy
        .allowed_models
        .iter()
        .map(|binding| format!("{}:{}", binding.provider_id, binding.model_id))
        .collect::<BTreeSet<_>>();
    add_scope("model", policy.allow_all_models, &values, changes);
}

fn diff_resource_grants(
    previous: &[ExecutionResourceGrantV1],
    next: &[ExecutionResourceGrantV1],
    changes: &mut Vec<CapabilityChangeV1>,
) {
    let previous = previous
        .iter()
        .map(|grant| (&grant.binding_id, grant))
        .collect::<BTreeMap<_, _>>();
    let next = next
        .iter()
        .map(|grant| (&grant.binding_id, grant))
        .collect::<BTreeMap<_, _>>();
    for (id, grant) in &next {
        match previous.get(id) {
            None => changes.push(CapabilityChangeV1 {
                scope: "resource".to_string(),
                value: (*id).clone(),
                kind: CapabilityChangeKindV1::Added,
            }),
            Some(previous_grant) if *previous_grant != *grant => {
                let expanded = (grant.can_read && !previous_grant.can_read)
                    || (grant.can_write && !previous_grant.can_write)
                    || grant.max_data_classification > previous_grant.max_data_classification
                    || grant.kind != previous_grant.kind
                    || grant.resource != previous_grant.resource;
                changes.push(CapabilityChangeV1 {
                    scope: "resource".to_string(),
                    value: (*id).clone(),
                    kind: if expanded {
                        CapabilityChangeKindV1::Expanded
                    } else {
                        CapabilityChangeKindV1::Reduced
                    },
                });
            }
            _ => {}
        }
    }
    for id in previous.keys().filter(|id| !next.contains_key(*id)) {
        changes.push(CapabilityChangeV1 {
            scope: "resource".to_string(),
            value: (*id).clone(),
            kind: CapabilityChangeKindV1::Removed,
        });
    }
}

fn diff_delegates(
    previous: &AgentTemplateSpecV1,
    next: &AgentTemplateSpecV1,
    changes: &mut Vec<CapabilityChangeV1>,
) {
    diff_scope(
        "delegate",
        previous.allow_all_delegates,
        &previous.delegate_template_ids,
        next.allow_all_delegates,
        &next.delegate_template_ids,
        changes,
    );
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
    use crate::{ExecutionConnectionOperationV1, OperationGrantV1};

    fn template_spec(tools: &[&str], models: &[(&str, &str)]) -> AgentTemplateSpecV1 {
        AgentTemplateSpecV1 {
            description: "Test Agent".to_string(),
            instructions: "Only perform the assigned test task.".to_string(),
            capabilities: CapabilityProjection::only_tools(tools.iter().copied()),
            resource_grants: Vec::new(),
            model_policy: AgentModelPolicyV1::only(models.iter().map(|(provider_id, model_id)| {
                AgentModelBindingV1 {
                    provider_id: (*provider_id).to_string(),
                    model_id: (*model_id).to_string(),
                }
            })),
            state_schema: serde_json::json!({
                "type": "object",
                "required": ["caseId"],
                "properties": { "caseId": { "type": "string" } },
                "additionalProperties": false
            }),
            output_schema: serde_json::json!({ "type": "object" }),
            allow_all_delegates: false,
            delegate_template_ids: BTreeSet::new(),
            budget: AgentBudgetV1::default(),
            risk_class: AgentRiskClassV1::Medium,
            connection_bindings: Vec::new(),
            knowledge_binding: None,
        }
    }

    fn published_template(
        id: &str,
        version: u32,
        spec: AgentTemplateSpecV1,
    ) -> AgentTemplateVersionV1 {
        AgentTemplateVersionV1::new_draft(id, version, id, "owner", spec)
            .unwrap()
            .publish("owner", None, true)
            .unwrap()
            .0
    }

    #[test]
    fn knowledge_binding_derives_library_tool_and_is_a_capability_diff() {
        let namespace = "opentopia.audit.work-injury.v1".to_string();
        let mut first_spec = template_spec(&[], &[("provider", "model")]);
        first_spec.knowledge_binding = Some(AgentKnowledgeBindingV1 {
            provider: KnowledgeLibraryProviderV1::Sag,
            namespaces: BTreeSet::from([namespace.clone()]),
        });
        let first = published_template("audit", 1, first_spec);
        assert!(first.spec.capabilities.allows_tool("library_search"));
        let serialized = serde_json::to_value(&first).expect("serialize legacy-compatible SAG");
        assert!(serialized["spec"]["knowledgeBinding"]
            .get("provider")
            .is_none());
        let restored: AgentTemplateVersionV1 =
            serde_json::from_value(serialized).expect("restore SAG template");
        restored.validate().expect("validate restored SAG template");
        assert_eq!(restored.content_hash, restored.calculate_content_hash());
        let mut next_spec = first.spec.clone();
        next_spec
            .knowledge_binding
            .as_mut()
            .unwrap()
            .namespaces
            .insert("opentopia.audit.shared.v1".to_string());
        let next =
            AgentTemplateVersionV1::new_draft("audit", 2, "audit", "owner", next_spec).unwrap();
        let diff = AgentTemplateDiffV1::between(Some(&first), &next);
        assert!(diff.widens_capabilities);
        assert!(diff.changes.iter().any(|change| {
            change.scope == "knowledge_namespace"
                && change.value == "opentopia.audit.shared.v1"
                && change.kind == CapabilityChangeKindV1::Added
        }));
    }

    #[test]
    fn graph_rag_binding_derives_access_without_binding_a_database() {
        let mut spec = template_spec(&[], &[("provider", "model")]);
        spec.knowledge_binding = Some(AgentKnowledgeBindingV1 {
            provider: KnowledgeLibraryProviderV1::GraphRag,
            namespaces: BTreeSet::new(),
        });

        let template = AgentTemplateVersionV1::new_draft("techqa", 1, "TechQA", "owner", spec)
            .expect("Graph RAG Agent template");

        assert!(template.spec.capabilities.allows_tool("library_search"));
        assert_eq!(
            template
                .spec
                .knowledge_binding
                .as_ref()
                .map(|binding| binding.provider),
            Some(KnowledgeLibraryProviderV1::GraphRag)
        );
        assert_eq!(
            serde_json::to_value(&template).expect("serialize template")["spec"]
                ["knowledgeBinding"]["provider"],
            serde_json::json!("graph-rag")
        );
    }

    #[test]
    fn graph_rag_binding_rejects_sag_namespaces() {
        let mut spec = template_spec(&[], &[("provider", "model")]);
        spec.knowledge_binding = Some(AgentKnowledgeBindingV1 {
            provider: KnowledgeLibraryProviderV1::GraphRag,
            namespaces: BTreeSet::from(["sag.only".to_string()]),
        });

        assert_eq!(
            AgentTemplateVersionV1::new_draft("techqa", 1, "TechQA", "owner", spec).unwrap_err(),
            AgentTemplateError::GraphRagNamespacesUnsupported
        );
    }

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
    fn workspace_projection_uses_canonical_path_identity() {
        let canonical = std::env::current_dir().unwrap().canonicalize().unwrap();
        let lexical = std::env::current_dir().unwrap().join(".");
        let mut left = CapabilityProjection::deny_all();
        left.workspace_roots.insert(lexical.clone());
        let mut right = CapabilityProjection::deny_all();
        right.workspace_roots.insert(canonical.clone());

        let intersection = left.intersect(&right);
        let union = left.union(&right);

        assert!(left.allows_workspace_root(&canonical));
        assert!(right.allows_workspace_root(&lexical));
        assert!(intersection.allows_workspace_root(&canonical));
        assert!(union.allows_workspace_root(&lexical));
        assert_eq!(union.workspace_roots.len(), 1);
        assert!(left.is_subset_of(&right));
        assert!(right.is_subset_of(&left));
    }

    #[test]
    fn flow_profile_is_enterprise_only_and_inherits_full_capabilities() {
        let profile = ExperienceSurfaceProfile::for_mode(ExperienceMode::Flow);
        assert!(profile.enterprise_only);
        assert!(profile.capabilities.allows_tool("read_file"));
        assert!(profile.capabilities.allows_tool("read_attachment"));
        assert!(profile.capabilities.allows_tool("spreadsheet"));
        assert!(profile.capabilities.allows_tool("shell"));
        assert!(profile.capabilities.allows_tool("write_file"));
        assert!(profile.capabilities.allows_tool("flow_run"));
        assert!(profile.capabilities.allows_tool("flow_status"));
        assert!(profile.capabilities.allows_plugin("browser-automation"));
        assert!(profile.capabilities.allows_mcp_server("server-1"));
        assert!(profile.capabilities.allow_all_workspace_roots);
        assert!(ExperienceSurfaceProfile::flow_control_tools()
            .iter()
            .all(|tool| profile.capabilities.allows_tool(tool)));
    }

    #[test]
    fn unbound_flow_baseline_keeps_workspace_identity_without_widening_tools() {
        let workspace_root = PathBuf::from("C:/workspace");
        let capabilities = ExperienceSurfaceProfile::flow_runtime_baseline(workspace_root.clone());

        assert!(capabilities.allows_workspace_root(&workspace_root));
        assert!(capabilities.allows_tool("flow_run"));
        assert!(!capabilities.allows_tool("shell"));
        assert!(!capabilities.allow_all_plugins);
        assert!(!capabilities.allow_all_mcp_servers);
    }

    #[test]
    fn flow_harness_ceiling_adds_identity_without_widening_node_projection() {
        let workspace_root = PathBuf::from("C:/workspace");
        let node_capabilities = CapabilityProjection::deny_all();
        let harness = ExperienceSurfaceProfile::flow_harness_capabilities(
            workspace_root.clone(),
            &node_capabilities,
        );

        assert!(harness.allows_workspace_root(&workspace_root));
        assert!(harness.allows_tool("flow_run"));
        assert!(!harness.allows_tool("shell"));
        assert!(!node_capabilities.allows_workspace_root(&workspace_root));
        assert!(!node_capabilities.allows_tool("flow_run"));
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

    #[test]
    fn publishing_requires_owner_and_expansion_approval() {
        let first = AgentTemplateVersionV1::new_draft(
            "finance-reviewer",
            1,
            "Finance reviewer",
            "finance-platform",
            template_spec(&["read_file"], &[("openai", "gpt-enterprise")]),
        )
        .unwrap();
        assert_eq!(
            first.publish("other-owner", None, true).unwrap_err(),
            AgentTemplateError::OwnerApprovalRequired
        );
        assert_eq!(
            first.publish("finance-platform", None, false).unwrap_err(),
            AgentTemplateError::CapabilityExpansionApprovalRequired
        );
        let (published, diff) = first.publish("finance-platform", None, true).unwrap();
        assert_eq!(published.status, AgentTemplateStatusV1::Published);
        assert!(diff.widens_capabilities);
        assert_eq!(published.content_hash, first.content_hash);
    }

    #[test]
    fn legacy_template_hash_survives_missing_connection_bindings_field() {
        let template = published_template(
            "legacy-worker",
            1,
            template_spec(&["read_file"], &[("openai", "gpt-enterprise")]),
        );
        let document = serde_json::to_value(&template).expect("serialize legacy template");
        assert!(document["spec"].get("connectionBindings").is_none());

        let restored: AgentTemplateVersionV1 =
            serde_json::from_value(document).expect("deserialize legacy template");

        assert_eq!(restored.content_hash, restored.calculate_content_hash());
        restored.validate().expect("validate restored template");
    }

    #[test]
    fn structured_and_legacy_connection_grants_cannot_be_mixed() {
        let connection_id = Uuid::new_v4();
        let mut spec = template_spec(&["read_file"], &[("openai", "gpt-enterprise")]);
        spec.capabilities.allow_all_mcp_servers = false;
        spec.capabilities
            .mcp_servers
            .insert(connection_id.to_string());
        spec.connection_bindings = vec![ConnectionBindingV1 {
            connection_id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: "operation-1".to_string(),
            }],
        }];

        assert_eq!(
            AgentTemplateVersionV1::new_draft("mixed", 1, "Mixed", "owner", spec).unwrap_err(),
            AgentTemplateError::AmbiguousConnectionGrantPolicy
        );
    }

    #[test]
    fn connection_revision_change_is_a_capability_expansion() {
        let connection_id = Uuid::new_v4();
        let mut first_spec = template_spec(&["read_file"], &[("openai", "gpt-enterprise")]);
        first_spec.capabilities.allow_all_mcp_servers = false;
        first_spec.connection_bindings = vec![ConnectionBindingV1 {
            connection_id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: "operation-1".to_string(),
            }],
        }];
        let mut second_spec = first_spec.clone();
        second_spec.connection_bindings[0].capability_revision = 2;
        let previous = published_template("revisioned", 1, first_spec);
        let next =
            AgentTemplateVersionV1::new_draft("revisioned", 2, "revisioned", "owner", second_spec)
                .unwrap();

        assert!(AgentTemplateDiffV1::between(Some(&previous), &next).widens_capabilities);
    }

    #[test]
    fn tampered_instance_connection_revision_fails_closed() {
        let connection_id = Uuid::new_v4();
        let operation_id = "operation-1".to_string();
        let mut spec = template_spec(&["read_file"], &[("openai", "gpt-enterprise")]);
        spec.capabilities.allow_all_mcp_servers = false;
        spec.connection_bindings = vec![ConnectionBindingV1 {
            connection_id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: operation_id.clone(),
            }],
        }];
        let template = published_template("bound-worker", 1, spec);
        let operation = ExecutionConnectionOperationV1 {
            connection_id,
            capability_revision: 1,
            operation_id: operation_id.clone(),
            mcp_server_id: Uuid::new_v4(),
            provider_tool_name: "read".to_string(),
            model_tool_name: "mcp_read_1234567890abcdef".to_string(),
            pinned_operation_fingerprint: "sha256:test".to_string(),
        };
        let resolved = vec![ResolvedConnectionBindingV1 {
            binding: template.spec.connection_bindings[0].clone(),
            operations: BTreeMap::from([(operation_id, operation)]),
        }];
        let mut instance = AgentInstanceV1::instantiate_with_connections(
            &template,
            Uuid::new_v4(),
            ExperienceMode::Flow,
            &CapabilityProjection::unrestricted(),
            None,
            None,
            None,
            None,
            None,
            None,
            &resolved,
            serde_json::json!({"caseId": "one"}),
        )
        .unwrap();
        instance.execution_context.connection_bindings[0].capability_revision = 2;

        assert_eq!(
            instance
                .validate_execution_boundary_with_connections(
                    &template,
                    &CapabilityProjection::unrestricted(),
                    &resolved,
                )
                .unwrap_err(),
            AgentTemplateError::InstanceCapabilityViolation
        );
    }

    #[test]
    fn requested_connection_revision_mismatch_drops_routes_and_model_capabilities() {
        let connection_id = Uuid::new_v4();
        let mcp_server_id = Uuid::new_v4();
        let operation_id = "operation-1".to_string();
        let model_tool_name = "mcp_read_1234567890abcdef".to_string();
        let mut spec = template_spec(&["read_file"], &[("openai", "gpt-enterprise")]);
        spec.capabilities.allow_all_mcp_servers = false;
        spec.connection_bindings = vec![ConnectionBindingV1 {
            connection_id,
            capability_revision: 1,
            operation_grants: vec![OperationGrantV1 {
                operation_id: operation_id.clone(),
            }],
        }];
        let template = published_template("requested-revision", 1, spec);
        let resolved = vec![ResolvedConnectionBindingV1 {
            binding: template.spec.connection_bindings[0].clone(),
            operations: BTreeMap::from([(
                operation_id.clone(),
                ExecutionConnectionOperationV1 {
                    connection_id,
                    capability_revision: 1,
                    operation_id: operation_id.clone(),
                    mcp_server_id,
                    provider_tool_name: "read".to_string(),
                    model_tool_name: model_tool_name.clone(),
                    pinned_operation_fingerprint: "sha256:test".to_string(),
                },
            )]),
        }];
        let requested = vec![ConnectionBindingV1 {
            connection_id,
            capability_revision: 2,
            operation_grants: vec![OperationGrantV1 { operation_id }],
        }];

        let instance = AgentInstanceV1::instantiate_with_connections(
            &template,
            Uuid::new_v4(),
            ExperienceMode::Flow,
            &CapabilityProjection::unrestricted(),
            None,
            None,
            None,
            None,
            None,
            Some(&requested),
            &resolved,
            serde_json::json!({"caseId": "one"}),
        )
        .expect("instantiate with mismatched requested revision");

        assert!(instance.execution_context.connection_bindings.is_empty());
        assert!(instance.execution_context.connection_operations.is_empty());
        assert!(!instance
            .execution_context
            .capabilities
            .allows_tool(&model_tool_name));
        assert!(!instance
            .execution_context
            .capabilities
            .allows_mcp_server(&mcp_server_id.to_string()));
    }

    #[test]
    fn instances_are_isolated_and_state_schema_is_enforced() {
        let template = published_template(
            "case-worker",
            1,
            template_spec(&["read_file"], &[("openai", "gpt-enterprise")]),
        );
        let thread_id = Uuid::new_v4();
        let first = AgentInstanceV1::instantiate(
            &template,
            thread_id,
            ExperienceMode::Code,
            &CapabilityProjection::unrestricted(),
            None,
            None,
            None,
            None,
            None,
            serde_json::json!({"caseId": "one"}),
        )
        .unwrap();
        let second = AgentInstanceV1::instantiate(
            &template,
            thread_id,
            ExperienceMode::Code,
            &CapabilityProjection::unrestricted(),
            None,
            None,
            None,
            None,
            None,
            serde_json::json!({"caseId": "two"}),
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert_ne!(first.state, second.state);
        assert_eq!(
            AgentInstanceV1::instantiate(
                &template,
                Uuid::new_v4(),
                ExperienceMode::Code,
                &CapabilityProjection::unrestricted(),
                None,
                None,
                None,
                None,
                None,
                serde_json::json!({}),
            )
            .unwrap_err(),
            AgentTemplateError::StateSchemaViolation {
                path: "$state".to_string(),
                message: "missing required property `caseId`".to_string(),
            }
        );
    }

    #[test]
    fn delegated_instances_can_only_narrow_parent_capabilities() {
        let mut parent_spec = template_spec(&["read_file"], &[("openai", "gpt-enterprise")]);
        parent_spec
            .delegate_template_ids
            .insert("child-worker".to_string());
        parent_spec.resource_grants = vec![ExecutionResourceGrantV1 {
            binding_id: "finance".to_string(),
            kind: ResourceKind::Database,
            resource: "ledger_view".to_string(),
            can_read: true,
            can_write: false,
            max_data_classification: DataClassification::Confidential,
        }];
        let parent_template = published_template("parent-worker", 1, parent_spec);

        let mut child_spec = template_spec(
            &["read_file", "write_file", "shell"],
            &[("openai", "gpt-enterprise"), ("other", "external-model")],
        );
        child_spec.resource_grants = vec![ExecutionResourceGrantV1 {
            binding_id: "finance".to_string(),
            kind: ResourceKind::Database,
            resource: "ledger_view".to_string(),
            can_read: true,
            can_write: true,
            max_data_classification: DataClassification::Restricted,
        }];
        let child_template = published_template("child-worker", 1, child_spec);
        let thread_id = Uuid::new_v4();
        let parent = AgentInstanceV1::instantiate(
            &parent_template,
            thread_id,
            ExperienceMode::Code,
            &CapabilityProjection::unrestricted(),
            None,
            None,
            None,
            None,
            None,
            serde_json::json!({"caseId": "parent"}),
        )
        .unwrap();
        let child = AgentInstanceV1::instantiate(
            &child_template,
            thread_id,
            ExperienceMode::Code,
            &CapabilityProjection::unrestricted(),
            Some(&parent),
            Some(&parent_template),
            None,
            None,
            None,
            serde_json::json!({"caseId": "child"}),
        )
        .unwrap();
        assert!(child
            .execution_context
            .capabilities
            .allows_tool("read_file"));
        assert!(!child.execution_context.capabilities.allows_tool("shell"));
        assert!(!child
            .execution_context
            .capabilities
            .allows_tool("write_file"));
        assert!(child
            .execution_context
            .model_policy
            .allows("openai", "gpt-enterprise"));
        assert!(!child
            .execution_context
            .model_policy
            .allows("other", "external-model"));
        assert_eq!(child.execution_context.resource_grants.len(), 1);
        assert!(child.execution_context.resource_grants[0].can_read);
        assert!(!child.execution_context.resource_grants[0].can_write);
        assert_eq!(
            child.execution_context.resource_grants[0].max_data_classification,
            DataClassification::Confidential
        );
        child
            .validate_execution_boundary(&child_template, &CapabilityProjection::unrestricted())
            .unwrap();
        let mut tampered = child.clone();
        tampered.execution_context.capabilities = CapabilityProjection::unrestricted();
        assert_eq!(
            tampered
                .validate_execution_boundary(
                    &child_template,
                    &CapabilityProjection::unrestricted(),
                )
                .unwrap_err(),
            AgentTemplateError::InstanceCapabilityViolation
        );
    }
}
