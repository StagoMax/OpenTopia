//! Activity event persistence, bounded projections, and materialized activity state.

use super::{
    record_mapping::{parse_time, read_thread, read_turn},
    registry_persistence, SqliteCollaborationRepository,
};
use crate::collaboration::{
    AgentThreadId, AgentTurnId, CollaborationDomainError, CollaborationSessionId,
};
use crate::model::{AgentEvent, AgentEventPayload};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

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
