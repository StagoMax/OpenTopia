use super::sqlite_rows::{map_effect, map_goal};
use crate::effect_journal::EffectJournalRecord;
use crate::model::{AgentEventPayload, GoalRecord, GoalSnapshot, GoalStatus};
use crate::work_form::{WorkForm, WorkScope};
use anyhow::Context;
use rusqlite::{params, types::Type, Connection, OptionalExtension};
use uuid::Uuid;

pub(super) fn upsert_work_form_conn(conn: &Connection, form: &WorkForm) -> anyhow::Result<()> {
    anyhow::ensure!(
        form.id == form.scope.form_id(),
        "work form id does not match its stable scope identity"
    );
    conn.execute(
        r#"
        INSERT INTO work_forms (
            form_id, thread_id, scope_kind, scope_id, form_json, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(form_id) DO UPDATE SET
            thread_id = excluded.thread_id,
            scope_kind = excluded.scope_kind,
            scope_id = excluded.scope_id,
            form_json = excluded.form_json,
            updated_at = excluded.updated_at
        "#,
        params![
            form.id.to_string(),
            form.thread_id.to_string(),
            form.scope.kind(),
            form.scope.id().to_string(),
            serde_json::to_string(form)?,
            form.created_at.to_rfc3339(),
            form.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(super) fn query_work_form(
    conn: &Connection,
    form_id: Uuid,
) -> anyhow::Result<Option<WorkForm>> {
    conn.query_row(
        "SELECT form_json FROM work_forms WHERE form_id = ?1",
        params![form_id.to_string()],
        |row| {
            let json: String = row.get(0)?;
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn query_work_form_for_scope(
    conn: &Connection,
    scope: WorkScope,
) -> anyhow::Result<Option<WorkForm>> {
    query_work_form(conn, scope.form_id())
}

pub(super) fn valid_goal_transition(from: GoalStatus, to: GoalStatus) -> bool {
    if from == to {
        return true;
    }
    match from {
        GoalStatus::Active => matches!(
            to,
            GoalStatus::Paused
                | GoalStatus::Blocked
                | GoalStatus::Completed
                | GoalStatus::Cancelled
        ),
        GoalStatus::Paused | GoalStatus::Blocked => {
            matches!(to, GoalStatus::Active | GoalStatus::Cancelled)
        }
        GoalStatus::Completed | GoalStatus::Cancelled => false,
    }
}

pub(super) fn validate_goal_definition_list(
    field: &str,
    values: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        values.len() <= 20,
        "goal {field} may contain at most 20 entries"
    );
    values
        .into_iter()
        .map(|value| {
            let value = value.trim().to_string();
            anyhow::ensure!(!value.is_empty(), "goal {field} entries cannot be empty");
            anyhow::ensure!(
                value.chars().count() <= 300,
                "goal {field} entry exceeds 300 characters"
            );
            Ok(value)
        })
        .collect()
}

pub(super) fn query_goal(conn: &Connection, id: Uuid) -> anyhow::Result<Option<GoalRecord>> {
    conn.query_row(
        r#"
        SELECT id, thread_id, objective, token_budget, tokens_used,
               time_used_seconds, version, created_at, updated_at
        FROM goals
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_goal,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn load_goal_snapshot(
    conn: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<GoalSnapshot>> {
    let Some(mut goal) = query_goal(conn, id)? else {
        return Ok(None);
    };
    let work_form = query_work_form_for_scope(conn, WorkScope::Goal(id))?
        .with_context(|| format!("goal {id} is missing its WorkForm"))?;
    // The WorkForm is authoritative. Keep the legacy GoalRecord field only as
    // a derived API projection until the persisted column can be removed by a
    // dedicated data migration.
    goal.objective = work_form.objective.clone();
    Ok(Some(GoalSnapshot { goal, work_form }))
}

pub(super) fn conversation_payload_json(
    payload: &AgentEventPayload,
    full_payload_json: &str,
) -> anyhow::Result<Option<String>> {
    let compact = match payload {
        AgentEventPayload::ReasoningDelta { .. } => return Ok(None),
        AgentEventPayload::ModelContextBuilt {
            request_id,
            round,
            context_hash,
            stable_prefix_hash,
            dynamic_tail_hash,
            token_estimate,
            purpose,
            token_breakdown,
            ..
        } => serde_json::json!({
            "type": "model_context_built",
            "request_id": request_id,
            "round": round,
            "context_hash": context_hash,
            "stable_prefix_hash": stable_prefix_hash,
            "dynamic_tail_hash": dynamic_tail_hash,
            "token_estimate": token_estimate,
            "purpose": purpose,
            "token_breakdown": token_breakdown,
        }),
        AgentEventPayload::ModelRequest {
            request_id, round, ..
        } => serde_json::json!({
            "type": "model_request",
            "request_id": request_id,
            "round": round,
        }),
        AgentEventPayload::ProviderRequestSent {
            request_id,
            round,
            attempt,
            adapter,
            method,
            endpoint,
            cache_trace,
            ..
        } => serde_json::json!({
            "type": "provider_request_sent",
            "request_id": request_id,
            "round": round,
            "attempt": attempt,
            "adapter": adapter,
            "method": method,
            "endpoint": endpoint,
            "cache_trace": cache_trace,
        }),
        AgentEventPayload::ProviderRequestRetried {
            request_id,
            round,
            attempt,
            retry_kind,
            retry_index,
            retry_limit,
            reason,
            cache_trace,
            ..
        } => serde_json::json!({
            "type": "provider_request_retried",
            "request_id": request_id,
            "round": round,
            "attempt": attempt,
            "retry_kind": retry_kind,
            "retry_index": retry_index,
            "retry_limit": retry_limit,
            "reason": reason,
            "cache_trace": cache_trace,
        }),
        AgentEventPayload::ProviderResponseReceived {
            request_id,
            round,
            attempt,
            status,
            response_id,
            ..
        } => serde_json::json!({
            "type": "provider_response_received",
            "request_id": request_id,
            "round": round,
            "attempt": attempt,
            "status": status,
            "response_id": response_id,
        }),
        AgentEventPayload::ToolCallFinished { result } => serde_json::json!({
            "type": "tool_call_finished",
            "result": {
                "callId": result.call_id,
                "output": &result.output,
                "metadata": &result.metadata,
            },
        }),
        _ => return Ok(Some(full_payload_json.to_string())),
    };
    Ok(Some(serde_json::to_string(&compact)?))
}

pub(super) fn effect_select_sql() -> &'static str {
    r#"
        SELECT effect_id, thread_id, turn_id, agent_path, idempotency_key,
               kind, operation, input_hash, input_json, result_json, status,
               side_effect_class, idempotent, attempt, error, created_at,
               started_at, completed_at, updated_at
        FROM effect_journal
    "#
}

pub(super) fn query_effect(
    conn: &Connection,
    effect_id: Uuid,
) -> anyhow::Result<Option<EffectJournalRecord>> {
    conn.query_row(
        &format!("{} WHERE effect_id = ?1", effect_select_sql()),
        params![effect_id.to_string()],
        map_effect,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn query_effect_by_idempotency_key(
    conn: &Connection,
    thread_id: Uuid,
    turn_id: Uuid,
    agent_path: &str,
    idempotency_key: &str,
) -> anyhow::Result<Option<EffectJournalRecord>> {
    conn.query_row(
        &format!(
            "{} WHERE thread_id = ?1 AND turn_id = ?2 AND agent_path = ?3 AND idempotency_key = ?4",
            effect_select_sql()
        ),
        params![
            thread_id.to_string(),
            turn_id.to_string(),
            agent_path,
            idempotency_key,
        ],
        map_effect,
    )
    .optional()
    .map_err(Into::into)
}
