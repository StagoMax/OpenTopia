use super::{AgentSpawnPolicy, CollaborationDomainError};
use crate::RuntimeConnectionAuthorityV1;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RUNTIME_SNAPSHOT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeWorkspaceModeV1 {
    SharedReadOnly,
    SharedCoordinated,
    IsolatedWorktree,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeWorkspaceDeliveryStateV1 {
    Pending,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeWorkspaceAssignmentV1 {
    SharedReadOnly {
        root: PathBuf,
    },
    SharedCoordinated {
        root: PathBuf,
    },
    IsolatedWorktree {
        repository_root: PathBuf,
        root: PathBuf,
        branch: String,
        base_commit: String,
        delivery_state: RuntimeWorkspaceDeliveryStateV1,
    },
}

impl RuntimeWorkspaceAssignmentV1 {
    pub fn shared(mode: RuntimeWorkspaceModeV1, root: PathBuf) -> Self {
        match mode {
            RuntimeWorkspaceModeV1::SharedReadOnly => Self::SharedReadOnly { root },
            RuntimeWorkspaceModeV1::SharedCoordinated => Self::SharedCoordinated { root },
            RuntimeWorkspaceModeV1::IsolatedWorktree => {
                unreachable!("isolated worktrees require a complete assignment")
            }
        }
    }

    pub fn mode(&self) -> RuntimeWorkspaceModeV1 {
        match self {
            Self::SharedReadOnly { .. } => RuntimeWorkspaceModeV1::SharedReadOnly,
            Self::SharedCoordinated { .. } => RuntimeWorkspaceModeV1::SharedCoordinated,
            Self::IsolatedWorktree { .. } => RuntimeWorkspaceModeV1::IsolatedWorktree,
        }
    }

    pub fn root(&self) -> &Path {
        match self {
            Self::SharedReadOnly { root }
            | Self::SharedCoordinated { root }
            | Self::IsolatedWorktree { root, .. } => root,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeForkTurnsLabelV1 {
    None,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(untagged)]
pub enum RuntimeForkTurnsV1 {
    Label(RuntimeForkTurnsLabelV1),
    Count { count: usize },
}

/// The validated collaboration runtime contract stored in snapshot JSON.
///
/// Security-relevant fields are strongly typed. Provider, plugin, and tool
/// descriptors remain opaque because their owners validate them when consumed;
/// keeping them here preserves the frozen snapshot without creating a reverse
/// dependency from the collaboration domain into every contribution subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshotV1 {
    #[schemars(range(min = 1, max = 1))]
    pub schema_version: u16,
    pub agent_type: String,
    pub allowed_agent_types: Vec<String>,
    #[serde(default)]
    pub agent_profiles: Vec<Value>,
    pub workspace_root: PathBuf,
    pub workspace_mode: RuntimeWorkspaceModeV1,
    pub workspace_assignment: RuntimeWorkspaceAssignmentV1,
    pub git_base_commit: Option<String>,
    pub fork_turns: RuntimeForkTurnsV1,
    pub provider: Option<Value>,
    pub permission_mode: Option<Value>,
    pub sandbox: Option<Value>,
    pub agent_runtime: Option<Value>,
    pub capability_projection: Option<Value>,
    pub connection_authority: RuntimeConnectionAuthorityV1,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tool_catalog: Vec<Value>,
    #[serde(default)]
    pub plugin_contributions: Vec<Value>,
    #[serde(default)]
    pub attachment_references: Vec<Value>,
    pub spawn_policy: AgentSpawnPolicy,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl RuntimeSnapshotV1 {
    pub fn decode(value: &Value) -> Result<Self, CollaborationDomainError> {
        let mut normalized = value
            .as_object()
            .cloned()
            .ok_or_else(|| invalid_snapshot("runtime snapshot must be a JSON object"))?;
        let legacy = !normalized.contains_key("schemaVersion");
        if legacy {
            upgrade_legacy_snapshot(&mut normalized)?;
        }
        normalize_missing_connection_authority(&mut normalized);
        let snapshot: Self =
            serde_json::from_value(Value::Object(normalized)).map_err(|error| {
                invalid_snapshot(format!("runtime snapshot V1 is malformed: {error}"))
            })?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn encode(&self) -> Result<Value, CollaborationDomainError> {
        self.validate()?;
        serde_json::to_value(self).map_err(|error| {
            invalid_snapshot(format!("runtime snapshot V1 could not be encoded: {error}"))
        })
    }

    pub fn validate(&self) -> Result<(), CollaborationDomainError> {
        if self.schema_version != RUNTIME_SNAPSHOT_SCHEMA_VERSION {
            return Err(invalid_snapshot(format!(
                "unsupported runtime snapshot schema version: {}",
                self.schema_version
            )));
        }
        if self.agent_type.trim().is_empty() {
            return Err(invalid_snapshot("agentType cannot be empty"));
        }
        if self
            .allowed_agent_types
            .iter()
            .any(|item| item.trim().is_empty())
        {
            return Err(invalid_snapshot(
                "allowedAgentTypes cannot contain an empty agent type",
            ));
        }
        if self.workspace_root.as_os_str().is_empty() {
            return Err(invalid_snapshot("workspaceRoot cannot be empty"));
        }
        if self.workspace_assignment.mode() != self.workspace_mode {
            return Err(invalid_snapshot(
                "workspaceAssignment mode does not match workspaceMode",
            ));
        }
        if self.workspace_assignment.root() != self.workspace_root {
            return Err(invalid_snapshot(
                "workspaceAssignment root does not match workspaceRoot",
            ));
        }
        if self.spawn_policy.max_depth == 0 {
            return Err(invalid_snapshot("spawnPolicy.maxDepth must be positive"));
        }
        if !self.spawn_policy.allow_child_spawns && self.spawn_policy.max_direct_children != 0 {
            return Err(invalid_snapshot(
                "spawnPolicy.maxDirectChildren must be zero when child spawns are disabled",
            ));
        }
        if self.spawn_policy.allow_child_spawns && self.spawn_policy.max_direct_children == 0 {
            return Err(invalid_snapshot(
                "spawnPolicy.maxDirectChildren must be positive when child spawns are enabled",
            ));
        }
        for (name, value) in [
            ("provider", self.provider.as_ref()),
            ("permissionMode", self.permission_mode.as_ref()),
            ("sandbox", self.sandbox.as_ref()),
            ("capabilityProjection", self.capability_projection.as_ref()),
        ] {
            if value.is_none() {
                return Err(invalid_snapshot(format!(
                    "runtime snapshot is missing required security field `{name}`"
                )));
            }
        }
        if let RuntimeConnectionAuthorityV1::Structured { operations } = &self.connection_authority
        {
            let mut operation_routes = std::collections::BTreeSet::new();
            let mut model_tool_names = std::collections::BTreeSet::new();
            for operation in operations {
                if operation.connection_id.is_nil()
                    || operation.mcp_server_id.is_nil()
                    || operation.capability_revision == 0
                    || operation.operation_id.trim().is_empty()
                    || operation.provider_tool_name.trim().is_empty()
                    || operation.model_tool_name.trim().is_empty()
                    || operation.pinned_operation_fingerprint.trim().is_empty()
                {
                    return Err(invalid_snapshot(
                        "structured Connection authority contains an invalid operation",
                    ));
                }
                if !operation_routes
                    .insert((operation.connection_id, operation.operation_id.as_str()))
                {
                    return Err(invalid_snapshot(
                        "structured Connection authority contains a duplicate operation",
                    ));
                }
                if !model_tool_names.insert(operation.model_tool_name.as_str()) {
                    return Err(invalid_snapshot(
                        "structured Connection authority contains a duplicate model tool name",
                    ));
                }
            }
        }
        if let RuntimeWorkspaceAssignmentV1::IsolatedWorktree {
            repository_root,
            branch,
            base_commit,
            ..
        } = &self.workspace_assignment
        {
            if repository_root.as_os_str().is_empty()
                || branch.trim().is_empty()
                || base_commit.trim().is_empty()
            {
                return Err(invalid_snapshot(
                    "isolated workspace assignment requires repositoryRoot, branch, and baseCommit",
                ));
            }
            if self.git_base_commit.as_deref() != Some(base_commit.as_str()) {
                return Err(invalid_snapshot(
                    "isolated workspace assignment baseCommit does not match gitBaseCommit",
                ));
            }
        }
        Ok(())
    }
}

fn normalize_missing_connection_authority(snapshot: &mut Map<String, Value>) {
    if snapshot.contains_key("connectionAuthority") {
        return;
    }
    let legacy_mcp = snapshot
        .get("capabilityProjection")
        .and_then(Value::as_object)
        .is_some_and(|projection| {
            projection
                .get("allowAllMcpServers")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || projection
                    .get("mcpServers")
                    .and_then(Value::as_array)
                    .is_some_and(|servers| !servers.is_empty())
        });
    snapshot.insert(
        "connectionAuthority".to_string(),
        serde_json::json!({
            "mode": if legacy_mcp { "legacy_mcp" } else { "deny_all" }
        }),
    );
}

fn upgrade_legacy_snapshot(
    snapshot: &mut Map<String, Value>,
) -> Result<(), CollaborationDomainError> {
    snapshot.insert(
        "schemaVersion".to_string(),
        Value::from(RUNTIME_SNAPSHOT_SCHEMA_VERSION),
    );
    let agent_type = snapshot
        .get("agentType")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_string();
    snapshot
        .entry("agentType".to_string())
        .or_insert_with(|| Value::String(agent_type.clone()));
    snapshot
        .entry("allowedAgentTypes".to_string())
        .or_insert_with(|| Value::Array(vec![Value::String(agent_type)]));
    for field in [
        "agentProfiles",
        "tools",
        "toolCatalog",
        "pluginContributions",
        "attachmentReferences",
    ] {
        snapshot
            .entry(field.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    snapshot
        .entry("forkTurns".to_string())
        .or_insert_with(|| Value::String("all".to_string()));
    snapshot
        .entry("spawnPolicy".to_string())
        .or_insert_with(|| {
            serde_json::json!({
                "allowChildSpawns": false,
                "maxDepth": 1,
                "maxDirectChildren": 0,
            })
        });

    let workspace_root = snapshot
        .get("workspaceRoot")
        .and_then(Value::as_str)
        .filter(|root| !root.is_empty())
        .ok_or_else(|| invalid_snapshot("legacy runtime snapshot is missing workspaceRoot"))?
        .to_string();
    let workspace_mode = snapshot
        .get("workspaceMode")
        .and_then(Value::as_str)
        .unwrap_or("shared_read_only")
        .to_string();
    snapshot
        .entry("workspaceMode".to_string())
        .or_insert_with(|| Value::String(workspace_mode.clone()));
    if !snapshot.contains_key("workspaceAssignment") {
        let assignment = match workspace_mode.as_str() {
            "shared_read_only" | "shared_coordinated" => serde_json::json!({
                "mode": workspace_mode,
                "root": workspace_root,
            }),
            "isolated_worktree" => {
                return Err(invalid_snapshot(
                    "legacy isolated runtime snapshot is missing workspaceAssignment",
                ));
            }
            other => {
                return Err(invalid_snapshot(format!(
                    "legacy runtime snapshot has unknown workspaceMode `{other}`"
                )));
            }
        };
        snapshot.insert("workspaceAssignment".to_string(), assignment);
    }
    Ok(())
}

fn invalid_snapshot(message: impl Into<String>) -> CollaborationDomainError {
    CollaborationDomainError::InvalidRuntimeSnapshot(message.into())
}

#[cfg(test)]
pub(crate) fn test_runtime_snapshot(
    agent_type: &str,
    workspace_mode: RuntimeWorkspaceModeV1,
) -> Value {
    use crate::enterprise::CapabilityProjection;
    use crate::policy::PermissionMode;
    use crate::sandbox::LocalSandboxConfig;
    use crate::settings::ProviderSettings;

    let workspace_root = PathBuf::from("C:/workspace/project");
    let workspace_assignment = match workspace_mode {
        RuntimeWorkspaceModeV1::SharedReadOnly | RuntimeWorkspaceModeV1::SharedCoordinated => {
            RuntimeWorkspaceAssignmentV1::shared(workspace_mode, workspace_root.clone())
        }
        RuntimeWorkspaceModeV1::IsolatedWorktree => {
            RuntimeWorkspaceAssignmentV1::IsolatedWorktree {
                repository_root: workspace_root.clone(),
                root: workspace_root.clone(),
                branch: "codex/agent/fixture".to_string(),
                base_commit: "0123456789abcdef".to_string(),
                delivery_state: RuntimeWorkspaceDeliveryStateV1::Ready,
            }
        }
    };
    let mut capability_projection = CapabilityProjection::deny_all();
    capability_projection
        .workspace_roots
        .insert(workspace_root.clone());
    RuntimeSnapshotV1 {
        schema_version: RUNTIME_SNAPSHOT_SCHEMA_VERSION,
        agent_type: agent_type.to_string(),
        allowed_agent_types: vec![agent_type.to_string()],
        agent_profiles: Vec::new(),
        workspace_root,
        workspace_mode,
        workspace_assignment,
        git_base_commit: (workspace_mode == RuntimeWorkspaceModeV1::IsolatedWorktree)
            .then(|| "0123456789abcdef".to_string()),
        fork_turns: RuntimeForkTurnsV1::Label(RuntimeForkTurnsLabelV1::All),
        provider: Some(serde_json::to_value(ProviderSettings::default()).unwrap()),
        permission_mode: Some(serde_json::to_value(PermissionMode::ReadOnly).unwrap()),
        sandbox: Some(serde_json::to_value(LocalSandboxConfig::default()).unwrap()),
        agent_runtime: None,
        capability_projection: Some(serde_json::to_value(capability_projection).unwrap()),
        connection_authority: RuntimeConnectionAuthorityV1::DenyAll,
        tools: Vec::new(),
        tool_catalog: Vec::new(),
        plugin_contributions: Vec::new(),
        attachment_references: Vec::new(),
        spawn_policy: AgentSpawnPolicy::disabled(1),
        extensions: BTreeMap::new(),
    }
    .encode()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_snapshot_is_upgraded_only_when_authority_is_explicit() {
        let decoded = RuntimeSnapshotV1::decode(&json!({
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_read_only",
            "provider": { "id": "test" },
            "permissionMode": "read_only",
            "sandbox": {},
            "capabilityProjection": {
                "allowAllWorkspaceRoots": true
            }
        }))
        .unwrap();

        assert_eq!(decoded.schema_version, RUNTIME_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(
            decoded.workspace_mode,
            RuntimeWorkspaceModeV1::SharedReadOnly
        );
        assert!(!decoded.spawn_policy.allow_child_spawns);
        assert_eq!(decoded.spawn_policy.max_direct_children, 0);
        assert_eq!(
            decoded.connection_authority,
            RuntimeConnectionAuthorityV1::DenyAll
        );
    }

    #[test]
    fn old_snapshots_infer_legacy_mcp_only_from_explicit_mcp_projection() {
        let mut value = test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated);
        value
            .as_object_mut()
            .expect("snapshot object")
            .remove("connectionAuthority");
        value["capabilityProjection"] = json!({
            "allowAllMcpServers": false,
            "mcpServers": ["47f13832-e1a7-4eb4-b76a-0a5d586e159f"]
        });

        let decoded = RuntimeSnapshotV1::decode(&value).expect("old V1 snapshot should decode");
        assert_eq!(
            decoded.connection_authority,
            RuntimeConnectionAuthorityV1::LegacyMcp
        );
    }

    #[test]
    fn explicitly_empty_structured_authority_is_not_coerced_to_legacy_or_deny_all() {
        let mut value = test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated);
        value["connectionAuthority"] = json!({
            "mode": "structured",
            "operations": []
        });

        let decoded = RuntimeSnapshotV1::decode(&value).expect("structured snapshot");
        assert_eq!(
            decoded.connection_authority,
            RuntimeConnectionAuthorityV1::Structured {
                operations: Vec::new()
            }
        );
    }

    #[test]
    fn legacy_snapshot_without_authority_is_rejected() {
        let error = RuntimeSnapshotV1::decode(&json!({
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_read_only"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("provider"));
    }

    #[test]
    fn explicit_v1_rejects_missing_security_fields() {
        let error = RuntimeSnapshotV1::decode(&json!({ "schemaVersion": 1 })).unwrap_err();
        assert!(error.to_string().contains("malformed"));
    }

    #[test]
    fn rejects_workspace_mode_and_assignment_mismatch() {
        let error = RuntimeSnapshotV1::decode(&json!({
            "schemaVersion": 1,
            "agentType": "default",
            "allowedAgentTypes": ["default"],
            "agentProfiles": [],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "workspaceAssignment": {
                "mode": "shared_read_only",
                "root": "C:/workspace/project"
            },
            "gitBaseCommit": null,
            "forkTurns": "all",
            "provider": null,
            "permissionMode": null,
            "sandbox": null,
            "agentRuntime": null,
            "capabilityProjection": null,
            "tools": [],
            "toolCatalog": [],
            "pluginContributions": [],
            "attachmentReferences": [],
            "spawnPolicy": {
                "allowChildSpawns": false,
                "maxDepth": 1,
                "maxDirectChildren": 0
            }
        }))
        .unwrap_err();

        assert!(error.to_string().contains("does not match workspaceMode"));
    }
}
