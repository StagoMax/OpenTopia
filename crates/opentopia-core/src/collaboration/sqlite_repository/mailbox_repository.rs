//! Durable mailbox row mapping and delivery lifecycle.

use super::{
    record_mapping::{parse_time, parse_uuid, read_thread},
    SqliteCollaborationRepository,
};
use crate::collaboration::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageId,
    AgentMailboxMessageKind, AgentThreadId, CollaborationSessionId, EnqueueAgentMessage,
};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub(super) struct RawMailboxMessage {
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

fn mailbox_persistence(error: impl Into<anyhow::Error>) -> AgentMailboxError {
    let error = error.into();
    match error.downcast::<AgentMailboxError>() {
        Ok(mailbox) => mailbox,
        Err(error) => AgentMailboxError::Persistence(error.to_string()),
    }
}

pub(super) fn raw_mailbox_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMailboxMessage> {
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

pub(super) fn mailbox_from_raw(
    raw: RawMailboxMessage,
) -> Result<AgentMailboxMessage, AgentMailboxError> {
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
