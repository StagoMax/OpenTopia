use super::{
    AgentRuntimeSnapshotRecord, AgentSpawnPolicy, AgentWorkspaceMode, ChildRuntimeSnapshotRequest,
    DerivedChildRuntime, ForkTurns, RuntimeSnapshotDerivationError, RuntimeSnapshotDeriver,
    RuntimeSnapshotSeed,
};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
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
        let mut snapshot = parent.snapshot.as_object().cloned().ok_or_else(|| {
            RuntimeSnapshotDerivationError::Rejected(
                "parent runtime snapshot must be a JSON object".to_string(),
            )
        })?;
        if let Some(allowed) = snapshot.get("allowedAgentTypes").and_then(Value::as_array) {
            let permitted = allowed
                .iter()
                .filter_map(Value::as_str)
                .any(|agent_type| agent_type == request.agent_type);
            if !permitted {
                return Err(RuntimeSnapshotDerivationError::Rejected(format!(
                    "agent type `{}` is outside the frozen parent snapshot",
                    request.agent_type
                )));
            }
        }

        let parent_spawn = snapshot
            .get("spawnPolicy")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let parent_allows_children = parent_spawn
            .get("allowChildSpawns")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max_depth = parent_spawn
            .get("maxDepth")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(1);
        let max_direct_children = parent_spawn
            .get("maxDirectChildren")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        if request.allow_child_spawns && !parent_allows_children {
            return Err(RuntimeSnapshotDerivationError::Rejected(
                "the parent snapshot does not allow recursive spawn".to_string(),
            ));
        }
        let spawn_policy = if request.allow_child_spawns {
            AgentSpawnPolicy::allows_children(max_depth, max_direct_children)
        } else {
            AgentSpawnPolicy::disabled(max_depth)
        };

        let inherited_workspace = snapshot
            .get("workspaceMode")
            .and_then(Value::as_str)
            .unwrap_or("shared_read_only")
            .to_string();
        let workspace_mode = match request.workspace_mode {
            AgentWorkspaceMode::Auto => inherited_workspace.clone(),
            AgentWorkspaceMode::SharedReadOnly => "shared_read_only".to_string(),
            AgentWorkspaceMode::SharedCoordinated => {
                if inherited_workspace == "shared_read_only" {
                    return Err(RuntimeSnapshotDerivationError::Rejected(
                        "shared coordinated access would expand a read-only parent snapshot"
                            .to_string(),
                    ));
                }
                "shared_coordinated".to_string()
            }
            AgentWorkspaceMode::IsolatedWorktree => "isolated_worktree".to_string(),
        };

        snapshot.insert("agentType".to_string(), Value::String(request.agent_type));
        snapshot.insert(
            "forkTurns".to_string(),
            match request.fork_turns {
                ForkTurns::None => Value::String("none".to_string()),
                ForkTurns::All => Value::String("all".to_string()),
                ForkTurns::Count(count) => json!({ "count": count }),
            },
        );
        let inherited_root = snapshot
            .get("workspaceRoot")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| {
                RuntimeSnapshotDerivationError::Rejected(
                    "parent runtime snapshot does not contain a workspace root".to_string(),
                )
            })?;
        let workspace_assignment = if workspace_mode == "isolated_worktree" {
            let base_commit = snapshot
                .get("gitBaseCommit")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
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
            snapshot.insert(
                "workspaceRoot".to_string(),
                Value::String(root.to_string_lossy().into_owned()),
            );
            json!({
                "mode": "isolated_worktree",
                "repositoryRoot": inherited_root,
                "root": root,
                "branch": format!("codex/agent/{token}"),
                "baseCommit": base_commit,
                "deliveryState": "pending"
            })
        } else {
            json!({
                "mode": workspace_mode,
                "root": inherited_root,
            })
        };
        snapshot.insert("workspaceMode".to_string(), Value::String(workspace_mode));
        snapshot.insert("workspaceAssignment".to_string(), workspace_assignment);
        snapshot.insert("spawnPolicy".to_string(), spawn_policy_json(&spawn_policy));
        Ok(DerivedChildRuntime {
            runtime_snapshot: RuntimeSnapshotSeed::new(Some(parent.id), Value::Object(snapshot)),
            spawn_policy,
        })
    }
}

fn spawn_policy_json(policy: &AgentSpawnPolicy) -> Value {
    let mut value = Map::new();
    value.insert(
        "allowChildSpawns".to_string(),
        Value::Bool(policy.allow_child_spawns),
    );
    value.insert("maxDepth".to_string(), json!(policy.max_depth));
    value.insert(
        "maxDirectChildren".to_string(),
        json!(policy.max_direct_children),
    );
    Value::Object(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{CollaborationSessionId, RuntimeSnapshotId};
    use chrono::Utc;

    fn parent(snapshot: Value) -> AgentRuntimeSnapshotRecord {
        AgentRuntimeSnapshotRecord {
            id: RuntimeSnapshotId::new(),
            session_id: CollaborationSessionId::new(),
            parent_snapshot_id: None,
            content_hash: "fixture".to_string(),
            snapshot,
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
