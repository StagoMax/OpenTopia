use super::domain::{
    validate_task_name, AgentPath, AgentRuntimeSnapshotRecord, AgentSpawnPolicy, AgentThreadId,
    AgentThreadRecord, AgentTurnId, AgentTurnRecord, AgentTurnStatus, CollaborationDomainError,
    CollaborationSessionId, CollaborationSessionPolicy, CollaborationSessionRecord,
    RuntimeSnapshotId, RuntimeSnapshotSeed,
};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CreateCollaborationSession {
    pub user_task_id: Uuid,
    pub root_turn_id: AgentTurnId,
    pub root_task_message: String,
    pub root_agent_type: String,
    pub root_runtime_snapshot: RuntimeSnapshotSeed,
    pub session_policy: CollaborationSessionPolicy,
    pub root_spawn_policy: AgentSpawnPolicy,
}

#[derive(Debug, Clone)]
pub struct SpawnAgentThread {
    pub parent_agent_thread_id: AgentThreadId,
    pub requested_by_turn_id: AgentTurnId,
    pub task_name: String,
    pub agent_type: String,
    pub task_message: String,
    pub runtime_snapshot: RuntimeSnapshotSeed,
    pub spawn_policy: AgentSpawnPolicy,
}

#[derive(Debug, Clone)]
pub struct FollowupAgentTurn {
    pub requested_by_agent_thread_id: AgentThreadId,
    pub requested_by_turn_id: AgentTurnId,
    pub target_agent_thread_id: AgentThreadId,
    pub task_message: String,
}

#[async_trait]
pub trait CollaborationRegistry: Send + Sync {
    async fn create_session(
        &self,
        request: CreateCollaborationSession,
    ) -> Result<
        (
            CollaborationSessionRecord,
            AgentThreadRecord,
            AgentTurnRecord,
        ),
        CollaborationDomainError,
    >;

    async fn spawn_agent(
        &self,
        request: SpawnAgentThread,
    ) -> Result<(AgentThreadRecord, AgentTurnRecord), CollaborationDomainError>;

    async fn create_followup_turn(
        &self,
        request: FollowupAgentTurn,
    ) -> Result<AgentTurnRecord, CollaborationDomainError>;

    async fn transition_turn(
        &self,
        turn_id: AgentTurnId,
        next: AgentTurnStatus,
    ) -> Result<AgentTurnRecord, CollaborationDomainError>;

    async fn get_session(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<CollaborationSessionRecord, CollaborationDomainError>;

    async fn get_thread(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<AgentThreadRecord, CollaborationDomainError>;

    async fn get_turn(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<AgentTurnRecord, CollaborationDomainError>;

    async fn get_runtime_snapshot(
        &self,
        snapshot_id: RuntimeSnapshotId,
    ) -> Result<AgentRuntimeSnapshotRecord, CollaborationDomainError>;

    async fn latest_turn(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<Option<AgentTurnRecord>, CollaborationDomainError>;

    async fn resolve_path(
        &self,
        session_id: CollaborationSessionId,
        path: &AgentPath,
    ) -> Result<Option<AgentThreadRecord>, CollaborationDomainError>;

    async fn list_threads(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<Vec<AgentThreadRecord>, CollaborationDomainError>;
}

#[derive(Default)]
struct RegistryState {
    sessions: HashMap<CollaborationSessionId, CollaborationSessionRecord>,
    threads: HashMap<AgentThreadId, AgentThreadRecord>,
    turns: HashMap<AgentTurnId, AgentTurnRecord>,
    snapshots: HashMap<RuntimeSnapshotId, AgentRuntimeSnapshotRecord>,
    paths: HashMap<(CollaborationSessionId, AgentPath), AgentThreadId>,
    turns_by_thread: HashMap<AgentThreadId, Vec<AgentTurnId>>,
}

#[derive(Clone, Default)]
pub struct InMemoryCollaborationRegistry {
    state: Arc<RwLock<RegistryState>>,
}

impl InMemoryCollaborationRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

fn require_text(
    value: &str,
    empty: CollaborationDomainError,
) -> Result<String, CollaborationDomainError> {
    let value = value.trim();
    if value.is_empty() {
        Err(empty)
    } else {
        Ok(value.to_string())
    }
}

fn latest_turn_locked(
    state: &RegistryState,
    agent_thread_id: AgentThreadId,
) -> Option<&AgentTurnRecord> {
    state
        .turns_by_thread
        .get(&agent_thread_id)
        .and_then(|ids| ids.last())
        .and_then(|id| state.turns.get(id))
}

fn ensure_requesting_turn(
    state: &RegistryState,
    agent_thread_id: AgentThreadId,
    turn_id: AgentTurnId,
) -> Result<(), CollaborationDomainError> {
    let turn = state
        .turns
        .get(&turn_id)
        .ok_or(CollaborationDomainError::AgentTurnNotFound(turn_id))?;
    if turn.agent_thread_id != agent_thread_id {
        return Err(CollaborationDomainError::RequestingTurnOwnership {
            turn_id,
            agent_thread_id,
        });
    }
    Ok(())
}

#[async_trait]
impl CollaborationRegistry for InMemoryCollaborationRegistry {
    async fn create_session(
        &self,
        request: CreateCollaborationSession,
    ) -> Result<
        (
            CollaborationSessionRecord,
            AgentThreadRecord,
            AgentTurnRecord,
        ),
        CollaborationDomainError,
    > {
        request.session_policy.validate()?;
        request.root_runtime_snapshot.validate()?;
        if request.root_runtime_snapshot.parent_snapshot_id.is_some() {
            return Err(CollaborationDomainError::InvalidRuntimeSnapshot(
                "root runtime snapshot cannot have a parent".to_string(),
            ));
        }
        let root_message = require_text(
            &request.root_task_message,
            CollaborationDomainError::EmptyTaskMessage,
        )?;
        let root_agent_type = require_text(
            &request.root_agent_type,
            CollaborationDomainError::EmptyAgentType,
        )?;
        if request.root_spawn_policy.max_depth > request.session_policy.max_depth {
            return Err(CollaborationDomainError::MaximumDepth {
                actual: request.root_spawn_policy.max_depth,
                maximum: request.session_policy.max_depth,
            });
        }

        let now = Utc::now();
        let session = CollaborationSessionRecord {
            id: CollaborationSessionId::new(),
            user_task_id: request.user_task_id,
            policy: request.session_policy,
            created_at: now,
            closed_at: None,
        };
        let root = AgentThreadRecord {
            id: AgentThreadId::new(),
            session_id: session.id,
            parent_agent_thread_id: None,
            path: AgentPath::root(),
            task_name: "root".to_string(),
            agent_type: root_agent_type,
            runtime_snapshot_id: request.root_runtime_snapshot.id,
            spawn_policy: request.root_spawn_policy,
            created_at: now,
            archived_at: None,
        };
        let root_turn = AgentTurnRecord {
            id: request.root_turn_id,
            session_id: session.id,
            agent_thread_id: root.id,
            requested_by_agent_thread_id: None,
            requested_by_turn_id: None,
            sequence: 1,
            task_message: root_message,
            status: AgentTurnStatus::Queued,
            invocation_id: 1,
            outcome_ref: None,
            created_at: now,
            started_at: None,
            completed_at: None,
        };
        let root_snapshot = AgentRuntimeSnapshotRecord {
            id: request.root_runtime_snapshot.id,
            session_id: session.id,
            parent_snapshot_id: None,
            content_hash: request.root_runtime_snapshot.content_hash,
            snapshot: request.root_runtime_snapshot.snapshot,
            created_at: now,
        };

        let mut state = self
            .state
            .write()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        state.sessions.insert(session.id, session.clone());
        state.snapshots.insert(root_snapshot.id, root_snapshot);
        state.paths.insert((session.id, root.path.clone()), root.id);
        state.threads.insert(root.id, root.clone());
        state.turns.insert(root_turn.id, root_turn.clone());
        state.turns_by_thread.insert(root.id, vec![root_turn.id]);
        Ok((session, root, root_turn))
    }

    async fn spawn_agent(
        &self,
        request: SpawnAgentThread,
    ) -> Result<(AgentThreadRecord, AgentTurnRecord), CollaborationDomainError> {
        validate_task_name(request.task_name.trim())?;
        let agent_type = require_text(
            &request.agent_type,
            CollaborationDomainError::EmptyAgentType,
        )?;
        let task_message = require_text(
            &request.task_message,
            CollaborationDomainError::EmptyTaskMessage,
        )?;
        request.runtime_snapshot.validate()?;
        let mut state = self
            .state
            .write()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        let parent = state
            .threads
            .get(&request.parent_agent_thread_id)
            .cloned()
            .ok_or(CollaborationDomainError::AgentThreadNotFound(
                request.parent_agent_thread_id,
            ))?;
        if parent.is_archived() {
            return Err(CollaborationDomainError::AgentThreadArchived(parent.id));
        }
        ensure_requesting_turn(&state, parent.id, request.requested_by_turn_id)?;
        if !parent.spawn_policy.allow_child_spawns {
            return Err(CollaborationDomainError::SpawnDisabled(parent.path));
        }
        if !request.spawn_policy.is_attenuation_of(&parent.spawn_policy) {
            return Err(CollaborationDomainError::SpawnPolicyEscalation);
        }
        if request.runtime_snapshot.parent_snapshot_id != Some(parent.runtime_snapshot_id) {
            return Err(CollaborationDomainError::InvalidRuntimeSnapshot(
                "child runtime snapshot must reference the parent Agent snapshot".to_string(),
            ));
        }

        let session = state
            .sessions
            .get(&parent.session_id)
            .ok_or(CollaborationDomainError::SessionNotFound(parent.session_id))?;
        let path = parent.path.child(request.task_name.trim())?;
        let child_depth = path.depth();
        let max_depth = session.policy.max_depth.min(parent.spawn_policy.max_depth);
        if child_depth > max_depth {
            return Err(CollaborationDomainError::MaximumDepth {
                actual: child_depth,
                maximum: max_depth,
            });
        }
        if state.paths.contains_key(&(parent.session_id, path.clone())) {
            return Err(CollaborationDomainError::DuplicateAgentPath(path));
        }
        let session_agent_count = state
            .threads
            .values()
            .filter(|thread| thread.session_id == parent.session_id)
            .count();
        if session_agent_count >= session.policy.max_agents {
            return Err(CollaborationDomainError::MaximumAgents {
                maximum: session.policy.max_agents,
            });
        }
        let direct_children = state
            .threads
            .values()
            .filter(|thread| thread.parent_agent_thread_id == Some(parent.id))
            .count();
        if direct_children >= parent.spawn_policy.max_direct_children {
            return Err(CollaborationDomainError::MaximumDirectChildren {
                path: parent.path,
                maximum: parent.spawn_policy.max_direct_children,
            });
        }

        let now = Utc::now();
        let child = AgentThreadRecord {
            id: AgentThreadId::new(),
            session_id: parent.session_id,
            parent_agent_thread_id: Some(parent.id),
            path: path.clone(),
            task_name: request.task_name.trim().to_string(),
            agent_type,
            runtime_snapshot_id: request.runtime_snapshot.id,
            spawn_policy: request.spawn_policy,
            created_at: now,
            archived_at: None,
        };
        let turn = AgentTurnRecord {
            id: AgentTurnId::new(),
            session_id: parent.session_id,
            agent_thread_id: child.id,
            requested_by_agent_thread_id: Some(parent.id),
            requested_by_turn_id: Some(request.requested_by_turn_id),
            sequence: 1,
            task_message,
            status: AgentTurnStatus::Queued,
            invocation_id: 1,
            outcome_ref: None,
            created_at: now,
            started_at: None,
            completed_at: None,
        };
        let snapshot = AgentRuntimeSnapshotRecord {
            id: request.runtime_snapshot.id,
            session_id: parent.session_id,
            parent_snapshot_id: request.runtime_snapshot.parent_snapshot_id,
            content_hash: request.runtime_snapshot.content_hash,
            snapshot: request.runtime_snapshot.snapshot,
            created_at: now,
        };
        state.snapshots.insert(snapshot.id, snapshot);
        state.paths.insert((parent.session_id, path), child.id);
        state.threads.insert(child.id, child.clone());
        state.turns.insert(turn.id, turn.clone());
        state.turns_by_thread.insert(child.id, vec![turn.id]);
        Ok((child, turn))
    }

    async fn create_followup_turn(
        &self,
        request: FollowupAgentTurn,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        let task_message = require_text(
            &request.task_message,
            CollaborationDomainError::EmptyTaskMessage,
        )?;
        let mut state = self
            .state
            .write()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        ensure_requesting_turn(
            &state,
            request.requested_by_agent_thread_id,
            request.requested_by_turn_id,
        )?;
        let caller = state
            .threads
            .get(&request.requested_by_agent_thread_id)
            .cloned()
            .ok_or(CollaborationDomainError::AgentThreadNotFound(
                request.requested_by_agent_thread_id,
            ))?;
        let target = state
            .threads
            .get(&request.target_agent_thread_id)
            .cloned()
            .ok_or(CollaborationDomainError::AgentThreadNotFound(
                request.target_agent_thread_id,
            ))?;
        if caller.session_id != target.session_id || !target.path.is_descendant_of(&caller.path) {
            return Err(CollaborationDomainError::LifecyclePermissionDenied {
                caller: caller.path,
                target: target.path,
            });
        }
        if target.is_archived() {
            return Err(CollaborationDomainError::AgentThreadArchived(target.id));
        }
        if latest_turn_locked(&state, target.id).is_some_and(|turn| !turn.status.is_terminal()) {
            return Err(CollaborationDomainError::AgentTurnAlreadyActive(target.id));
        }
        let sequence = state
            .turns_by_thread
            .get(&target.id)
            .map_or(1, |turns| turns.len() as u64 + 1);
        let turn = AgentTurnRecord {
            id: AgentTurnId::new(),
            session_id: target.session_id,
            agent_thread_id: target.id,
            requested_by_agent_thread_id: Some(caller.id),
            requested_by_turn_id: Some(request.requested_by_turn_id),
            sequence,
            task_message,
            status: AgentTurnStatus::Queued,
            invocation_id: 1,
            outcome_ref: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        state.turns.insert(turn.id, turn.clone());
        state
            .turns_by_thread
            .entry(target.id)
            .or_default()
            .push(turn.id);
        Ok(turn)
    }

    async fn transition_turn(
        &self,
        turn_id: AgentTurnId,
        next: AgentTurnStatus,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        let turn = state
            .turns
            .get_mut(&turn_id)
            .ok_or(CollaborationDomainError::AgentTurnNotFound(turn_id))?;
        turn.transition(next, Utc::now())?;
        Ok(turn.clone())
    }

    async fn get_session(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<CollaborationSessionRecord, CollaborationDomainError> {
        self.state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or(CollaborationDomainError::SessionNotFound(session_id))
    }

    async fn get_thread(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<AgentThreadRecord, CollaborationDomainError> {
        self.state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?
            .threads
            .get(&agent_thread_id)
            .cloned()
            .ok_or(CollaborationDomainError::AgentThreadNotFound(
                agent_thread_id,
            ))
    }

    async fn get_turn(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        self.state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?
            .turns
            .get(&turn_id)
            .cloned()
            .ok_or(CollaborationDomainError::AgentTurnNotFound(turn_id))
    }

    async fn get_runtime_snapshot(
        &self,
        snapshot_id: RuntimeSnapshotId,
    ) -> Result<AgentRuntimeSnapshotRecord, CollaborationDomainError> {
        self.state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?
            .snapshots
            .get(&snapshot_id)
            .cloned()
            .ok_or_else(|| {
                CollaborationDomainError::InvalidRuntimeSnapshot(format!(
                    "runtime snapshot not found: {snapshot_id}"
                ))
            })
    }

    async fn latest_turn(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<Option<AgentTurnRecord>, CollaborationDomainError> {
        let state = self
            .state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        if !state.threads.contains_key(&agent_thread_id) {
            return Err(CollaborationDomainError::AgentThreadNotFound(
                agent_thread_id,
            ));
        }
        Ok(latest_turn_locked(&state, agent_thread_id).cloned())
    }

    async fn resolve_path(
        &self,
        session_id: CollaborationSessionId,
        path: &AgentPath,
    ) -> Result<Option<AgentThreadRecord>, CollaborationDomainError> {
        let state = self
            .state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        if !state.sessions.contains_key(&session_id) {
            return Err(CollaborationDomainError::SessionNotFound(session_id));
        }
        Ok(state
            .paths
            .get(&(session_id, path.clone()))
            .and_then(|id| state.threads.get(id))
            .cloned())
    }

    async fn list_threads(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<Vec<AgentThreadRecord>, CollaborationDomainError> {
        let state = self
            .state
            .read()
            .map_err(|_| CollaborationDomainError::RegistryPoisoned)?;
        if !state.sessions.contains_key(&session_id) {
            return Err(CollaborationDomainError::SessionNotFound(session_id));
        }
        let mut threads = state
            .threads
            .values()
            .filter(|thread| thread.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        threads.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(threads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{test_runtime_snapshot, RuntimeWorkspaceModeV1};

    async fn session_with_depth(
        depth: u16,
    ) -> (
        InMemoryCollaborationRegistry,
        CollaborationSessionRecord,
        AgentThreadRecord,
        AgentTurnRecord,
    ) {
        let registry = InMemoryCollaborationRegistry::new();
        let (session, root, turn) = registry
            .create_session(CreateCollaborationSession {
                user_task_id: Uuid::new_v4(),
                root_turn_id: AgentTurnId::new(),
                root_task_message: "implement the feature".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(
                    None,
                    test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                session_policy: CollaborationSessionPolicy {
                    max_agents: 8,
                    max_active_runs: 4,
                    max_depth: depth,
                },
                root_spawn_policy: AgentSpawnPolicy::allows_children(depth, 4),
            })
            .await
            .unwrap();
        (registry, session, root, turn)
    }

    #[tokio::test]
    async fn spawn_creates_identity_and_first_turn_atomically() {
        let (registry, session, root, root_turn) = session_with_depth(2).await;
        let (child, child_turn) = registry
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect the runtime".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::allows_children(2, 2),
            })
            .await
            .unwrap();

        assert_eq!(child.session_id, session.id);
        assert_eq!(child.path.as_str(), "/root/research");
        assert_eq!(child_turn.agent_thread_id, child.id);
        assert_eq!(child_turn.sequence, 1);
        assert_eq!(child_turn.status, AgentTurnStatus::Queued);
        assert_eq!(registry.list_threads(session.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn grandchildren_use_the_same_spawn_operation_when_policy_allows() {
        let (registry, _session, root, root_turn) = session_with_depth(2).await;
        let (child, child_turn) = registry
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::allows_children(2, 2),
            })
            .await
            .unwrap();
        let (grandchild, _) = registry
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: child.id,
                requested_by_turn_id: child_turn.id,
                task_name: "reviewer".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "review".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(child.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(2),
            })
            .await
            .unwrap();

        assert_eq!(grandchild.path.as_str(), "/root/research/reviewer");
        assert_eq!(grandchild.path.depth(), 2);
    }

    #[tokio::test]
    async fn followup_creates_a_new_turn_without_overwriting_history() {
        let (registry, _session, root, root_turn) = session_with_depth(1).await;
        let (child, first_turn) = registry
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "worker".to_string(),
                agent_type: "worker".to_string(),
                task_message: "first".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("worker", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap();
        registry
            .transition_turn(first_turn.id, AgentTurnStatus::Running)
            .await
            .unwrap();
        registry
            .transition_turn(first_turn.id, AgentTurnStatus::Completed)
            .await
            .unwrap();

        let second_turn = registry
            .create_followup_turn(FollowupAgentTurn {
                requested_by_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                target_agent_thread_id: child.id,
                task_message: "second".to_string(),
            })
            .await
            .unwrap();

        assert_ne!(first_turn.id, second_turn.id);
        assert_eq!(second_turn.sequence, 2);
        assert_eq!(
            registry.get_turn(first_turn.id).await.unwrap().task_message,
            "first"
        );
    }

    #[tokio::test]
    async fn active_turn_blocks_followup() {
        let (registry, _session, root, root_turn) = session_with_depth(1).await;
        let (child, _) = registry
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "worker".to_string(),
                agent_type: "worker".to_string(),
                task_message: "first".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("worker", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap();

        let error = registry
            .create_followup_turn(FollowupAgentTurn {
                requested_by_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                target_agent_thread_id: child.id,
                task_message: "too soon".to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(
            error,
            CollaborationDomainError::AgentTurnAlreadyActive(child.id)
        );
    }
}
