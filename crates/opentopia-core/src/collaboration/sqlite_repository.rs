use super::domain::validate_task_name;
use super::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageId,
    AgentMailboxMessageKind, AgentPath, AgentRuntimeSnapshotRecord, AgentThreadId,
    AgentThreadRecord, AgentTurnId, AgentTurnRecord, AgentTurnStatus, CollaborationDomainError,
    CollaborationRegistry, CollaborationSessionId, CollaborationSessionRecord,
    CreateCollaborationSession, EnqueueAgentMessage, FollowupAgentTurn, RuntimeSnapshotId,
    SpawnAgentThread,
};
use crate::store::SqliteSessionStore;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteCollaborationRepository {
    store: Arc<SqliteSessionStore>,
}

impl SqliteCollaborationRepository {
    pub fn new(store: Arc<SqliteSessionStore>) -> Result<Self, CollaborationDomainError> {
        store
            .with_collaboration_read(|connection| {
                for table in [
                    "agent_sessions",
                    "agent_runtime_snapshots",
                    "agent_threads",
                    "agent_turns",
                    "agent_mailbox_messages",
                ] {
                    let exists: bool = connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
                        params![table],
                        |row| row.get(0),
                    )?;
                    anyhow::ensure!(exists, "required collaboration table `{table}` is missing");
                }
                Ok(())
            })
            .map_err(registry_persistence)?;
        Ok(Self { store })
    }
}

struct RawSession {
    id: String,
    user_task_id: String,
    policy_json: String,
    created_at: String,
    closed_at: Option<String>,
}

struct RawSnapshot {
    id: String,
    session_id: String,
    parent_snapshot_id: Option<String>,
    content_hash: String,
    snapshot_json: String,
    created_at: String,
}

struct RawThread {
    id: String,
    session_id: String,
    parent_agent_thread_id: Option<String>,
    agent_path: String,
    task_name: String,
    agent_type: String,
    runtime_snapshot_id: String,
    spawn_policy_json: String,
    created_at: String,
    archived_at: Option<String>,
}

struct RawTurn {
    id: String,
    session_id: String,
    agent_thread_id: String,
    requested_by_agent_thread_id: Option<String>,
    requested_by_turn_id: Option<String>,
    sequence: i64,
    task_message: String,
    status: String,
    invocation_id: i64,
    outcome_ref: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

struct RawMailboxMessage {
    id: String,
    session_id: String,
    sequence: i64,
    from_agent_thread_id: String,
    to_agent_thread_id: String,
    kind: String,
    payload_json: String,
    causation_id: Option<String>,
    created_at: String,
    acknowledged_at: Option<String>,
}

fn registry_persistence(error: anyhow::Error) -> CollaborationDomainError {
    match error.downcast::<CollaborationDomainError>() {
        Ok(domain) => domain,
        Err(error) => CollaborationDomainError::Persistence(error.to_string()),
    }
}

fn mailbox_persistence(error: impl Into<anyhow::Error>) -> AgentMailboxError {
    let error = error.into();
    match error.downcast::<AgentMailboxError>() {
        Ok(mailbox) => mailbox,
        Err(error) => AgentMailboxError::Persistence(error.to_string()),
    }
}

fn parse_uuid(value: &str) -> anyhow::Result<Uuid> {
    Ok(Uuid::parse_str(value)?)
}

fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

fn parse_optional_time(value: Option<String>) -> anyhow::Result<Option<DateTime<Utc>>> {
    value.map(|value| parse_time(&value)).transpose()
}

fn read_session(
    connection: &Connection,
    session_id: CollaborationSessionId,
) -> anyhow::Result<Option<CollaborationSessionRecord>> {
    let raw = connection
        .query_row(
            "SELECT id, user_task_id, policy_json, created_at, closed_at FROM agent_sessions WHERE id = ?1",
            params![session_id.to_string()],
            |row| {
                Ok(RawSession {
                    id: row.get(0)?,
                    user_task_id: row.get(1)?,
                    policy_json: row.get(2)?,
                    created_at: row.get(3)?,
                    closed_at: row.get(4)?,
                })
            },
        )
        .optional()?;
    raw.map(|raw| {
        Ok(CollaborationSessionRecord {
            id: CollaborationSessionId::from_uuid(parse_uuid(&raw.id)?),
            user_task_id: parse_uuid(&raw.user_task_id)?,
            policy: serde_json::from_str(&raw.policy_json)?,
            created_at: parse_time(&raw.created_at)?,
            closed_at: parse_optional_time(raw.closed_at)?,
        })
    })
    .transpose()
}

fn read_snapshot(
    connection: &Connection,
    snapshot_id: RuntimeSnapshotId,
) -> anyhow::Result<Option<AgentRuntimeSnapshotRecord>> {
    let raw = connection
        .query_row(
            "SELECT id, session_id, parent_snapshot_id, content_hash, snapshot_json, created_at FROM agent_runtime_snapshots WHERE id = ?1",
            params![snapshot_id.to_string()],
            |row| {
                Ok(RawSnapshot {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    parent_snapshot_id: row.get(2)?,
                    content_hash: row.get(3)?,
                    snapshot_json: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    raw.map(|raw| {
        Ok(AgentRuntimeSnapshotRecord {
            id: RuntimeSnapshotId::from_uuid(parse_uuid(&raw.id)?),
            session_id: CollaborationSessionId::from_uuid(parse_uuid(&raw.session_id)?),
            parent_snapshot_id: raw
                .parent_snapshot_id
                .map(|id| parse_uuid(&id).map(RuntimeSnapshotId::from_uuid))
                .transpose()?,
            content_hash: raw.content_hash,
            snapshot: serde_json::from_str(&raw.snapshot_json)?,
            created_at: parse_time(&raw.created_at)?,
        })
    })
    .transpose()
}

fn read_thread(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
) -> anyhow::Result<Option<AgentThreadRecord>> {
    read_thread_where(connection, "id = ?1", agent_thread_id.to_string())
}

fn read_thread_by_path(
    connection: &Connection,
    session_id: CollaborationSessionId,
    path: &AgentPath,
) -> anyhow::Result<Option<AgentThreadRecord>> {
    let raw = connection
        .query_row(
            r#"
            SELECT id, session_id, parent_agent_thread_id, agent_path, task_name,
                   agent_type, runtime_snapshot_id, spawn_policy_json, created_at, archived_at
            FROM agent_threads
            WHERE session_id = ?1 AND agent_path = ?2
            "#,
            params![session_id.to_string(), path.as_str()],
            raw_thread,
        )
        .optional()?;
    raw.map(thread_from_raw).transpose()
}

fn read_thread_where(
    connection: &Connection,
    predicate: &str,
    value: String,
) -> anyhow::Result<Option<AgentThreadRecord>> {
    let sql = format!(
        r#"
        SELECT id, session_id, parent_agent_thread_id, agent_path, task_name,
               agent_type, runtime_snapshot_id, spawn_policy_json, created_at, archived_at
        FROM agent_threads WHERE {predicate}
        "#
    );
    let raw = connection
        .query_row(&sql, params![value], raw_thread)
        .optional()?;
    raw.map(thread_from_raw).transpose()
}

fn raw_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawThread> {
    Ok(RawThread {
        id: row.get(0)?,
        session_id: row.get(1)?,
        parent_agent_thread_id: row.get(2)?,
        agent_path: row.get(3)?,
        task_name: row.get(4)?,
        agent_type: row.get(5)?,
        runtime_snapshot_id: row.get(6)?,
        spawn_policy_json: row.get(7)?,
        created_at: row.get(8)?,
        archived_at: row.get(9)?,
    })
}

fn thread_from_raw(raw: RawThread) -> anyhow::Result<AgentThreadRecord> {
    Ok(AgentThreadRecord {
        id: AgentThreadId::from_uuid(parse_uuid(&raw.id)?),
        session_id: CollaborationSessionId::from_uuid(parse_uuid(&raw.session_id)?),
        parent_agent_thread_id: raw
            .parent_agent_thread_id
            .map(|id| parse_uuid(&id).map(AgentThreadId::from_uuid))
            .transpose()?,
        path: AgentPath::parse(&raw.agent_path)?,
        task_name: raw.task_name,
        agent_type: raw.agent_type,
        runtime_snapshot_id: RuntimeSnapshotId::from_uuid(parse_uuid(&raw.runtime_snapshot_id)?),
        spawn_policy: serde_json::from_str(&raw.spawn_policy_json)?,
        created_at: parse_time(&raw.created_at)?,
        archived_at: parse_optional_time(raw.archived_at)?,
    })
}

fn read_turn(
    connection: &Connection,
    turn_id: AgentTurnId,
) -> anyhow::Result<Option<AgentTurnRecord>> {
    let raw = connection
        .query_row(
            r#"
            SELECT id, session_id, agent_thread_id, requested_by_agent_thread_id,
                   requested_by_turn_id, sequence, task_message, status, invocation_id,
                   outcome_ref, created_at, started_at, completed_at
            FROM agent_turns WHERE id = ?1
            "#,
            params![turn_id.to_string()],
            raw_turn,
        )
        .optional()?;
    raw.map(turn_from_raw).transpose()
}

fn read_latest_turn(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
) -> anyhow::Result<Option<AgentTurnRecord>> {
    let raw = connection
        .query_row(
            r#"
            SELECT id, session_id, agent_thread_id, requested_by_agent_thread_id,
                   requested_by_turn_id, sequence, task_message, status, invocation_id,
                   outcome_ref, created_at, started_at, completed_at
            FROM agent_turns WHERE agent_thread_id = ?1 ORDER BY sequence DESC LIMIT 1
            "#,
            params![agent_thread_id.to_string()],
            raw_turn,
        )
        .optional()?;
    raw.map(turn_from_raw).transpose()
}

fn raw_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTurn> {
    Ok(RawTurn {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_thread_id: row.get(2)?,
        requested_by_agent_thread_id: row.get(3)?,
        requested_by_turn_id: row.get(4)?,
        sequence: row.get(5)?,
        task_message: row.get(6)?,
        status: row.get(7)?,
        invocation_id: row.get(8)?,
        outcome_ref: row.get(9)?,
        created_at: row.get(10)?,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

fn turn_from_raw(raw: RawTurn) -> anyhow::Result<AgentTurnRecord> {
    Ok(AgentTurnRecord {
        id: AgentTurnId::from_uuid(parse_uuid(&raw.id)?),
        session_id: CollaborationSessionId::from_uuid(parse_uuid(&raw.session_id)?),
        agent_thread_id: AgentThreadId::from_uuid(parse_uuid(&raw.agent_thread_id)?),
        requested_by_agent_thread_id: raw
            .requested_by_agent_thread_id
            .map(|id| parse_uuid(&id).map(AgentThreadId::from_uuid))
            .transpose()?,
        requested_by_turn_id: raw
            .requested_by_turn_id
            .map(|id| parse_uuid(&id).map(AgentTurnId::from_uuid))
            .transpose()?,
        sequence: u64::try_from(raw.sequence)?,
        task_message: raw.task_message,
        status: AgentTurnStatus::parse(&raw.status)?,
        invocation_id: u64::try_from(raw.invocation_id)?,
        outcome_ref: raw.outcome_ref.map(|id| parse_uuid(&id)).transpose()?,
        created_at: parse_time(&raw.created_at)?,
        started_at: parse_optional_time(raw.started_at)?,
        completed_at: parse_optional_time(raw.completed_at)?,
    })
}

fn insert_session_bundle(
    connection: &Connection,
    session: &CollaborationSessionRecord,
    snapshot: &AgentRuntimeSnapshotRecord,
    root: &AgentThreadRecord,
    turn: &AgentTurnRecord,
) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO agent_sessions (id, user_task_id, policy_json, created_at, closed_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            session.id.to_string(),
            session.user_task_id.to_string(),
            serde_json::to_string(&session.policy)?,
            session.created_at.to_rfc3339(),
            session.closed_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    insert_snapshot(connection, snapshot)?;
    insert_thread(connection, root)?;
    insert_turn(connection, turn)?;
    Ok(())
}

fn insert_snapshot(
    connection: &Connection,
    snapshot: &AgentRuntimeSnapshotRecord,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO agent_runtime_snapshots (
            id, session_id, parent_snapshot_id, content_hash, snapshot_json, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            snapshot.id.to_string(),
            snapshot.session_id.to_string(),
            snapshot.parent_snapshot_id.map(|id| id.to_string()),
            snapshot.content_hash,
            serde_json::to_string(&snapshot.snapshot)?,
            snapshot.created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn insert_thread(connection: &Connection, thread: &AgentThreadRecord) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO agent_threads (
            id, session_id, parent_agent_thread_id, agent_path, task_name, agent_type,
            runtime_snapshot_id, spawn_policy_json, created_at, archived_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            thread.id.to_string(),
            thread.session_id.to_string(),
            thread.parent_agent_thread_id.map(|id| id.to_string()),
            thread.path.as_str(),
            thread.task_name,
            thread.agent_type,
            thread.runtime_snapshot_id.to_string(),
            serde_json::to_string(&thread.spawn_policy)?,
            thread.created_at.to_rfc3339(),
            thread.archived_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    Ok(())
}

fn insert_turn(connection: &Connection, turn: &AgentTurnRecord) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO agent_turns (
            id, session_id, agent_thread_id, requested_by_agent_thread_id,
            requested_by_turn_id, sequence, task_message, status, invocation_id,
            outcome_ref, created_at, started_at, completed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            turn.id.to_string(),
            turn.session_id.to_string(),
            turn.agent_thread_id.to_string(),
            turn.requested_by_agent_thread_id.map(|id| id.to_string()),
            turn.requested_by_turn_id.map(|id| id.to_string()),
            i64::try_from(turn.sequence)?,
            turn.task_message,
            turn.status.as_str(),
            i64::try_from(turn.invocation_id)?,
            turn.outcome_ref.map(|id| id.to_string()),
            turn.created_at.to_rfc3339(),
            turn.started_at.map(|value| value.to_rfc3339()),
            turn.completed_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    Ok(())
}

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
            id: AgentTurnId::new(),
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

fn raw_mailbox_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMailboxMessage> {
    Ok(RawMailboxMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        sequence: row.get(2)?,
        from_agent_thread_id: row.get(3)?,
        to_agent_thread_id: row.get(4)?,
        kind: row.get(5)?,
        payload_json: row.get(6)?,
        causation_id: row.get(7)?,
        created_at: row.get(8)?,
        acknowledged_at: row.get(9)?,
    })
}

fn mailbox_from_raw(raw: RawMailboxMessage) -> Result<AgentMailboxMessage, AgentMailboxError> {
    Ok(AgentMailboxMessage {
        id: AgentMailboxMessageId::from_uuid(parse_uuid(&raw.id).map_err(mailbox_persistence)?),
        session_id: CollaborationSessionId::from_uuid(
            parse_uuid(&raw.session_id).map_err(mailbox_persistence)?,
        ),
        sequence: u64::try_from(raw.sequence).map_err(mailbox_persistence)?,
        from_agent_thread_id: AgentThreadId::from_uuid(
            parse_uuid(&raw.from_agent_thread_id).map_err(mailbox_persistence)?,
        ),
        to_agent_thread_id: AgentThreadId::from_uuid(
            parse_uuid(&raw.to_agent_thread_id).map_err(mailbox_persistence)?,
        ),
        kind: AgentMailboxMessageKind::parse(&raw.kind)?,
        payload: serde_json::from_str(&raw.payload_json).map_err(mailbox_persistence)?,
        causation_id: raw
            .causation_id
            .map(|id| parse_uuid(&id).map_err(mailbox_persistence))
            .transpose()?,
        created_at: parse_time(&raw.created_at).map_err(mailbox_persistence)?,
        acknowledged_at: raw
            .acknowledged_at
            .map(|value| parse_time(&value).map_err(mailbox_persistence))
            .transpose()?,
    })
}

fn read_mailbox_message(
    connection: &Connection,
    message_id: AgentMailboxMessageId,
) -> Result<Option<AgentMailboxMessage>, AgentMailboxError> {
    let raw = connection
        .query_row(
            r#"
            SELECT id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                   kind, payload_json, causation_id, created_at, acknowledged_at
            FROM agent_mailbox_messages WHERE id = ?1
            "#,
            params![message_id.to_string()],
            raw_mailbox_message,
        )
        .optional()
        .map_err(mailbox_persistence)?;
    raw.map(mailbox_from_raw).transpose()
}

#[async_trait]
impl AgentMailbox for SqliteCollaborationRepository {
    async fn enqueue(
        &self,
        request: EnqueueAgentMessage,
    ) -> Result<AgentMailboxMessage, AgentMailboxError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let from = read_thread(&transaction, request.from_agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("sender AgentThread was not found"))?;
                let to = read_thread(&transaction, request.to_agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("target AgentThread was not found"))?;
                anyhow::ensure!(
                    from.session_id == request.session_id && to.session_id == request.session_id,
                    "mailbox endpoints must belong to the requested CollaborationSession"
                );
                if let Some(causation_id) = request.causation_id {
                    let existing = transaction
                        .query_row(
                            r#"
                            SELECT id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                                   kind, payload_json, causation_id, created_at, acknowledged_at
                            FROM agent_mailbox_messages
                            WHERE session_id = ?1 AND to_agent_thread_id = ?2
                              AND kind = ?3 AND causation_id = ?4
                            "#,
                            params![
                                request.session_id.to_string(),
                                request.to_agent_thread_id.to_string(),
                                request.kind.as_str(),
                                causation_id.to_string(),
                            ],
                            raw_mailbox_message,
                        )
                        .optional()?;
                    if let Some(existing) = existing {
                        return Ok(mailbox_from_raw(existing).map_err(|error| anyhow::anyhow!(error))?);
                    }
                }
                let next_sequence: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_mailbox_messages WHERE session_id = ?1",
                    params![request.session_id.to_string()],
                    |row| row.get(0),
                )?;
                let message = AgentMailboxMessage {
                    id: AgentMailboxMessageId::new(),
                    session_id: request.session_id,
                    sequence: u64::try_from(next_sequence)?,
                    from_agent_thread_id: request.from_agent_thread_id,
                    to_agent_thread_id: request.to_agent_thread_id,
                    kind: request.kind,
                    payload: request.payload,
                    causation_id: request.causation_id,
                    created_at: Utc::now(),
                    acknowledged_at: None,
                };
                transaction.execute(
                    r#"
                    INSERT INTO agent_mailbox_messages (
                        id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                        kind, payload_json, causation_id, created_at, acknowledged_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
                    "#,
                    params![
                        message.id.to_string(),
                        message.session_id.to_string(),
                        i64::try_from(message.sequence)?,
                        message.from_agent_thread_id.to_string(),
                        message.to_agent_thread_id.to_string(),
                        message.kind.as_str(),
                        serde_json::to_string(&message.payload)?,
                        message.causation_id.map(|id| id.to_string()),
                        message.created_at.to_rfc3339(),
                    ],
                )?;
                transaction.commit()?;
                Ok(message)
            })
            .map_err(mailbox_persistence)
    }

    async fn snapshot(
        &self,
        session_id: CollaborationSessionId,
        target: AgentThreadId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<AgentMailboxMessage>, AgentMailboxError> {
        self.store
            .with_collaboration_read(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                           kind, payload_json, causation_id, created_at, acknowledged_at
                    FROM agent_mailbox_messages
                    WHERE session_id = ?1 AND to_agent_thread_id = ?2
                      AND acknowledged_at IS NULL AND sequence > ?3
                    ORDER BY sequence LIMIT ?4
                    "#,
                )?;
                let rows = statement.query_map(
                    params![
                        session_id.to_string(),
                        target.to_string(),
                        i64::try_from(after_sequence.unwrap_or(0))?,
                        i64::try_from(limit.clamp(1, 256))?,
                    ],
                    raw_mailbox_message,
                )?;
                let mut messages = Vec::new();
                for row in rows {
                    messages.push(mailbox_from_raw(row?).map_err(|error| anyhow::anyhow!(error))?);
                }
                Ok(messages)
            })
            .map_err(mailbox_persistence)
    }

    async fn acknowledge(
        &self,
        target: AgentThreadId,
        message_ids: &[AgentMailboxMessageId],
    ) -> Result<(), AgentMailboxError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                for message_id in message_ids {
                    let message = read_mailbox_message(&transaction, *message_id)
                        .map_err(|error| anyhow::anyhow!(error))?
                        .ok_or_else(|| anyhow::anyhow!("mailbox message was not found"))?;
                    anyhow::ensure!(
                        message.to_agent_thread_id == target,
                        "mailbox message belongs to another target"
                    );
                    transaction.execute(
                        "UPDATE agent_mailbox_messages SET acknowledged_at = COALESCE(acknowledged_at, ?2) WHERE id = ?1",
                        params![message.id.to_string(), Utc::now().to_rfc3339()],
                    )?;
                }
                transaction.commit()?;
                Ok(())
            })
            .map_err(mailbox_persistence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{AgentSpawnPolicy, CollaborationSessionPolicy, RuntimeSnapshotSeed};
    use crate::store::SessionStore;
    use serde_json::json;
    use std::path::PathBuf;

    async fn fixture() -> (
        Arc<SqliteSessionStore>,
        SqliteCollaborationRepository,
        CollaborationSessionRecord,
        AgentThreadRecord,
        AgentTurnRecord,
    ) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
        let user_thread = store
            .create_thread(None, PathBuf::from("C:/workspace/collaboration-sqlite"))
            .unwrap();
        let repository = SqliteCollaborationRepository::new(store.clone()).unwrap();
        let (session, root, root_turn) = repository
            .create_session(CreateCollaborationSession {
                user_task_id: user_thread.id,
                root_task_message: "root task".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(
                    None,
                    json!({ "agentType": "default" }),
                ),
                session_policy: CollaborationSessionPolicy {
                    max_agents: 8,
                    max_active_runs: 4,
                    max_depth: 2,
                },
                root_spawn_policy: AgentSpawnPolicy::allows_children(2, 4),
            })
            .await
            .unwrap();
        (store, repository, session, root, root_turn)
    }

    #[tokio::test]
    async fn sqlite_registry_persists_recursive_identity_turns_and_snapshots() {
        let (_store, repository, session, root, root_turn) = fixture().await;
        let (child, child_turn) = repository
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    json!({ "agentType": "explorer" }),
                ),
                spawn_policy: AgentSpawnPolicy::allows_children(2, 2),
            })
            .await
            .unwrap();
        let (grandchild, _) = repository
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: child.id,
                requested_by_turn_id: child_turn.id,
                task_name: "reviewer".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "review".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(child.runtime_snapshot_id),
                    json!({ "agentType": "explorer", "role": "reviewer" }),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(2),
            })
            .await
            .unwrap();

        assert_eq!(grandchild.path.as_str(), "/root/research/reviewer");
        assert_eq!(repository.list_threads(session.id).await.unwrap().len(), 3);
        assert_eq!(
            repository
                .get_runtime_snapshot(child.runtime_snapshot_id)
                .await
                .unwrap()
                .parent_snapshot_id,
            Some(root.runtime_snapshot_id)
        );
    }

    #[tokio::test]
    async fn sqlite_mailbox_is_idempotent_and_acknowledged_transactionally() {
        let (_store, repository, session, root, root_turn) = fixture().await;
        let (child, _) = repository
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "worker".to_string(),
                agent_type: "worker".to_string(),
                task_message: "work".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    json!({ "agentType": "worker" }),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(2),
            })
            .await
            .unwrap();
        let causation_id = Uuid::new_v4();
        let request = || EnqueueAgentMessage {
            session_id: session.id,
            from_agent_thread_id: root.id,
            to_agent_thread_id: child.id,
            kind: AgentMailboxMessageKind::Message,
            payload: json!({ "text": "hello" }),
            causation_id: Some(causation_id),
        };
        let first = repository.enqueue(request()).await.unwrap();
        let duplicate = repository.enqueue(request()).await.unwrap();
        assert_eq!(first.id, duplicate.id);

        repository.acknowledge(child.id, &[first.id]).await.unwrap();
        assert!(repository
            .snapshot(session.id, child.id, None, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn sqlite_collaboration_state_survives_store_reopen() {
        let path =
            std::env::temp_dir().join(format!("opentopia-collaboration-{}.sqlite", Uuid::new_v4()));
        let (session_id, child_id, message_id) = {
            let store = Arc::new(SqliteSessionStore::open(&path).unwrap());
            let user_thread = store
                .create_thread(None, PathBuf::from("C:/workspace/collaboration-reopen"))
                .unwrap();
            let repository = SqliteCollaborationRepository::new(store.clone()).unwrap();
            let (session, root, root_turn) = repository
                .create_session(CreateCollaborationSession {
                    user_task_id: user_thread.id,
                    root_task_message: "root task".to_string(),
                    root_agent_type: "default".to_string(),
                    root_runtime_snapshot: RuntimeSnapshotSeed::new(
                        None,
                        json!({ "agentType": "default" }),
                    ),
                    session_policy: CollaborationSessionPolicy {
                        max_agents: 4,
                        max_active_runs: 2,
                        max_depth: 1,
                    },
                    root_spawn_policy: AgentSpawnPolicy::allows_children(1, 2),
                })
                .await
                .unwrap();
            let (child, _) = repository
                .spawn_agent(SpawnAgentThread {
                    parent_agent_thread_id: root.id,
                    requested_by_turn_id: root_turn.id,
                    task_name: "worker".to_string(),
                    agent_type: "worker".to_string(),
                    task_message: "work".to_string(),
                    runtime_snapshot: RuntimeSnapshotSeed::new(
                        Some(root.runtime_snapshot_id),
                        json!({ "agentType": "worker" }),
                    ),
                    spawn_policy: AgentSpawnPolicy::disabled(1),
                })
                .await
                .unwrap();
            let message = repository
                .enqueue(EnqueueAgentMessage {
                    session_id: session.id,
                    from_agent_thread_id: root.id,
                    to_agent_thread_id: child.id,
                    kind: AgentMailboxMessageKind::Message,
                    payload: json!({ "text": "persist me" }),
                    causation_id: Some(Uuid::new_v4()),
                })
                .await
                .unwrap();
            (session.id, child.id, message.id)
        };

        {
            let reopened_store = Arc::new(SqliteSessionStore::open(&path).unwrap());
            let repository = SqliteCollaborationRepository::new(reopened_store).unwrap();
            assert_eq!(repository.get_thread(child_id).await.unwrap().id, child_id);
            let pending = repository
                .snapshot(session_id, child_id, None, 10)
                .await
                .unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].id, message_id);
        }

        let _ = std::fs::remove_file(path);
    }
}
