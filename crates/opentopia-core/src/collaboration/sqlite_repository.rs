use super::domain::validate_task_name;
use super::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageId,
    AgentMailboxMessageKind, AgentPath, AgentRuntimeSnapshotRecord, AgentThreadId,
    AgentThreadRecord, AgentTurnId, AgentTurnRecord, AgentTurnStatus, CollaborationDomainError,
    CollaborationRegistry, CollaborationSessionId, CollaborationSessionRecord,
    CreateCollaborationSession, EnqueueAgentMessage, FollowupAgentTurn, RuntimeSnapshotId,
    SpawnAgentThread,
};
use crate::model::{AgentEvent, AgentEventPayload};
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

const MAX_ACTIVITY_REASONING_TAIL_CHARS: usize = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqliteActivitySummary {
    pub cursor: i64,
    pub model_round: Option<usize>,
    pub reasoning_tail: Option<String>,
}

#[derive(Debug, Clone)]
struct ActivityStateRow {
    session_id: CollaborationSessionId,
    cursor: i64,
    model_round: Option<usize>,
    round_boundary: Option<i64>,
    reasoning_tail: String,
    updated_at: DateTime<Utc>,
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
    ) -> Result<(AgentThreadRecord, AgentTurnRecord), CollaborationDomainError> {
        let task_message = task_message.trim();
        if task_message.is_empty() {
            return Err(CollaborationDomainError::EmptyTaskMessage);
        }
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let root = read_thread_by_path(&transaction, session_id, &AgentPath::root())?
                    .ok_or_else(|| anyhow::anyhow!("root AgentThread was not found"))?;
                let latest = read_latest_turn(&transaction, root.id)?
                    .ok_or_else(|| anyhow::anyhow!("root AgentTurn was not found"))?;
                if !latest.status.is_terminal() {
                    return Err(anyhow::anyhow!(
                        CollaborationDomainError::AgentTurnAlreadyActive(root.id)
                    ));
                }
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
                    created_at: Utc::now(),
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

    pub fn append_activity_event(
        &self,
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        invocation_id: u64,
        payload: AgentEventPayload,
        causation_id: Option<Uuid>,
    ) -> Result<AgentEvent, CollaborationDomainError> {
        self.append_activity_events(
            session_id,
            agent_thread_id,
            agent_turn_id,
            invocation_id,
            vec![payload],
            causation_id,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| {
            registry_persistence(anyhow::anyhow!("single-event append returned no event"))
        })
    }

    pub fn append_activity_events(
        &self,
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        invocation_id: u64,
        payloads: Vec<AgentEventPayload>,
        causation_id: Option<Uuid>,
    ) -> Result<Vec<AgentEvent>, CollaborationDomainError> {
        if payloads.is_empty() {
            return Ok(Vec::new());
        }
        self.store
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let thread = read_thread(&transaction, agent_thread_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentThread was not found"))?;
                let turn = read_turn(&transaction, agent_turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("AgentTurn was not found"))?;
                anyhow::ensure!(
                    thread.session_id == session_id
                        && turn.session_id == session_id
                        && turn.agent_thread_id == agent_thread_id,
                    "activity event identity does not belong to one Agent Turn"
                );
                let first_event_seq: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(event_seq), 0) + 1 FROM agent_events WHERE session_id = ?1",
                    params![session_id.to_string()],
                    |row| row.get(0),
                )?;
                let mut events = Vec::with_capacity(payloads.len());
                for (offset, payload) in payloads.into_iter().enumerate() {
                    let event = AgentEvent {
                        id: Uuid::new_v4(),
                        thread_id: agent_thread_id.as_uuid(),
                        turn_id: Some(agent_turn_id.as_uuid()),
                        seq: first_event_seq + i64::try_from(offset)?,
                        created_at: Utc::now(),
                        payload,
                    };
                    transaction.execute(
                        r#"
                        INSERT INTO agent_events (
                            id, session_id, event_seq, agent_thread_id, agent_turn_id,
                            invocation_id, event_kind, payload_json, causation_id, created_at
                        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                        "#,
                        params![
                            event.id.to_string(),
                            session_id.to_string(),
                            event.seq,
                            agent_thread_id.to_string(),
                            agent_turn_id.to_string(),
                            i64::try_from(invocation_id.max(1))?,
                            event.kind(),
                            serde_json::to_string(&event.payload)?,
                            causation_id.map(|id| id.to_string()),
                            event.created_at.to_rfc3339(),
                        ],
                    )?;
                    events.push(event);
                }
                update_activity_state(
                    &transaction,
                    session_id,
                    agent_thread_id,
                    agent_turn_id,
                    &events,
                )?;
                transaction.commit()?;
                Ok(events)
            })
            .map_err(registry_persistence)
    }

    pub fn list_activity_events(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        after_cursor: Option<i64>,
    ) -> Result<Vec<AgentEvent>, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                let mut statement = connection.prepare(
                    r#"
                    SELECT id, event_seq, payload_json, created_at
                    FROM agent_events
                    WHERE agent_thread_id = ?1 AND agent_turn_id = ?2 AND event_seq > ?3
                    ORDER BY event_seq
                    "#,
                )?;
                let rows = statement.query_map(
                    params![
                        agent_thread_id.to_string(),
                        agent_turn_id.to_string(),
                        after_cursor.unwrap_or(0),
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?;
                let mut events = Vec::new();
                for row in rows {
                    let (id, seq, payload, created_at) = row?;
                    events.push(AgentEvent {
                        id: Uuid::parse_str(&id)?,
                        thread_id: agent_thread_id.as_uuid(),
                        turn_id: Some(agent_turn_id.as_uuid()),
                        seq,
                        created_at: parse_time(&created_at)?,
                        payload: serde_json::from_str(&payload)?,
                    });
                }
                Ok(events)
            })
            .map_err(registry_persistence)
    }

    pub(crate) fn latest_activity_cursor(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
    ) -> Result<i64, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                latest_activity_cursor(connection, agent_thread_id, agent_turn_id)
            })
            .map_err(registry_persistence)
    }

    pub fn latest_session_activity_cursor(
        &self,
        session_id: CollaborationSessionId,
    ) -> Result<i64, CollaborationDomainError> {
        self.store
            .with_collaboration_read(|connection| {
                let projected: i64 = connection.query_row(
                    "SELECT COALESCE(MAX(cursor), 0) FROM agent_activity_state WHERE session_id = ?1",
                    params![session_id.to_string()],
                    |row| row.get(0),
                )?;
                let durable: i64 = connection.query_row(
                    "SELECT COALESCE(MAX(event_seq), 0) FROM agent_events WHERE session_id = ?1",
                    params![session_id.to_string()],
                    |row| row.get(0),
                )?;
                Ok(projected.max(durable))
            })
            .map_err(registry_persistence)
    }

    pub(crate) fn activity_summary(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        after_cursor: Option<i64>,
        reasoning_tail_chars: usize,
    ) -> Result<SqliteActivitySummary, CollaborationDomainError> {
        let (mut state, cursor) = self
            .store
            .with_collaboration_read(|connection| {
                let state = read_activity_state(connection, agent_thread_id, agent_turn_id)?;
                let cursor = latest_activity_cursor(connection, agent_thread_id, agent_turn_id)?;
                Ok((state, cursor))
            })
            .map_err(registry_persistence)?;
        if state.as_ref().is_none_or(|state| state.cursor < cursor) {
            state = Some(
                self.store
                    .with_collaboration_write(|connection| {
                        let state =
                            rebuild_activity_state(connection, agent_thread_id, agent_turn_id)?;
                        write_activity_state(connection, agent_thread_id, agent_turn_id, &state)?;
                        Ok(state)
                    })
                    .map_err(registry_persistence)?,
            );
        }
        let state = state.unwrap_or_else(|| ActivityStateRow {
            session_id: CollaborationSessionId::from_uuid(Uuid::nil()),
            cursor: 0,
            model_round: None,
            round_boundary: None,
            reasoning_tail: String::new(),
            updated_at: Utc::now(),
        });
        let reasoning_start = after_cursor
            .unwrap_or(i64::MIN)
            .max(state.round_boundary.unwrap_or(i64::MIN));
        let reasoning_tail = if reasoning_start <= state.round_boundary.unwrap_or(i64::MIN) {
            tail_chars(&state.reasoning_tail, reasoning_tail_chars)
        } else {
            self.store
                .with_collaboration_read(|connection| {
                    read_reasoning_tail(
                        connection,
                        agent_thread_id,
                        agent_turn_id,
                        reasoning_start,
                        reasoning_tail_chars,
                    )
                })
                .map_err(registry_persistence)?
        };
        Ok(SqliteActivitySummary {
            cursor: cursor.max(state.cursor),
            model_round: state.model_round,
            reasoning_tail: (!reasoning_tail.is_empty()).then_some(reasoning_tail),
        })
    }

    pub(crate) fn list_projectable_activity_events(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        after_cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<AgentEvent>, CollaborationDomainError> {
        self.list_bounded_activity_events(
            agent_thread_id,
            agent_turn_id,
            after_cursor,
            limit,
            r#"
                event_kind IN (
                    'model_context_built', 'model_request',
                    'tool_call_started', 'tool_call_finished',
                    'turn_suspended', 'turn_awaiting_input', 'error',
                    'turn_started', 'turn_finished', 'turn_cancelled'
                )
            "#,
        )
    }

    pub(crate) fn list_activity_tool_result_events(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        after_cursor: Option<i64>,
        limit: usize,
    ) -> Result<Vec<AgentEvent>, CollaborationDomainError> {
        self.list_bounded_activity_events(
            agent_thread_id,
            agent_turn_id,
            after_cursor,
            limit,
            "event_kind = 'tool_call_finished'",
        )
    }

    fn list_bounded_activity_events(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        after_cursor: Option<i64>,
        limit: usize,
        predicate: &str,
    ) -> Result<Vec<AgentEvent>, CollaborationDomainError> {
        let limit = limit.clamp(1, 64);
        self.store
            .with_collaboration_read(|connection| {
                let sql = format!(
                    r#"
                    SELECT id, event_seq, payload_json, created_at
                    FROM agent_events
                    WHERE agent_thread_id = ?1
                      AND agent_turn_id = ?2
                      AND event_seq > ?3
                      AND {predicate}
                    ORDER BY event_seq DESC
                    LIMIT ?4
                    "#
                );
                let mut statement = connection.prepare(&sql)?;
                let rows = statement.query_map(
                    params![
                        agent_thread_id.to_string(),
                        agent_turn_id.to_string(),
                        after_cursor.unwrap_or(0),
                        i64::try_from(limit)?,
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )?;
                let mut events = Vec::new();
                for row in rows {
                    let (id, seq, payload, created_at) = row?;
                    events.push(AgentEvent {
                        id: Uuid::parse_str(&id)?,
                        thread_id: agent_thread_id.as_uuid(),
                        turn_id: Some(agent_turn_id.as_uuid()),
                        seq,
                        created_at: parse_time(&created_at)?,
                        payload: serde_json::from_str(&payload)?,
                    });
                }
                events.reverse();
                Ok(events)
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

fn latest_activity_cursor(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
) -> anyhow::Result<i64> {
    let projected: Option<i64> = connection
        .query_row(
            r#"
            SELECT cursor
            FROM agent_activity_state
            WHERE agent_thread_id = ?1 AND agent_turn_id = ?2
            "#,
            params![agent_thread_id.to_string(), agent_turn_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let durable: i64 = connection.query_row(
        r#"
        SELECT COALESCE(MAX(event_seq), 0)
        FROM agent_events
        WHERE agent_thread_id = ?1 AND agent_turn_id = ?2
        "#,
        params![agent_thread_id.to_string(), agent_turn_id.to_string()],
        |row| row.get(0),
    )?;
    Ok(projected.unwrap_or_default().max(durable))
}

fn read_activity_state(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
) -> anyhow::Result<Option<ActivityStateRow>> {
    connection
        .query_row(
            r#"
            SELECT session_id, cursor, model_round, round_boundary,
                   reasoning_tail, updated_at
            FROM agent_activity_state
            WHERE agent_thread_id = ?1 AND agent_turn_id = ?2
            "#,
            params![agent_thread_id.to_string(), agent_turn_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?
        .map(
            |(session_id, cursor, model_round, round_boundary, reasoning_tail, updated_at)| {
                Ok(ActivityStateRow {
                    session_id: CollaborationSessionId::from_uuid(Uuid::parse_str(&session_id)?),
                    cursor,
                    model_round: model_round.map(usize::try_from).transpose()?,
                    round_boundary,
                    reasoning_tail,
                    updated_at: parse_time(&updated_at)?,
                })
            },
        )
        .transpose()
}

fn rebuild_activity_state(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
) -> anyhow::Result<ActivityStateRow> {
    let session_id: String = connection.query_row(
        "SELECT session_id FROM agent_turns WHERE id = ?1",
        params![agent_turn_id.to_string()],
        |row| row.get(0),
    )?;
    let latest: Option<(i64, String)> = connection
        .query_row(
            r#"
            SELECT event_seq, created_at
            FROM agent_events
            WHERE agent_thread_id = ?1 AND agent_turn_id = ?2
            ORDER BY event_seq DESC
            LIMIT 1
            "#,
            params![agent_thread_id.to_string(), agent_turn_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let round_event: Option<(i64, String)> = connection
        .query_row(
            r#"
            SELECT event_seq, payload_json
            FROM agent_events
            WHERE agent_thread_id = ?1
              AND agent_turn_id = ?2
              AND event_kind IN (
                  'model_context_built', 'model_request',
                  'provider_request_sent', 'provider_request_retried',
                  'provider_response_received'
              )
            ORDER BY event_seq DESC
            LIMIT 1
            "#,
            params![agent_thread_id.to_string(), agent_turn_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (model_round, round_boundary) = if let Some((seq, payload)) = round_event {
        let payload: AgentEventPayload = serde_json::from_str(&payload)?;
        (activity_model_round(&payload), Some(seq))
    } else {
        (None, None)
    };
    let reasoning_tail = read_reasoning_tail(
        connection,
        agent_thread_id,
        agent_turn_id,
        round_boundary.unwrap_or(0),
        MAX_ACTIVITY_REASONING_TAIL_CHARS,
    )?;
    let (cursor, updated_at) = if let Some((cursor, updated_at)) = latest {
        (cursor, parse_time(&updated_at)?)
    } else {
        (0, Utc::now())
    };
    Ok(ActivityStateRow {
        session_id: CollaborationSessionId::from_uuid(Uuid::parse_str(&session_id)?),
        cursor,
        model_round,
        round_boundary,
        reasoning_tail,
        updated_at,
    })
}

fn update_activity_state(
    connection: &Connection,
    session_id: CollaborationSessionId,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
    events: &[AgentEvent],
) -> anyhow::Result<()> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let existing = read_activity_state(connection, agent_thread_id, agent_turn_id)?;
    let mut state = if existing
        .as_ref()
        .is_none_or(|state| state.cursor < first.seq - 1)
    {
        rebuild_activity_state(connection, agent_thread_id, agent_turn_id)?
    } else {
        existing.expect("activity state was checked above")
    };
    if state.cursor < first.seq {
        let previous_cursor = state.cursor;
        for event in events.iter().filter(|event| event.seq > previous_cursor) {
            if let Some(round) = activity_model_round(&event.payload) {
                state.model_round = Some(round);
                state.round_boundary = Some(event.seq);
                state.reasoning_tail.clear();
            } else if let AgentEventPayload::ReasoningDelta { text } = &event.payload {
                state.reasoning_tail.push_str(text);
                state.reasoning_tail =
                    tail_chars(&state.reasoning_tail, MAX_ACTIVITY_REASONING_TAIL_CHARS);
            }
            state.cursor = event.seq;
            state.updated_at = event.created_at;
        }
    }
    state.session_id = session_id;
    write_activity_state(connection, agent_thread_id, agent_turn_id, &state)?;
    Ok(())
}

fn write_activity_state(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
    state: &ActivityStateRow,
) -> anyhow::Result<()> {
    connection.execute(
        r#"
        INSERT INTO agent_activity_state (
            session_id, agent_thread_id, agent_turn_id, cursor,
            model_round, round_boundary, reasoning_tail, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(agent_thread_id, agent_turn_id) DO UPDATE SET
            session_id = excluded.session_id,
            cursor = excluded.cursor,
            model_round = excluded.model_round,
            round_boundary = excluded.round_boundary,
            reasoning_tail = excluded.reasoning_tail,
            updated_at = excluded.updated_at
        "#,
        params![
            state.session_id.to_string(),
            agent_thread_id.to_string(),
            agent_turn_id.to_string(),
            state.cursor,
            state.model_round.map(i64::try_from).transpose()?,
            state.round_boundary,
            &state.reasoning_tail,
            state.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn activity_model_round(payload: &AgentEventPayload) -> Option<usize> {
    match payload {
        AgentEventPayload::ModelContextBuilt { round, .. }
        | AgentEventPayload::ModelRequest { round, .. }
        | AgentEventPayload::ProviderRequestSent { round, .. }
        | AgentEventPayload::ProviderRequestRetried { round, .. }
        | AgentEventPayload::ProviderResponseReceived { round, .. } => Some(*round),
        _ => None,
    }
}

fn read_reasoning_tail(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
    agent_turn_id: AgentTurnId,
    after_cursor: i64,
    max_chars: usize,
) -> anyhow::Result<String> {
    let max_chars = max_chars.clamp(1, MAX_ACTIVITY_REASONING_TAIL_CHARS);
    let mut statement = connection.prepare(
        r#"
        SELECT payload_json
        FROM agent_events
        WHERE agent_thread_id = ?1
          AND agent_turn_id = ?2
          AND event_seq > ?3
          AND event_kind = 'reasoning_delta'
        ORDER BY event_seq DESC
        "#,
    )?;
    let rows = statement.query_map(
        params![
            agent_thread_id.to_string(),
            agent_turn_id.to_string(),
            after_cursor,
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut reverse_chunks = Vec::new();
    let mut chars = 0usize;
    for row in rows {
        let payload: AgentEventPayload = serde_json::from_str(&row?)?;
        let AgentEventPayload::ReasoningDelta { text } = payload else {
            continue;
        };
        chars = chars.saturating_add(text.chars().count());
        reverse_chunks.push(text);
        if chars >= max_chars {
            break;
        }
    }
    reverse_chunks.reverse();
    Ok(tail_chars(&reverse_chunks.concat(), max_chars))
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let mut tail = value.chars().rev().take(max_chars).collect::<Vec<_>>();
    tail.reverse();
    tail.into_iter().collect()
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
    delivered_at: Option<String>,
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
        delivered_at: row.get(9)?,
        acknowledged_at: row.get(10)?,
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
        delivered_at: raw
            .delivered_at
            .map(|value| parse_time(&value).map_err(mailbox_persistence))
            .transpose()?,
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
                   kind, payload_json, causation_id, created_at, delivered_at, acknowledged_at
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
                                   kind, payload_json, causation_id, created_at, delivered_at, acknowledged_at
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
                    delivered_at: None,
                    acknowledged_at: None,
                };
                transaction.execute(
                    r#"
                    INSERT INTO agent_mailbox_messages (
                        id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                        kind, payload_json, causation_id, created_at, delivery_state,
                        delivered_at, acknowledged_at
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
            .with_collaboration_write(|connection| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut statement = transaction.prepare(
                    r#"
                    SELECT id, session_id, sequence, from_agent_thread_id, to_agent_thread_id,
                           kind, payload_json, causation_id, created_at, delivered_at, acknowledged_at
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
                drop(statement);
                let delivered_at = Utc::now();
                for message in &mut messages {
                    transaction.execute(
                        r#"
                        UPDATE agent_mailbox_messages
                        SET delivery_state = CASE
                                WHEN acknowledged_at IS NULL THEN 'delivered'
                                ELSE 'acknowledged'
                            END,
                            delivered_at = COALESCE(delivered_at, ?2)
                        WHERE id = ?1
                        "#,
                        params![message.id.to_string(), delivered_at.to_rfc3339()],
                    )?;
                    message.delivered_at.get_or_insert(delivered_at);
                }
                transaction.commit()?;
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
                        "UPDATE agent_mailbox_messages SET delivery_state = 'acknowledged', delivered_at = COALESCE(delivered_at, ?2), acknowledged_at = COALESCE(acknowledged_at, ?2) WHERE id = ?1",
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
                root_turn_id: AgentTurnId::new(),
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
    async fn activity_event_batches_receive_contiguous_session_sequences() {
        let (_store, repository, session, root, root_turn) = fixture().await;
        let events = repository
            .append_activity_events(
                session.id,
                root.id,
                root_turn.id,
                root_turn.invocation_id,
                vec![
                    AgentEventPayload::ReasoningDelta {
                        text: "one".to_string(),
                    },
                    AgentEventPayload::ModelDelta {
                        text: "two".to_string(),
                    },
                ],
                None,
            )
            .expect("append activity batch");

        assert_eq!(
            events.iter().map(|event| event.seq).collect::<Vec<_>>(),
            [1, 2]
        );
        let listed = repository
            .list_activity_events(root.id, root_turn.id, None)
            .expect("list activity batch");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, events[0].id);
        assert_eq!(listed[1].id, events[1].id);
    }

    #[tokio::test]
    async fn activity_summary_materializes_and_reads_a_bounded_reasoning_tail() {
        let (store, repository, session, root, root_turn) = fixture().await;
        let events = repository
            .append_activity_events(
                session.id,
                root.id,
                root_turn.id,
                root_turn.invocation_id,
                vec![
                    AgentEventPayload::ReasoningDelta {
                        text: "one".to_string(),
                    },
                    AgentEventPayload::ReasoningDelta {
                        text: "two".to_string(),
                    },
                ],
                None,
            )
            .expect("append reasoning events");
        store
            .with_collaboration_write(|connection| {
                connection.execute(
                    "DELETE FROM agent_activity_state WHERE agent_turn_id = ?1",
                    params![root_turn.id.to_string()],
                )?;
                Ok(())
            })
            .expect("remove eager activity state");

        let summary = repository
            .activity_summary(root.id, root_turn.id, None, 4)
            .expect("read activity summary");
        assert_eq!(summary.cursor, events[1].seq);
        assert_eq!(summary.reasoning_tail.as_deref(), Some("etwo"));

        let incremental = repository
            .activity_summary(root.id, root_turn.id, Some(events[0].seq), 16)
            .expect("read incremental activity summary");
        assert_eq!(incremental.reasoning_tail.as_deref(), Some("two"));

        let state_rows = store
            .with_collaboration_read(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM agent_activity_state WHERE agent_turn_id = ?1",
                        params![root_turn.id.to_string()],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(Into::into)
            })
            .expect("count materialized state rows");
        assert_eq!(state_rows, 1);
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
    async fn recoverable_turn_query_distinguishes_queued_work_from_interrupted_runs() {
        let (_store, repository, _session, root, root_turn) = fixture().await;
        repository
            .transition_turn(root_turn.id, AgentTurnStatus::Running)
            .await
            .unwrap();
        let (_child, child_turn) = repository
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "recoverable".to_string(),
                agent_type: "worker".to_string(),
                task_message: "resume after restart".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    json!({ "agentType": "worker" }),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(2),
            })
            .await
            .unwrap();

        let recoverable = repository.list_recoverable_turns().unwrap();
        assert_eq!(
            recoverable
                .iter()
                .map(|turn| (turn.id, turn.status))
                .collect::<Vec<_>>(),
            [
                (root_turn.id, AgentTurnStatus::Running),
                (child_turn.id, AgentTurnStatus::Queued)
            ]
        );

        repository
            .record_turn_state(
                root_turn.id,
                AgentTurnStatus::Interrupted,
                &json!({ "reason": "simulated server restart" }),
            )
            .unwrap();
        let recoverable = repository.list_recoverable_turns().unwrap();
        assert_eq!(recoverable.len(), 1);
        assert_eq!(recoverable[0].id, child_turn.id);
        assert_eq!(recoverable[0].status, AgentTurnStatus::Queued);
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
                    root_turn_id: AgentTurnId::new(),
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
