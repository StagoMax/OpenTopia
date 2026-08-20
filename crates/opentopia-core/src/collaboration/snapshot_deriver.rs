use super::{
    AgentRuntimeSnapshotRecord, AgentSpawnPolicy, AgentWorkspaceMode, ChildRuntimeSnapshotRequest,
    DerivedChildRuntime, ForkTurns, RuntimeForkTurnsLabelV1, RuntimeForkTurnsV1,
    RuntimeSnapshotDerivationError, RuntimeSnapshotDeriver, RuntimeSnapshotSeed,
    RuntimeWorkspaceAssignmentV1, RuntimeWorkspaceDeliveryStateV1, RuntimeWorkspaceModeV1,
};
use async_trait::async_trait;
use uuid::Uuid;

/// Production snapshot derivation policy. It starts from the already frozen
/// parent JSON and only changes lineage metadata or narrows workspace/spawn
/// capability; live global settings are deliberately never re-read here.
#[derive(Debug, Default)]
pub struct AttenuatingRuntimeSnapshotDeriver;

#[async_trait]
impl RuntimeSnapshotDeriver for AttenuatingRuntimeSnapshotDeriver {
    async fn derive_child(
        &self,
        parent: &AgentRuntimeSnapshotRecord,
        request: ChildRuntimeSnapshotRequest,
    ) -> Result<DerivedChildRuntime, RuntimeSnapshotDerivationError> {
        let mut snapshot = parent.decode().map_err(|error| {
            RuntimeSnapshotDerivationError::Rejected(format!(
                "parent runtime snapshot is invalid: {error}"
            ))
        })?;
        if !snapshot
            .allowed_agent_types
            .iter()
            .any(|agent_type| agent_type == &request.agent_type)
        {
            return Err(RuntimeSnapshotDerivationError::Rejected(format!(
                "agent type `{}` is outside the frozen parent snapshot",
                request.agent_type
            )));
        }

        if request.allow_child_spawns && !snapshot.spawn_policy.allow_child_spawns {
            return Err(RuntimeSnapshotDerivationError::Rejected(
                "the parent snapshot does not allow recursive spawn".to_string(),
            ));
        }
        let spawn_policy = if request.allow_child_spawns {
            AgentSpawnPolicy::allows_children(
                snapshot.spawn_policy.max_depth,
                snapshot.spawn_policy.max_direct_children,
            )
        } else {
            AgentSpawnPolicy::disabled(snapshot.spawn_policy.max_depth)
        };

        let workspace_mode = match request.workspace_mode {
            AgentWorkspaceMode::Auto => snapshot.workspace_mode,
            AgentWorkspaceMode::SharedReadOnly => RuntimeWorkspaceModeV1::SharedReadOnly,
            AgentWorkspaceMode::SharedCoordinated => {
                if snapshot.workspace_mode == RuntimeWorkspaceModeV1::SharedReadOnly {
                    return Err(RuntimeSnapshotDerivationError::Rejected(
                        "shared coordinated access would expand a read-only parent snapshot"
                            .to_string(),
                    ));
                }
                RuntimeWorkspaceModeV1::SharedCoordinated
            }
            AgentWorkspaceMode::IsolatedWorktree => RuntimeWorkspaceModeV1::IsolatedWorktree,
        };

        snapshot.agent_type = request.agent_type;
        snapshot.fork_turns = match request.fork_turns {
            ForkTurns::None => RuntimeForkTurnsV1::Label(RuntimeForkTurnsLabelV1::None),
            ForkTurns::All => RuntimeForkTurnsV1::Label(RuntimeForkTurnsLabelV1::All),
            ForkTurns::Count(count) => RuntimeForkTurnsV1::Count { count },
        };
        let inherited_root = snapshot.workspace_root.clone();
        let workspace_assignment = if workspace_mode == RuntimeWorkspaceModeV1::IsolatedWorktree {
            let base_commit = snapshot
                .git_base_commit
                .clone()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RuntimeSnapshotDerivationError::Rejected(
                        "isolated worktree requires a frozen Git base commit".to_string(),
                    )
                })?;
            let token = Uuid::new_v4().simple().to_string();
            let root = inherited_root
                .join(".opentopia")
                .join("worktrees")
                .join(format!("agent-{token}"));
            snapshot.workspace_root = root.clone();
            RuntimeWorkspaceAssignmentV1::IsolatedWorktree {
                repository_root: inherited_root,
                root,
                branch: format!("codex/agent/{token}"),
                base_commit: base_commit.clone(),
                delivery_state: RuntimeWorkspaceDeliveryStateV1::Pending,
            }
        } else {
            RuntimeWorkspaceAssignmentV1::shared(workspace_mode, inherited_root)
        };
        snapshot.workspace_mode = workspace_mode;
        snapshot.workspace_assignment = workspace_assignment;
        snapshot.spawn_policy = spawn_policy.clone();
        let snapshot = snapshot.encode().map_err(|error| {
            RuntimeSnapshotDerivationError::Unavailable(format!(
                "child runtime snapshot could not be encoded: {error}"
            ))
        })?;
        Ok(DerivedChildRuntime {
            runtime_snapshot: RuntimeSnapshotSeed::new(Some(parent.id), snapshot),
            spawn_policy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{
        test_runtime_snapshot, CollaborationSessionId, RuntimeSnapshotId, RuntimeWorkspaceModeV1,
    };
    use chrono::Utc;
    use serde_json::{json, Value};

    fn parent(snapshot: Value) -> AgentRuntimeSnapshotRecord {
        let workspace_mode = match snapshot.get("workspaceMode").and_then(Value::as_str) {
            Some("shared_read_only") => RuntimeWorkspaceModeV1::SharedReadOnly,
            _ => RuntimeWorkspaceModeV1::SharedCoordinated,
        };
        let mut complete = test_runtime_snapshot("default", workspace_mode);
        complete
            .as_object_mut()
            .unwrap()
            .extend(snapshot.as_object().unwrap().clone());
        AgentRuntimeSnapshotRecord {
            id: RuntimeSnapshotId::new(),
            session_id: CollaborationSessionId::new(),
            parent_snapshot_id: None,
            content_hash: "fixture".to_string(),
            snapshot: complete,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn child_snapshot_is_derived_from_frozen_parent_and_only_narrows() {
        let parent = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "tools": ["read", "write"],
            "toolCatalog": [{"name": "read"}, {"name": "write"}],
            "pluginContributions": [{"pluginId": "documents"}],
            "attachmentReferences": [{"id": "attachment-1"}],
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 3,
                "maxDirectChildren": 2
            }
        }));
        let child = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &parent,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::None,
                    workspace_mode: AgentWorkspaceMode::SharedReadOnly,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(child.runtime_snapshot.parent_snapshot_id, Some(parent.id));
        assert_eq!(
            child.runtime_snapshot.snapshot["tools"],
            json!(["read", "write"])
        );
        for inherited in ["toolCatalog", "pluginContributions", "attachmentReferences"] {
            assert_eq!(
                child.runtime_snapshot.snapshot[inherited], parent.snapshot[inherited],
                "{inherited} must be inherited from the frozen parent snapshot"
            );
        }
        assert_eq!(
            child.runtime_snapshot.snapshot["workspaceMode"],
            "shared_read_only"
        );
        assert!(!child.spawn_policy.allow_child_spawns);
    }

    #[tokio::test]
    async fn empty_structured_connection_authority_is_inherited_without_legacy_fallback() {
        let parent = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "connectionAuthority": {
                "mode": "structured",
                "operations": []
            },
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 1
            }
        }));

        let child = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &parent,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::All,
                    workspace_mode: AgentWorkspaceMode::Auto,
                    allow_child_spawns: false,
                },
            )
            .await
            .expect("derive child");
        assert_eq!(
            child.runtime_snapshot.snapshot["connectionAuthority"],
            json!({ "mode": "structured", "operations": [] })
        );
    }

    #[tokio::test]
    async fn workspace_modes_inherit_or_narrow_but_never_expand_write_authority() {
        let coordinated = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 2
            }
        }));
        let inherited = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &coordinated,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::All,
                    workspace_mode: AgentWorkspaceMode::Auto,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            inherited.runtime_snapshot.snapshot["workspaceMode"],
            "shared_coordinated"
        );
        assert_eq!(
            inherited.runtime_snapshot.snapshot["workspaceAssignment"]["root"],
            "C:/workspace/project"
        );

        let read_only = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_read_only",
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 2
            }
        }));
        let error = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &read_only,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::None,
                    workspace_mode: AgentWorkspaceMode::SharedCoordinated,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("expand a read-only"));
    }

    #[tokio::test]
    async fn isolated_worktree_snapshot_requires_a_frozen_commit_and_unique_assignment() {
        let without_commit = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 2
            }
        }));
        let error = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &without_commit,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::None,
                    workspace_mode: AgentWorkspaceMode::IsolatedWorktree,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("frozen Git base commit"));

        let with_commit = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_coordinated",
            "gitBaseCommit": "0123456789abcdef",
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 2
            }
        }));
        let first = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &with_commit,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::Count(2),
                    workspace_mode: AgentWorkspaceMode::IsolatedWorktree,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap();
        let second = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &with_commit,
                ChildRuntimeSnapshotRequest {
                    agent_type: "worker".to_string(),
                    fork_turns: ForkTurns::Count(2),
                    workspace_mode: AgentWorkspaceMode::IsolatedWorktree,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap();
        let assignment = &first.runtime_snapshot.snapshot["workspaceAssignment"];
        assert_eq!(assignment["mode"], "isolated_worktree");
        assert_eq!(assignment["repositoryRoot"], "C:/workspace/project");
        assert_eq!(assignment["baseCommit"], "0123456789abcdef");
        assert_eq!(assignment["deliveryState"], "pending");
        assert!(assignment["branch"]
            .as_str()
            .is_some_and(|branch| branch.starts_with("codex/agent/")));
        assert_ne!(
            first.runtime_snapshot.snapshot["workspaceRoot"],
            second.runtime_snapshot.snapshot["workspaceRoot"]
        );
        assert_eq!(first.runtime_snapshot.snapshot["forkTurns"]["count"], 2);
    }

    #[tokio::test]
    async fn frozen_agent_type_allowlist_is_enforced() {
        let parent = parent(json!({
            "allowedAgentTypes": ["worker"],
            "workspaceRoot": "C:/workspace/project",
            "workspaceMode": "shared_read_only",
            "spawnPolicy": {
                "allowChildSpawns": true,
                "maxDepth": 2,
                "maxDirectChildren": 2
            }
        }));
        let error = AttenuatingRuntimeSnapshotDeriver
            .derive_child(
                &parent,
                ChildRuntimeSnapshotRequest {
                    agent_type: "admin".to_string(),
                    fork_turns: ForkTurns::None,
                    workspace_mode: AgentWorkspaceMode::SharedReadOnly,
                    allow_child_spawns: false,
                },
            )
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("outside the frozen parent snapshot"));
    }
}
