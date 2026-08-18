use super::{
    AgentMailboxMessage, AgentMailboxMessageId, AgentMailboxMessageKind, AgentPath,
    AgentRuntimeSnapshotRecord, AgentThreadId, AgentThreadRecord, AgentTurnId, AgentTurnRecord,
    AgentTurnStatus, CollaborationDomainError, CollaborationSessionId, CollaborationSessionRecord,
    RuntimeSnapshotSeed,
};
use crate::store::SqliteSessionStore;
use chrono::Utc;
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::sync::Arc;
use uuid::Uuid;

mod activity_repository;
mod mailbox_repository;
mod record_mapping;
mod registry_repository;

#[allow(unused_imports)]
pub(crate) use activity_repository::SqliteActivitySummary;
use mailbox_repository::{mailbox_from_raw, raw_mailbox_message};
use record_mapping::{
    insert_snapshot, insert_turn, raw_turn, read_latest_turn, read_session, read_thread,
    read_thread_by_path, read_turn, turn_from_raw,
};

#[derive(Clone)]
pub struct SqliteCollaborationRepository {
    pub(super) store: Arc<SqliteSessionStore>,
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
                    "agent_events",
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

    pub fn find_session_by_user_task_id(
        &self,
        user_task_id: Uuid,
    ) -> Result<Option<CollaborationSessionRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                let id = connection
                    .query_row(
                        "SELECT id FROM agent_sessions WHERE user_task_id = ?1",
                        params![user_task_id.to_string()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                id.map(|id| {
                    read_session(
                        connection,
                        CollaborationSessionId::from_uuid(Uuid::parse_str(&id)?),
                    )?
                    .ok_or_else(|| anyhow::anyhow!("CollaborationSession was not found"))
                })
                .transpose()
            })
            .map_err(registry_persistence)
    }

    pub fn find_turn(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<Option<AgentTurnRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| read_turn(connection, turn_id))
            .map_err(registry_persistence)
    }

    pub fn list_recoverable_turns(&self) -> Result<Vec<AgentTurnRecord>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, session_id, agent_thread_id, requested_by_agent_thread_id,
                           requested_by_turn_id, sequence, task_message, status, invocation_id,
                           outcome_ref, created_at, started_at, completed_at
                    FROM agent_turns
                    WHERE status IN ('queued', 'running')
                    ORDER BY created_at
                    "#,
                )?;
                let rows = statement.query_map([], raw_turn)?;
                let mut turns = Vec::new();
                for row in rows {
                    turns.push(turn_from_raw(row?)?);
                }
                Ok(turns)
            })
            .map_err(registry_persistence)
    }

    pub fn create_root_followup_turn(
        &self,
        session_id: CollaborationSessionId,
        turn_id: AgentTurnId,
        task_message: &str,
        mut runtime_snapshot: RuntimeSnapshotSeed,
    ) -> Result<(AgentThreadRecord, AgentTurnRecord), CollaborationDomainError> {
        let task_message = task_message.trim();
        if task_message.is_empty() {
            return Err(CollaborationDomainError::EmptyTaskMessage);
        }
        runtime_snapshot.validate()?;
        if runtime_snapshot.parent_snapshot_id.is_some() {
            return Err(CollaborationDomainError::InvalidRuntimeSnapshot(
                "root follow-up snapshot parent is assigned transactionally".to_string(),
            ));
        }
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut root = read_thread_by_path(&transaction, session_id, &AgentPath::root())?
                    .ok_or_else(|| anyhow::anyhow!("root AgentThread was not found"))?;
                let latest = read_latest_turn(&transaction, root.id)?
                    .ok_or_else(|| anyhow::anyhow!("root AgentTurn was not found"))?;
                if !latest.status.is_terminal() {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::AgentTurnAlreadyActive(root.id)
                    ));
                }
                let now = Utc::now();
                runtime_snapshot.parent_snapshot_id = Some(root.runtime_snapshot_id);
                let snapshot = AgentRuntimeSnapshotRecord {
                    id: runtime_snapshot.id,
                    session_id,
                    parent_snapshot_id: runtime_snapshot.parent_snapshot_id,
                    content_hash: runtime_snapshot.content_hash,
                    snapshot: runtime_snapshot.snapshot,
                    created_at: now,
                };
                insert_snapshot(&transaction, &snapshot)?;
                root.runtime_snapshot_id = snapshot.id;
                transaction.execute(
                    "UPDATE agent_threads SET runtime_snapshot_id = ?2 WHERE id = ?1",
                    params![root.id.to_string(), root.runtime_snapshot_id.to_string()],
                )?;
                let turn = AgentTurnRecord {
                    id: turn_id,
                    session_id,
                    agent_thread_id: root.id,
                    requested_by_agent_thread_id: Some(root.id),
                    requested_by_turn_id: Some(latest.id),
                    sequence: latest.sequence + 1,
                    task_message: task_message.to_string(),
                    status: AgentTurnStatus::Queued,
                    invocation_id: 1,
                    outcome_ref: None,
                    created_at: now,
                    started_at: None,
                    completed_at: None,
                };
                insert_turn(&transaction, &turn)?;
                transaction.commit()?;
                Ok((root, turn))
            })
            .map_err(registry_persistence)
    }

    pub fn resume_turn(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<AgentTurnRecord, CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut turn = read_turn(&transaction, turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentTurn was not found"))?;
                anyhow::ensure!(
                    turn.status.needs_attention(),
                    "AgentTurn is not waiting for a resumable interaction"
                );
                turn.transition(AgentTurnStatus::Running, Utc::now())?;
                turn.invocation_id = turn.invocation_id.saturating_add(1);
                transaction.execute(
                    r#"
                    UPDATE agent_turns
                    SET status = 'running', invocation_id = ?2, completed_at = NULL
                    WHERE id = ?1
                    "#,
                    params![turn.id.to_string(), i64::try_from(turn.invocation_id)?],
                )?;
                transaction.commit()?;
                Ok(turn)
            })
            .map_err(registry_persistence)
    }

    pub fn append_ledger_item(
        &self,
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        item_kind: &str,
        payload: &serde_json::Value,
    ) -> Result<Uuid, CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let sequence: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_ledger_items WHERE agent_thread_id = ?1",
                    params![agent_thread_id.to_string()],
                    |row| row.get(0),
                )?;
                let id = Uuid::new_v4();
                transaction.execute(
                    r#"
                    INSERT INTO agent_ledger_items (
                        id, session_id, agent_thread_id, agent_turn_id,
                        sequence, item_kind, payload_json, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                    params![
                        id.to_string(),
                        session_id.to_string(),
                        agent_thread_id.to_string(),
                        agent_turn_id.to_string(),
                        sequence,
                        item_kind,
                        serde_json::to_string(payload)?,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
                transaction.commit()?;
                Ok(id)
            })
            .map_err(registry_persistence)
    }

    pub fn list_ledger_items(
        &self,
        agent_thread_id: AgentThreadId,
        item_kind: &str,
    ) -> Result<Vec<serde_json::Value>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT payload_json FROM agent_ledger_items
                    WHERE agent_thread_id = ?1 AND item_kind = ?2
                    ORDER BY sequence
                    "#,
                )?;
                let rows = statement
                    .query_map(params![agent_thread_id.to_string(), item_kind], |row| {
                        row.get::<_, String>(0)
                    })?;
                let mut values = Vec::new();
                for row in rows {
                    values.push(serde_json::from_str(&row?)?);
                }
                Ok(values)
            })
            .map_err(registry_persistence)
    }

    /// Persists the Turn outcome, its ledger reference, and the parent mailbox
    /// envelope in one transaction. Waiters can therefore never observe a
    /// terminal child without its completion fact already being durable.
    pub fn record_turn_state(
        &self,
        turn_id: AgentTurnId,
        next: AgentTurnStatus,
        payload: &serde_json::Value,
    ) -> Result<Option<AgentMailboxMessage>, CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut turn = read_turn(&transaction, turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentTurn was not found"))?;
                if turn.status != next {
                    turn.transition(next, Utc::now())?;
                }
                let ledger_sequence: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_ledger_items WHERE agent_thread_id = ?1",
                    params![turn.agent_thread_id.to_string()],
                    |row| row.get(0),
                )?;
                let outcome_id = turn.outcome_ref.unwrap_or_else(Uuid::new_v4);
                transaction.execute(
                    r#"
                    INSERT INTO agent_ledger_items (
                        id, session_id, agent_thread_id, agent_turn_id,
                        sequence, item_kind, payload_json, created_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, 'turn_outcome', ?6, ?7)
                    ON CONFLICT(id) DO UPDATE SET
                        payload_json = excluded.payload_json
                    "#,
                    params![
                        outcome_id.to_string(),
                        turn.session_id.to_string(),
                        turn.agent_thread_id.to_string(),
                        turn.id.to_string(),
                        ledger_sequence,
                        serde_json::to_string(payload)?,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
                turn.outcome_ref = Some(outcome_id);

                let agent = read_thread(&transaction, turn.agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentThread was not found"))?;
                let mailbox_kind = if next.is_terminal() {
                    Some(AgentMailboxMessageKind::Completion)
                } else if next.needs_attention() {
                    Some(AgentMailboxMessageKind::NeedsAttention)
                } else {
                    None
                };
                let mut envelope = None;
                if let (Some(parent), Some(kind)) = (agent.parent_agent_thread_id, mailbox_kind) {
                    let existing = transaction
                        .query_row(
                            r#"
                            SELECT id, session_id, sequence, from_agent_thread_id,
                                   to_agent_thread_id, kind, payload_json, causation_id,
                                   created_at, delivered_at, acknowledged_at
                            FROM agent_mailbox_messages
                            WHERE session_id = ?1 AND to_agent_thread_id = ?2
                              AND kind = ?3 AND causation_id = ?4
                            "#,
                            params![
                                turn.session_id.to_string(),
                                parent.to_string(),
                                kind.as_str(),
                                turn.id.to_string(),
                            ],
                            raw_mailbox_message,
                        )
                        .optional()?;
                    envelope = match existing {
                        Some(existing) => Some(
                            mailbox_from_raw(existing).map_err(|error| anyhow::anyhow!(error))?,
                        ),
                        None => {
                            let sequence: i64 = transaction.query_row(
                                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_mailbox_messages WHERE session_id = ?1",
                                params![turn.session_id.to_string()],
                                |row| row.get(0),
                            )?;
                            let message = AgentMailboxMessage {
                                id: AgentMailboxMessageId::new(),
                                session_id: turn.session_id,
                                sequence: u64::try_from(sequence)?,
                                from_agent_thread_id: turn.agent_thread_id,
                                to_agent_thread_id: parent,
                                kind,
                                payload: payload.clone(),
                                causation_id: Some(turn.id.as_uuid()),
                                created_at: Utc::now(),
                                delivered_at: None,
                                acknowledged_at: None,
                            };
                            transaction.execute(
                                r#"
                                INSERT INTO agent_mailbox_messages (
                                    id, session_id, sequence, from_agent_thread_id,
                                    to_agent_thread_id, kind, payload_json, causation_id,
                                    created_at, delivery_state, delivered_at, acknowledged_at
                                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', NULL, NULL)
                                "#,
                                params![
                                    message.id.to_string(),
                                    message.session_id.to_string(),
                                    i64::try_from(message.sequence)?,
                                    message.from_agent_thread_id.to_string(),
                                    message.to_agent_thread_id.to_string(),
                                    message.kind.as_str(),
                                    serde_json::to_string(&message.payload)?,
                                    turn.id.to_string(),
                                    message.created_at.to_rfc3339(),
                                ],
                            )?;
                            Some(message)
                        }
                    };
                }
                transaction.execute(
                    r#"
                    UPDATE agent_turns
                    SET status = ?2, outcome_ref = ?3, started_at = ?4, completed_at = ?5
                    WHERE id = ?1
                    "#,
                    params![
                        turn.id.to_string(),
                        turn.status.as_str(),
                        outcome_id.to_string(),
                        turn.started_at.map(|value| value.to_rfc3339()),
                        turn.completed_at.map(|value| value.to_rfc3339()),
                    ],
                )?;
                transaction.commit()?;
                Ok(envelope)
            })
            .map_err(registry_persistence)
    }

    pub fn put_turn_checkpoint(
        &self,
        turn_id: AgentTurnId,
        wait_kind: &str,
        continuation: &serde_json::Value,
    ) -> Result<(), CollaborationDomainError> {
        if !matches!(wait_kind, "approval" | "user_input" | "external_action") {
            return Err(CollaborationDomainError::Persistence(format!(
                "unsupported checkpoint kind `{wait_kind}`"
            )));
        }
        self.store
            .with_collaboration_write(|connection| {
                let now = Utc::now().to_rfc3339();
                connection.execute(
                    r#"
                    INSERT INTO agent_turn_checkpoints (
                        agent_turn_id, wait_kind, continuation_json, created_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?4)
                    ON CONFLICT(agent_turn_id) DO UPDATE SET
                        wait_kind = excluded.wait_kind,
                        continuation_json = excluded.continuation_json,
                        updated_at = excluded.updated_at
                    "#,
                    params![
                        turn_id.to_string(),
                        wait_kind,
                        serde_json::to_string(continuation)?,
                        now,
                    ],
                )?;
                Ok(())
            })
            .map_err(registry_persistence)
    }

    pub fn get_turn_checkpoint(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<Option<(String, serde_json::Value)>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                connection
                    .query_row(
                        "SELECT wait_kind, continuation_json FROM agent_turn_checkpoints WHERE agent_turn_id = ?1",
                        params![turn_id.to_string()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?
                    .map(|(kind, value)| Ok((kind, serde_json::from_str(&value)?)))
                    .transpose()
            })
            .map_err(registry_persistence)
    }

    pub fn delete_turn_checkpoint(
        &self,
        turn_id: AgentTurnId,
    ) -> Result<(), CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                connection.execute(
                    "DELETE FROM agent_turn_checkpoints WHERE agent_turn_id = ?1",
                    params![turn_id.to_string()],
                )?;
                Ok(())
            })
            .map_err(registry_persistence)
    }

    pub fn load_provider_state(
        &self,
        agent_thread_id: AgentThreadId,
        provider_id: &str,
    ) -> Result<Option<(String, String, serde_json::Value)>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                connection
                    .query_row(
                        r#"
                        SELECT model, compatibility_hash, state_json
                        FROM agent_provider_states
                        WHERE agent_thread_id = ?1 AND provider_id = ?2
                        "#,
                        params![agent_thread_id.to_string(), provider_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?
                    .map(|(model, hash, state)| Ok((model, hash, serde_json::from_str(&state)?)))
                    .transpose()
            })
            .map_err(registry_persistence)
    }

    pub fn save_provider_state(
        &self,
        agent_thread_id: AgentThreadId,
        provider_id: &str,
        model: &str,
        response_id: &str,
        compatibility_hash: &str,
        state: &serde_json::Value,
    ) -> Result<(), CollaborationDomainError> {
        self.store
            .with_collaboration_write(|connection| {
                connection.execute(
                    r#"
                    INSERT INTO agent_provider_states (
                        agent_thread_id, provider_id, model, response_id,
                        compatibility_hash, state_json, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    ON CONFLICT(agent_thread_id, provider_id) DO UPDATE SET
                        model = excluded.model,
                        response_id = excluded.response_id,
                        compatibility_hash = excluded.compatibility_hash,
                        state_json = excluded.state_json,
                        updated_at = excluded.updated_at
                    "#,
                    params![
                        agent_thread_id.to_string(),
                        provider_id,
                        model,
                        response_id,
                        compatibility_hash,
                        serde_json::to_string(state)?,
                        Utc::now().to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .map_err(registry_persistence)
    }
}

pub(super) fn registry_persistence(error: anyhow::Error) -> CollaborationDomainError {
    match error.downcast::<CollaborationDomainError>() {
        Ok(domain) => domain,
        Err(error) => CollaborationDomainError::Persistence(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
