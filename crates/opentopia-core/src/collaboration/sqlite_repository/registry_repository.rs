//! Transactional CollaborationRegistry implementation.

use super::{
    record_mapping::{
        insert_session_bundle, insert_snapshot, insert_thread, insert_turn, raw_thread,
        read_latest_turn, read_session, read_snapshot, read_thread, read_thread_by_path, read_turn,
        thread_from_raw,
    },
    registry_persistence, SqliteCollaborationRepository,
};
use crate::collaboration::domain::validate_task_name;
use crate::collaboration::{
    AgentPath, AgentRuntimeSnapshotRecord, AgentThreadId, AgentThreadRecord, AgentTurnId,
    AgentTurnRecord, AgentTurnStatus, CollaborationDomainError, CollaborationRegistry,
    CollaborationSessionId, CollaborationSessionRecord, CreateCollaborationSession,
    FollowupAgentTurn, RuntimeSnapshotId, SpawnAgentThread,
};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, TransactionBehavior};

#[async_trait]
impl CollaborationRegistry for SqliteCollaborationRepository {
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
        if request.root_spawn_policy.max_depth > request.session_policy.max_depth {
            return Err(CollaborationDomainError::MaximumDepth {
                actual: request.root_spawn_policy.max_depth,
                maximum: request.session_policy.max_depth,
            });
        }
        let root_message = request.root_task_message.trim().to_string();
        if root_message.is_empty() {
            return Err(CollaborationDomainError::EmptyTaskMessage);
        }
        let root_agent_type = request.root_agent_type.trim().to_string();
        if root_agent_type.is_empty() {
            return Err(CollaborationDomainError::EmptyAgentType);
        }

        let now = Utc::now();
        let session = CollaborationSessionRecord {
            id: CollaborationSessionId::new(),
            user_task_id: request.user_task_id,
            policy: request.session_policy,
            created_at: now,
            closed_at: None,
        };
        let snapshot = AgentRuntimeSnapshotRecord {
            id: request.root_runtime_snapshot.id,
            session_id: session.id,
            parent_snapshot_id: None,
            content_hash: request.root_runtime_snapshot.content_hash,
            snapshot: request.root_runtime_snapshot.snapshot,
            created_at: now,
        };
        let root = AgentThreadRecord {
            id: AgentThreadId::new(),
            session_id: session.id,
            parent_agent_thread_id: None,
            path: AgentPath::root(),
            task_name: "root".to_string(),
            agent_type: root_agent_type,
            runtime_snapshot_id: snapshot.id,
            spawn_policy: request.root_spawn_policy,
            created_at: now,
            archived_at: None,
        };
        let turn = AgentTurnRecord {
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
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                insert_session_bundle(&transaction, &session, &snapshot, &root, &turn)?;
                transaction.commit()?;
                Ok(())
            })
            .map_err(registry_persistence)?;
        Ok((session, root, turn))
    }

    async fn spawn_agent(
        &self,
        request: SpawnAgentThread,
    ) -> Result<(AgentThreadRecord, AgentTurnRecord), CollaborationDomainError> {
        validate_task_name(request.task_name.trim())?;
        request.runtime_snapshot.validate()?;
        let agent_type = request.agent_type.trim().to_string();
        if agent_type.is_empty() {
            return Err(CollaborationDomainError::EmptyAgentType);
        }
        let task_message = request.task_message.trim().to_string();
        if task_message.is_empty() {
            return Err(CollaborationDomainError::EmptyTaskMessage);
        }

        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let parent = read_thread(&transaction, request.parent_agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("parent AgentThread was not found"))?;
                if parent.is_archived() {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::AgentThreadArchived(parent.id)
                    ));
                }
                let requesting_turn = read_turn(&transaction, request.requested_by_turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("requesting AgentTurn was not found"))?;
                if requesting_turn.agent_thread_id != parent.id {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::RequestingTurnOwnership {
                            turn_id: request.requested_by_turn_id,
                            agent_thread_id: parent.id,
                        }
                    ));
                }
                if !parent.spawn_policy.allow_child_spawns {
                    return Err(anyhow::anyhow!(CollaborationDomainError::SpawnDisabled(
                        parent.path
                    )));
                }
                if !request.spawn_policy.is_attenuation_of(&parent.spawn_policy) {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::SpawnPolicyEscalation
                    ));
                }
                if request.runtime_snapshot.parent_snapshot_id != Some(parent.runtime_snapshot_id) {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::InvalidRuntimeSnapshot(
                            "child runtime snapshot must reference the parent Agent snapshot"
                                .to_string()
                        )
                    ));
                }
                let session = read_session(&transaction, parent.session_id)?
                    .ok_or_else(|| anyhow::anyhow!("CollaborationSession was not found"))?;
                let path = parent.path.child(request.task_name.trim())?;
                let maximum_depth = session.policy.max_depth.min(parent.spawn_policy.max_depth);
                if path.depth() > maximum_depth {
                    return Err(anyhow::anyhow!(CollaborationDomainError::MaximumDepth {
                        actual: path.depth(),
                        maximum: maximum_depth,
                    }));
                }
                if read_thread_by_path(&transaction, parent.session_id, &path)?.is_some() {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::DuplicateAgentPath(path)
                    ));
                }
                let session_agents: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM agent_threads WHERE session_id = ?1",
                    params![parent.session_id.to_string()],
                    |row| row.get(0),
                )?;
                if usize::try_from(session_agents)? >= session.policy.max_agents {
                    return Err(anyhow::anyhow!(CollaborationDomainError::MaximumAgents {
                        maximum: session.policy.max_agents,
                    }));
                }
                let direct_children: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM agent_threads WHERE parent_agent_thread_id = ?1",
                    params![parent.id.to_string()],
                    |row| row.get(0),
                )?;
                if usize::try_from(direct_children)? >= parent.spawn_policy.max_direct_children {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::MaximumDirectChildren {
                            path: parent.path,
                            maximum: parent.spawn_policy.max_direct_children,
                        }
                    ));
                }

                let now = Utc::now();
                let snapshot = AgentRuntimeSnapshotRecord {
                    id: request.runtime_snapshot.id,
                    session_id: parent.session_id,
                    parent_snapshot_id: request.runtime_snapshot.parent_snapshot_id,
                    content_hash: request.runtime_snapshot.content_hash,
                    snapshot: request.runtime_snapshot.snapshot,
                    created_at: now,
                };
                let child = AgentThreadRecord {
                    id: AgentThreadId::new(),
                    session_id: parent.session_id,
                    parent_agent_thread_id: Some(parent.id),
                    path,
                    task_name: request.task_name.trim().to_string(),
                    agent_type,
                    runtime_snapshot_id: snapshot.id,
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
                insert_snapshot(&transaction, &snapshot)?;
                insert_thread(&transaction, &child)?;
                insert_turn(&transaction, &turn)?;
                transaction.commit()?;
                Ok((child, turn))
            })
            .map_err(registry_persistence)
    }

    async fn create_followup_turn(
        &self,
        request: FollowupAgentTurn,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        let task_message = request.task_message.trim().to_string();
        if task_message.is_empty() {
            return Err(CollaborationDomainError::EmptyTaskMessage);
        }
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let caller = read_thread(&transaction, request.requested_by_agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("requesting AgentThread was not found"))?;
                let requesting_turn = read_turn(&transaction, request.requested_by_turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("requesting AgentTurn was not found"))?;
                if requesting_turn.agent_thread_id != caller.id {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::RequestingTurnOwnership {
                            turn_id: request.requested_by_turn_id,
                            agent_thread_id: caller.id,
                        }
                    ));
                }
                let target = read_thread(&transaction, request.target_agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("target AgentThread was not found"))?;
                if caller.session_id != target.session_id
                    || !target.path.is_descendant_of(&caller.path)
                {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::LifecyclePermissionDenied {
                            caller: caller.path,
                            target: target.path,
                        }
                    ));
                }
                if target.is_archived() {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::AgentThreadArchived(target.id)
                    ));
                }
                let latest = read_latest_turn(&transaction, target.id)?;
                if latest
                    .as_ref()
                    .is_some_and(|turn| !turn.status.is_terminal())
                {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::AgentTurnAlreadyActive(target.id)
                    ));
                }
                let sequence = latest.map_or(1, |turn| turn.sequence + 1);
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
                insert_turn(&transaction, &turn)?;
                transaction.commit()?;
                Ok(turn)
            })
            .map_err(registry_persistence)
    }

    async fn transition_turn(
        &self,
        turn_id: AgentTurnId,
        next: AgentTurnStatus,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut turn = read_turn(&transaction, turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentTurn was not found"))?;
                turn.transition(next, Utc::now())?;
                transaction.execute(
                    "UPDATE agent_turns SET status = ?2, started_at = ?3, completed_at = ?4 WHERE id = ?1",
                    params![
                        turn.id.to_string(),
                        turn.status.as_str(),
                        turn.started_at.map(|value| value.to_rfc3339()),
                        turn.completed_at.map(|value| value.to_rfc3339()),
                    ],
                )?;
                transaction.commit()?;
                Ok(turn)
            })
            .map_err(registry_persistence)
    }

    async fn get_session(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<CollaborationSessionRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                read_session(connection, session_id)?
                    .ok_or_else(|| anyhow::anyhow!("CollaborationSession was not found"))
            })
            .map_err(registry_persistence)
    }

    async fn get_thread(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<AgentThreadRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                read_thread(connection, agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentThread was not found"))
            })
            .map_err(registry_persistence)
    }

    async fn get_turn(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                read_turn(connection, turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentTurn was not found"))
            })
            .map_err(registry_persistence)
    }

    async fn get_runtime_snapshot(
        &self,
        snapshot_id: RuntimeSnapshotId,
    ) -> Result<AgentRuntimeSnapshotRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                read_snapshot(connection, snapshot_id)?
                    .ok_or_else(|| anyhow::anyhow!("RuntimeSnapshot was not found"))
            })
            .map_err(registry_persistence)
    }

    async fn latest_turn(
        &self,
        agent_thread_id: AgentThreadId,
    ) -> Result<Option<AgentTurnRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                if read_thread(connection, agent_thread_id)?.is_none() {
                    return Err(anyhow::anyhow!("AgentThread was not found"));
                }
                read_latest_turn(connection, agent_thread_id)
            })
            .map_err(registry_persistence)
    }

    async fn resolve_path(
        &self,
        session_id: CollaborationSessionId,
        path: &AgentPath,
    ) -> Result<Option<AgentThreadRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                if read_session(connection, session_id)?.is_none() {
                    return Err(anyhow::anyhow!("CollaborationSession was not found"));
                }
                read_thread_by_path(connection, session_id, path)
            })
            .map_err(registry_persistence)
    }

    async fn list_threads(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<Vec<AgentThreadRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                if read_session(connection, session_id)?.is_none() {
                    return Err(anyhow::anyhow!("CollaborationSession was not found"));
                }
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, session_id, parent_agent_thread_id, agent_path, task_name,
                           agent_type, runtime_snapshot_id, spawn_policy_json, created_at, archived_at
                    FROM agent_threads WHERE session_id = ?1 ORDER BY agent_path
                    "#,
                )?;
                let rows = statement.query_map(params![session_id.to_string()], raw_thread)?;
                let mut threads = Vec::new();
                for row in rows {
                    threads.push(thread_from_raw(row?)?);
                }
                Ok(threads)
            })
            .map_err(registry_persistence)
    }
}
