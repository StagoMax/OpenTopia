//! SQLite row decoding and inserts for collaboration domain records.

use crate::collaboration::{
    AgentPath, AgentRuntimeSnapshotRecord, AgentThreadId, AgentThreadRecord, AgentTurnId,
    AgentTurnRecord, AgentTurnStatus, CollaborationSessionId, CollaborationSessionRecord,
    RuntimeSnapshotId,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

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

pub(super) struct RawThread {
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

pub(super) struct RawTurn {
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

pub(super) fn parse_uuid(value: &str) -> anyhow::Result<Uuid> {
    Ok(Uuid::parse_str(value)?)
}

pub(super) fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)?.with_timezone(&Utc))
}

pub(super) fn parse_optional_time(value: Option<String>) -> anyhow::Result<Option<DateTime<Utc>>> {
    value.map(|value| parse_time(&value)).transpose()
}

pub(super) fn read_session(
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

pub(super) fn read_snapshot(
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

pub(super) fn read_thread(
    connection: &Connection,
    agent_thread_id: AgentThreadId,
) -> anyhow::Result<Option<AgentThreadRecord>> {
    read_thread_where(connection, "id = ?1", agent_thread_id.to_string())
}

pub(super) fn read_thread_by_path(
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

pub(super) fn read_thread_where(
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

pub(super) fn raw_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawThread> {
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

pub(super) fn thread_from_raw(raw: RawThread) -> anyhow::Result<AgentThreadRecord> {
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

pub(super) fn read_turn(
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

pub(super) fn read_latest_turn(
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

pub(super) fn raw_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawTurn> {
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

pub(super) fn turn_from_raw(raw: RawTurn) -> anyhow::Result<AgentTurnRecord> {
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

pub(super) fn insert_session_bundle(
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

pub(super) fn insert_snapshot(
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

pub(super) fn insert_thread(
    connection: &Connection,
    thread: &AgentThreadRecord,
) -> anyhow::Result<()> {
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

pub(super) fn insert_turn(connection: &Connection, turn: &AgentTurnRecord) -> anyhow::Result<()> {
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
