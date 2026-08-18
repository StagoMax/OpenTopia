use super::{
    checkpoint_token_budget, checkpoint_token_estimate, context_compaction_details, context_status,
    context_window_tokens, estimate_tokens, generate_context_summary, latest_context_summary_event,
    merge_context_checkpoint, render_context_checkpoint, sanitize_checkpoint_draft,
    trim_checkpoint_to_budget, validate_checkpoint_draft, ContextCheckpointDraft,
};
use crate::{
    current_settings, ensure_thread, get_workspace_diff_inner, publish_payload,
    redact_model_observation, AgentEvent, AgentEventPayload, ApiError, AppState, Approval,
    Artifact, ContextCheckpoint, ContextCheckpointCoverage, ContextCheckpointMode,
    ContextProjection, ContextSummary, Message, SessionStore, WorkspaceDiff,
};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads/:thread_id/context", get(get_context_status))
        .route(
            "/api/threads/:thread_id/context/compact",
            post(compact_context),
        )
        .route("/api/threads/:thread_id/trajectory", get(export_trajectory))
}

async fn get_context_status(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<ContextStatusResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let worker_state = state.clone();
    let status = tokio::task::spawn_blocking(move || context_status(&worker_state, thread_id))
        .await
        .map_err(|error| ApiError::internal(format!("context status task failed: {error}")))??;
    Ok(Json(status))
}

async fn compact_context(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<ContextCompactRequest>,
) -> Result<Json<ContextSummary>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let queued = state
        .store
        .list_queued_turn_messages(thread_id)?
        .into_iter()
        .collect::<HashSet<_>>();
    let messages = state
        .store
        .list_messages(thread_id)?
        .into_iter()
        .filter(|message| !queued.contains(&message.id))
        .collect::<Vec<_>>();
    let events = state.store.list_events(thread_id, None)?;
    let ContextCompactRequest {
        summary: supplied_summary,
        checkpoint: supplied_checkpoint,
    } = request;
    let supplied_summary = supplied_summary
        .map(|summary| summary.trim().to_string())
        .filter(|summary| !summary.is_empty());
    let covered_through_seq = events.last().map(|event| event.seq).unwrap_or_default();
    let previous_summary = latest_context_summary_event(&events);
    let coverage = ContextCheckpointCoverage {
        through_seq: covered_through_seq,
        through_message_count: messages.len(),
    };
    let summary = if let Some(draft) = supplied_checkpoint {
        let redacted = serde_json::to_value(draft)
            .map(|value| redact_model_observation(&value))
            .map_err(|error| ApiError::bad_request(format!("invalid checkpoint: {error}")))?;
        let mut draft: ContextCheckpointDraft = serde_json::from_value(redacted)
            .map_err(|error| ApiError::bad_request(format!("invalid checkpoint: {error}")))?;
        sanitize_checkpoint_draft(&mut draft, covered_through_seq)
            .map_err(|error| ApiError::bad_request(error.message))?;
        validate_checkpoint_draft(&draft, &events)
            .map_err(|error| ApiError::bad_request(error.message))?;
        let active_provider = current_settings(&state).active_provider().clone();
        let provider_compatibility_hash = state
            .store
            .get_provider_conversation_state(thread_id, "/root")?
            .filter(|provider_state| {
                provider_state.provider_id == active_provider.id
                    && provider_state.model == active_provider.model
            })
            .map(|provider_state| provider_state.compatibility_hash);
        let mut checkpoint = merge_context_checkpoint(
            previous_summary
                .as_ref()
                .and_then(|summary| summary.checkpoint.as_ref()),
            draft,
            thread_id,
            coverage,
            provider_compatibility_hash,
        );
        checkpoint.mode = ContextCheckpointMode::Manual;
        let checkpoint_budget =
            checkpoint_token_budget(active_provider.resolved_context_window_tokens());
        trim_checkpoint_to_budget(&mut checkpoint, checkpoint_budget);
        let rendered = render_context_checkpoint(&checkpoint);
        let checkpoint_tokens = estimate_tokens(&rendered);
        if checkpoint_tokens > checkpoint_budget {
            return Err(ApiError::bad_request(format!(
                "manual checkpoint exceeds its token budget ({checkpoint_tokens} > {checkpoint_budget})"
            )));
        }
        let mut summary =
            ContextSummary::new(thread_id, covered_through_seq, messages.len(), rendered);
        summary.token_estimate = Some(checkpoint_tokens);
        summary.metadata = json!({
            "mode": "manual",
            "source": "context_compact_api_structured",
            "checkpointId": checkpoint.id,
            "checkpointTokens": checkpoint_tokens,
            "checkpointBudgetTokens": checkpoint_budget,
            "inputTokens": checkpoint_tokens,
            "tokenReductionPercent": 0,
            "latencyMs": 0,
            "factRetentionPercent": 100,
            "activeConstraintRetentionPercent": 100,
            "coveredThroughSeq": covered_through_seq,
            "coveredMessageCount": messages.len(),
        });
        summary.checkpoint = Some(checkpoint);
        summary
    } else if let Some(summary_text) = supplied_summary {
        let previous_checkpoint_id = previous_summary
            .as_ref()
            .and_then(|summary| summary.checkpoint.as_ref())
            .map(|checkpoint| checkpoint.id);
        let mut summary = ContextSummary::new(
            thread_id,
            covered_through_seq,
            messages.len(),
            &summary_text,
        );
        let mut checkpoint = ContextCheckpoint::manual(thread_id, coverage, summary_text);
        checkpoint.previous_checkpoint_id = previous_checkpoint_id;
        let checkpoint_budget = checkpoint_token_budget(context_window_tokens(&state));
        trim_checkpoint_to_budget(&mut checkpoint, checkpoint_budget);
        let checkpoint_tokens = checkpoint_token_estimate(&checkpoint);
        if checkpoint_tokens > checkpoint_budget {
            return Err(ApiError::bad_request(format!(
                "manual summary exceeds its checkpoint token budget ({checkpoint_tokens} > {checkpoint_budget})"
            )));
        }
        summary.checkpoint = Some(checkpoint);
        summary.token_estimate = Some(estimate_tokens(&summary.summary));
        summary.metadata = json!({
            "mode": "manual",
            "source": "context_compact_api",
            "checkpointTokens": checkpoint_tokens,
            "checkpointBudgetTokens": checkpoint_budget,
            "inputTokens": estimate_tokens(&summary.summary),
            "tokenReductionPercent": 0,
            "latencyMs": 0,
            "factRetentionPercent": 100,
            "activeConstraintRetentionPercent": 100,
            "coveredThroughSeq": covered_through_seq,
            "coveredMessageCount": messages.len(),
        });
        summary
    } else {
        generate_context_summary(
            &state,
            thread_id,
            &messages,
            &events,
            "context_compact_api",
            None,
        )
        .await?
    };

    publish_payload(
        &state,
        thread_id,
        Some(Uuid::new_v4()),
        AgentEventPayload::ContextCompacted {
            summary: summary.clone(),
            details: Some(context_compaction_details(&state, thread_id, &summary)),
        },
    );
    Ok(Json(summary))
}

async fn export_trajectory(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<TrajectoryExport>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let messages = state.store.list_messages(thread_id)?;
    let events = state.store.list_events(thread_id, None)?;
    let approvals = state.store.list_approvals(thread_id, None)?;
    let artifact_metas = state.store.list_artifacts(thread_id)?;
    let mut artifacts = Vec::new();
    for meta in &artifact_metas {
        if let Ok(Some(artifact)) = state.store.get_artifact(thread_id, meta.id) {
            artifacts.push(artifact);
        }
    }
    let workspace_diff = get_workspace_diff_inner(&thread.workspace_root).await.ok();
    Ok(Json(TrajectoryExport {
        exported_at: Utc::now(),
        thread,
        messages,
        events,
        approvals,
        artifacts,
        workspace_diff,
    }))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextStatusResponse {
    pub(crate) budget: opentopia_core::ContextBudget,
    pub(crate) latest_summary: Option<ContextSummary>,
    pub(crate) usage: ContextUsageMetrics,
    pub(crate) projection: ContextProjection,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextUsageMetrics {
    pub(crate) model_requests: usize,
    pub(crate) agent_model_requests: usize,
    pub(crate) compaction_model_requests: usize,
    pub(crate) auxiliary_model_requests: usize,
    pub(crate) provider_responses: usize,
    pub(crate) provider_usage_coverage: Option<f64>,
    pub(crate) input_tokens: usize,
    pub(crate) output_tokens: usize,
    pub(crate) total_tokens: usize,
    pub(crate) uncached_input_tokens: usize,
    pub(crate) cached_input_tokens: usize,
    pub(crate) cache_write_tokens: usize,
    pub(crate) reasoning_tokens: usize,
    pub(crate) local_input_estimate: usize,
    pub(crate) raw_input_estimate: usize,
    pub(crate) estimate_calibration_factor: Option<f64>,
    pub(crate) estimate_error_mean: Option<f64>,
    pub(crate) estimate_error_p95: Option<f64>,
    pub(crate) raw_estimate_error_mean: Option<f64>,
    pub(crate) raw_estimate_error_p95: Option<f64>,
    pub(crate) compactions: usize,
    pub(crate) native_compactions: usize,
    pub(crate) provider_fallbacks: usize,
    pub(crate) warnings: usize,
    pub(crate) compaction_input_tokens: usize,
    pub(crate) checkpoint_tokens: usize,
    pub(crate) compaction_latency_ms: u64,
    pub(crate) last_fact_retention_percent: usize,
    pub(crate) last_active_constraint_retention_percent: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextCompactRequest {
    summary: Option<String>,
    checkpoint: Option<ContextCheckpointDraft>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrajectoryExport {
    exported_at: DateTime<Utc>,
    thread: opentopia_core::Thread,
    messages: Vec<Message>,
    events: Vec<AgentEvent>,
    approvals: Vec<Approval>,
    artifacts: Vec<Artifact>,
    workspace_diff: Option<WorkspaceDiff>,
}
