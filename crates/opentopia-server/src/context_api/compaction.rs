use super::progress_deadline::{
    await_with_progress_deadlines, record_progress, ProgressDeadlineExceeded,
};
use super::{
    checkpoint_token_budget, context_summary_system_prompt, local_projection_retention_percentages,
    merge_context_checkpoint, project_checkpoint_draft, render_durable_context,
    sanitize_checkpoint_draft, semantic_summary_from_rendered_context, trim_checkpoint_to_budget,
    validate_checkpoint_draft, ContextStatusResponse, ContextUsageMetrics,
};
use super::{
    estimate_tokens, historical_tool_artifact_reference, project_model_conversation,
    recent_conversation_tail,
};
use crate::{
    configured_provider_from_settings, current_settings, publish_payload, redact_model_observation,
    AgentEvent, AgentEventPayload, ApiError, AppState, ContextCheckpointCoverage,
    ContextCheckpointMode, ContextCompactionDetails, ContextCompactionMetrics, ContextProjection,
    ContextSummary, Message, MessagePart, ModelCallPurpose, ModelGateway, ModelStreamDelta,
    ProviderConversationState, ProviderModelGateway, ProviderSettings, ProviderTransportEvent,
    ProviderTransportKind, SessionStore, CONTEXT_CHECKPOINT_SCHEMA_VERSION,
};
use opentopia_core::{
    content_fingerprint, AgentEventSender, ContextAssembler, ContextAssemblyInput,
    ContextCacheScope, ContextItemKind, ContextRole, ContextSensitivity, DefaultContextAssembler,
    ModelContextItem, ModelGatewayMetricEvent, ModelRequest,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering as AtomicOrdering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use uuid::Uuid;

const CONTEXT_SUMMARIZATION_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
const CONTEXT_SUMMARIZATION_ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(crate) fn context_window_tokens(state: &AppState) -> usize {
    current_settings(state)
        .active_provider()
        .resolved_context_window_tokens()
}

pub(crate) fn build_context_projection(
    summary: Option<&ContextSummary>,
    total_message_count: usize,
    events: &[AgentEvent],
    recent_tail_tokens: usize,
    provider: &ProviderSettings,
    provider_state: Option<&ProviderConversationState>,
) -> ContextProjection {
    let covered_through_seq = summary
        .map(|summary| summary.covered_through_seq)
        .unwrap_or_default();
    let unsummarized_event_count = events
        .iter()
        .filter(|event| event.seq > covered_through_seq)
        .count();
    build_context_projection_with_event_count(
        summary,
        total_message_count,
        unsummarized_event_count,
        recent_tail_tokens,
        provider,
        provider_state,
    )
}

fn build_context_projection_with_event_count(
    summary: Option<&ContextSummary>,
    total_message_count: usize,
    unsummarized_event_count: usize,
    recent_tail_tokens: usize,
    provider: &ProviderSettings,
    provider_state: Option<&ProviderConversationState>,
) -> ContextProjection {
    let covered_message_count = summary
        .map(summary_message_cursor)
        .unwrap_or_default()
        .min(total_message_count);
    let covered_through_seq = summary
        .map(|summary| summary.covered_through_seq)
        .unwrap_or_default();
    let capabilities = provider.capabilities();
    ContextProjection {
        checkpoint_id: summary
            .and_then(|summary| summary.checkpoint.as_ref())
            .map(|checkpoint| checkpoint.id),
        checkpoint_mode: summary.map(|summary| {
            summary
                .metadata
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("legacy_text")
                .to_string()
        }),
        checkpoint_tokens: summary
            .and_then(|summary| summary.token_estimate)
            .unwrap_or_default(),
        covered_through_seq,
        covered_message_count,
        unsummarized_message_count: total_message_count.saturating_sub(covered_message_count),
        unsummarized_event_count,
        recent_tail_tokens,
        native_compaction_supported: capabilities.supports_native_compaction,
        provider_state_available: provider_state.is_some(),
        provider_state_kind: provider_state
            .map(|provider_state| provider_state.state_kind.as_str().to_string()),
        provider_item_count: provider_state
            .map(|provider_state| provider_state.response_items.len())
            .unwrap_or_default(),
        native_compaction_item_count: provider_state
            .map(|provider_state| provider_state.compaction_item_count)
            .unwrap_or_default(),
    }
}

pub(crate) fn context_status(
    state: &AppState,
    thread_id: Uuid,
) -> Result<ContextStatusResponse, ApiError> {
    let messages = state.store.list_messages(thread_id)?;
    let mut budget = context_budget_from_messages(&messages, context_window_tokens(state));
    let events = state.store.list_context_events(thread_id)?;
    if let Some(model_tokens) = events.iter().rev().find_map(|event| match &event.payload {
        AgentEventPayload::ModelContextBuilt { token_estimate, .. } => Some(*token_estimate),
        _ => None,
    }) {
        budget.used_tokens = budget.used_tokens.max(model_tokens);
    }
    budget.estimated_usage = budget.used_tokens.saturating_mul(100) / budget.total_tokens.max(1);
    let latest_summary = latest_context_summary_event(&events);
    let (_, recent_tail_tokens) = recent_conversation_tail(
        &messages,
        (budget.total_tokens / 10).clamp(2_048, 16_384),
        &[],
    );
    let active_provider = current_settings(state).active_provider().clone();
    let provider_state = state
        .store
        .get_provider_conversation_state(thread_id, "/root")?
        .filter(|provider_state| {
            provider_state.provider_id == active_provider.id
                && provider_state.model == active_provider.model
        });
    let mut usage = ContextUsageMetrics::default();
    let mut estimate_errors = Vec::new();
    let mut raw_estimate_errors = Vec::new();
    let mut provider_response_request_ids = HashSet::new();
    let mut provider_usage_request_ids = HashSet::new();
    for event in &events {
        match &event.payload {
            AgentEventPayload::TokenUsage {
                input_tokens,
                output_tokens,
                total_tokens,
                cached_input_tokens,
                cache_write_tokens,
                reasoning_tokens,
                local_input_estimate,
                input_breakdown,
                purpose,
                request_id,
                ..
            } => {
                if let Some(request_id) = request_id {
                    provider_usage_request_ids.insert(*request_id);
                }
                usage.model_requests += 1;
                match purpose {
                    ModelCallPurpose::AgentRound => usage.agent_model_requests += 1,
                    ModelCallPurpose::ContextCompaction => usage.compaction_model_requests += 1,
                    _ => usage.auxiliary_model_requests += 1,
                }
                usage.input_tokens = usage.input_tokens.saturating_add(*input_tokens);
                usage.output_tokens = usage.output_tokens.saturating_add(*output_tokens);
                usage.total_tokens = usage.total_tokens.saturating_add(*total_tokens);
                usage.cached_input_tokens = usage
                    .cached_input_tokens
                    .saturating_add(cached_input_tokens.unwrap_or_default());
                usage.cache_write_tokens = usage
                    .cache_write_tokens
                    .saturating_add(cache_write_tokens.unwrap_or_default());
                usage.reasoning_tokens = usage
                    .reasoning_tokens
                    .saturating_add(reasoning_tokens.unwrap_or_default());
                usage.uncached_input_tokens = usage.uncached_input_tokens.saturating_add(
                    input_tokens.saturating_sub(cached_input_tokens.unwrap_or_default()),
                );
                if let Some(estimate) = local_input_estimate {
                    usage.local_input_estimate =
                        usage.local_input_estimate.saturating_add(*estimate);
                    if *input_tokens > 0 {
                        estimate_errors
                            .push(estimate.abs_diff(*input_tokens) as f64 / *input_tokens as f64);
                    }
                }
                if let Some(breakdown) = input_breakdown {
                    usage.raw_input_estimate =
                        usage.raw_input_estimate.saturating_add(breakdown.total);
                    if *input_tokens > 0 {
                        raw_estimate_errors.push(
                            breakdown.total.abs_diff(*input_tokens) as f64 / *input_tokens as f64,
                        );
                    }
                }
            }
            AgentEventPayload::ContextCompacted { details, .. } => {
                usage.compactions += 1;
                if let Some(metrics) = details
                    .as_ref()
                    .and_then(|details| details.metrics.as_ref())
                {
                    usage.compaction_input_tokens = usage
                        .compaction_input_tokens
                        .saturating_add(metrics.input_tokens);
                    usage.checkpoint_tokens = usage
                        .checkpoint_tokens
                        .saturating_add(metrics.checkpoint_tokens);
                    usage.compaction_latency_ms = usage
                        .compaction_latency_ms
                        .saturating_add(metrics.latency_ms);
                    usage.last_fact_retention_percent = metrics.fact_retention_percent;
                    usage.last_active_constraint_retention_percent =
                        metrics.active_constraint_retention_percent;
                }
            }
            AgentEventPayload::ProviderResponseReceived {
                request_id, body, ..
            } => {
                provider_response_request_ids.insert(*request_id);
                usage.native_compactions = usage.native_compactions.saturating_add(
                    body.get("providerItems")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter(|item| {
                                    item.get("type").and_then(Value::as_str) == Some("compaction")
                                })
                                .count()
                        })
                        .unwrap_or_default(),
                );
            }
            AgentEventPayload::ProviderContextStateInvalidated { .. } => {
                usage.provider_fallbacks += 1;
            }
            AgentEventPayload::ContextWarning { .. } => usage.warnings += 1,
            _ => {}
        }
    }
    usage.provider_responses = provider_response_request_ids.len();
    usage.provider_usage_coverage = (usage.provider_responses > 0).then(|| {
        provider_response_request_ids
            .intersection(&provider_usage_request_ids)
            .count() as f64
            / usage.provider_responses as f64
    });
    usage.estimate_calibration_factor = (usage.raw_input_estimate > 0)
        .then_some(usage.local_input_estimate as f64 / usage.raw_input_estimate as f64);
    if !estimate_errors.is_empty() {
        usage.estimate_error_mean =
            Some(estimate_errors.iter().sum::<f64>() / estimate_errors.len() as f64);
        estimate_errors.sort_by(f64::total_cmp);
        let p95_index = estimate_errors
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        usage.estimate_error_p95 = estimate_errors.get(p95_index).copied();
    }
    if !raw_estimate_errors.is_empty() {
        usage.raw_estimate_error_mean =
            Some(raw_estimate_errors.iter().sum::<f64>() / raw_estimate_errors.len() as f64);
        raw_estimate_errors.sort_by(f64::total_cmp);
        let p95_index = raw_estimate_errors
            .len()
            .saturating_mul(95)
            .div_ceil(100)
            .saturating_sub(1);
        usage.raw_estimate_error_p95 = raw_estimate_errors.get(p95_index).copied();
    }
    let covered_through_seq = latest_summary
        .as_ref()
        .map(|summary| summary.covered_through_seq)
        .unwrap_or_default();
    let unsummarized_event_count = state
        .store
        .count_events_after(thread_id, covered_through_seq)?;
    let projection = build_context_projection_with_event_count(
        latest_summary.as_ref(),
        messages.len(),
        unsummarized_event_count,
        recent_tail_tokens,
        &active_provider,
        provider_state.as_ref(),
    );
    Ok(ContextStatusResponse {
        budget,
        latest_summary,
        usage,
        projection,
    })
}

fn context_budget_from_messages(
    messages: &[Message],
    total_tokens: usize,
) -> opentopia_core::ContextBudget {
    let used_tokens = messages.iter().fold(0usize, |total, message| {
        let message_tokens = message
            .parts
            .iter()
            .map(|part| match part {
                MessagePart::Text { text } | MessagePart::ProposedPlan { text } => {
                    opentopia_core::estimate_model_context_tokens(text)
                }
                MessagePart::ToolResult { result } => {
                    opentopia_core::estimate_model_context_tokens(&result.output)
                }
                MessagePart::ToolCall { call } => {
                    opentopia_core::estimate_model_context_tokens(&call.name)
                        .saturating_add(opentopia_core::estimate_model_context_tokens(
                            &call.input.to_string(),
                        ))
                        .saturating_add(16)
                }
                _ => 16,
            })
            .sum::<usize>();
        total.saturating_add(message_tokens.saturating_add(50))
    });
    opentopia_core::ContextBudget {
        total_tokens,
        used_tokens,
        message_count: messages.len(),
        estimated_usage: used_tokens.saturating_mul(100) / total_tokens.max(1),
    }
}

pub(crate) fn latest_context_summary_event(events: &[AgentEvent]) -> Option<ContextSummary> {
    events.iter().rev().find_map(|event| {
        if let AgentEventPayload::ContextCompacted { summary, .. } = &event.payload {
            Some(summary.clone())
        } else {
            None
        }
    })
}

pub(crate) fn summary_message_cursor(summary: &ContextSummary) -> usize {
    summary
        .checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.coverage.through_message_count)
        .or_else(|| {
            summary
                .metadata
                .get("coveredMessageCount")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
        })
        .or_else(|| {
            (summary.metadata.get("mode").and_then(Value::as_str) == Some("manual"))
                .then_some(summary.message_count)
        })
        .unwrap_or_default()
        .min(summary.message_count)
}

pub(crate) fn latest_active_work_form_event(
    events: &[AgentEvent],
) -> Option<opentopia_core::WorkForm> {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.payload {
            AgentEventPayload::WorkFormUpdated { form } => Some(form.clone()),
            _ => None,
        })
        .filter(|form| form.status == opentopia_core::WorkFormStatus::Active)
}

pub(crate) fn durable_context(summary: Option<String>) -> Option<String> {
    summary.filter(|value| !value.trim().is_empty())
}

pub(crate) fn historical_context_model_request(
    messages: &[Message],
    previous_summary: Option<&ContextSummary>,
    provider_response_items: &[Value],
) -> ModelRequest {
    let (provider_transcript, previous_response_items) =
        opentopia_core::split_provider_transcript_state(provider_response_items.to_vec());
    let mut request = ModelRequest {
        instructions: Default::default(),
        input: Default::default(),
        tool_candidates: Vec::new(),
        previous_response_items,
        provider_transcript,
        previous_response_id: None,
        prompt_cache_breakpoint_policy: Default::default(),
        final_output_json_schema: None,
    };
    if let Some(summary) = previous_summary {
        request.instructions.items.push(
            ModelContextItem::text(
                ContextItemKind::Checkpoint,
                ContextRole::Developer,
                "opentopia:durable_checkpoint",
                format!(
                    "<durable_context>\n{}\n</durable_context>\nTreat this checkpoint as prior task state, not as a new user request.",
                    summary.summary
                ),
                ContextCacheScope::Thread,
                ContextSensitivity::Workspace,
            )
            .with_metadata(json!({
                "assemblyClass": "epoch",
                "selectedBy": "contextCheckpoint",
            })),
        );
    }
    let covered_messages = previous_summary
        .map(summary_message_cursor)
        .unwrap_or_default()
        .min(messages.len());
    request.input.conversation =
        project_model_conversation(&messages[covered_messages..], provider_response_items);
    request
}

fn assemble_current_context_compaction_request(
    current: &ModelRequest,
    events: &[AgentEvent],
    previous_summary: Option<&ContextSummary>,
) -> anyhow::Result<opentopia_core::CanonicalModelRequest> {
    let mut instructions = current.instructions.clone();
    instructions.items.push(ModelContextItem::text(
        ContextItemKind::BaseInstructions,
        ContextRole::System,
        "opentopia:context_compaction",
        context_summary_system_prompt(),
        ContextCacheScope::Stable,
        ContextSensitivity::Public,
    ));
    instructions.sort_items();

    let previous_seq = previous_summary
        .map(|summary| summary.covered_through_seq)
        .unwrap_or_default();
    let time_index = compact_event_time_index(events, previous_seq);
    let user_message = format!(
        "{}\n\n<opentopia_context_compaction_control>\nThe complete current model context above is the only semantic source to compress. Produce one replacement plain-text semantic summary for it. The following sampled durable time index is metadata for chronology, not a second history to catch up and not data that you need to serialize.\n{}\n</opentopia_context_compaction_control>",
        current.input.current_user.message,
        time_index,
    );
    let mut previous_response_items = current.previous_response_items.clone();
    if let Some(transcript) = current.provider_transcript.as_ref() {
        previous_response_items.push(opentopia_core::provider_transcript_state_item(transcript));
    }

    DefaultContextAssembler.compile(ContextAssemblyInput {
        model_context: &instructions,
        context_summary: None,
        conversation: current.input.conversation.clone(),
        user_message,
        user_content: current.input.current_user.content.clone(),
        // Compaction is observational. Tool schemas contributed to the source
        // request's pressure estimate, but the summarizer must not execute
        // tools while creating the checkpoint.
        tool_candidates: Vec::new(),
        previous_tool_calls: current.input.tool_calls.clone(),
        tool_results: current.input.tool_results.clone(),
        previous_response_items,
        // Always fork into a fresh provider call. The resulting checkpoint
        // starts another fresh provider epoch in Agent Core.
        previous_response_id: None,
        branch_developer_instructions: None,
        prompt_cache_breakpoint_policy: current.prompt_cache_breakpoint_policy,
        // The provider writes only semantic prose. The application projects
        // typed event facts and serializes the durable JSON envelope locally.
        final_output_json_schema: None,
    })
}

fn compact_event_time_index(events: &[AgentEvent], after_seq: i64) -> String {
    const MAX_INDEXED_EVENTS: usize = 256;
    let mut indexed = events
        .iter()
        .filter(|event| event.seq > after_seq)
        .filter(|event| {
            !matches!(
                event.payload,
                AgentEventPayload::ModelContextBuilt { .. }
                    | AgentEventPayload::ModelRequest { .. }
                    | AgentEventPayload::ProviderRequestSent { .. }
                    | AgentEventPayload::ProviderRequestRetried { .. }
                    | AgentEventPayload::ProviderResponseHeadersReceived { .. }
                    | AgentEventPayload::ProviderFirstTokenReceived { .. }
                    | AgentEventPayload::ProviderStreamProgress { .. }
                    | AgentEventPayload::ProviderResponseCommitStarted { .. }
                    | AgentEventPayload::ProviderResponseReceived { .. }
                    | AgentEventPayload::ModelDelta { .. }
                    | AgentEventPayload::ReasoningDelta { .. }
                    | AgentEventPayload::TokenUsage { .. }
            )
        })
        .collect::<Vec<_>>();
    if indexed.len() > MAX_INDEXED_EVENTS {
        let mut sampled = indexed.drain(..MAX_INDEXED_EVENTS / 2).collect::<Vec<_>>();
        sampled.extend(indexed.into_iter().rev().take(MAX_INDEXED_EVENTS / 2).rev());
        indexed = sampled;
    }
    if indexed.is_empty() {
        return "No new durable event timestamps were available.".to_string();
    }
    indexed
        .into_iter()
        .map(|event| {
            format!(
                "seq={} at={} kind={}",
                event.seq,
                event.created_at.to_rfc3339(),
                event.payload.kind(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_context_compaction_payload(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    event_sender: Option<&AgentEventSender>,
    payload: AgentEventPayload,
) {
    if let Err(payload) = send_context_compaction_payload(event_sender, payload) {
        // Manual compaction has no turn channel. A closed live channel can also
        // fall back here during shutdown so its last diagnostic is not lost.
        publish_payload(state, thread_id, turn_id, payload);
    }
}

fn send_context_compaction_payload(
    event_sender: Option<&AgentEventSender>,
    payload: AgentEventPayload,
) -> Result<(), AgentEventPayload> {
    let Some(sender) = event_sender else {
        return Err(payload);
    };
    sender.send(payload).map_err(|error| error.0)
}

fn context_summarization_timeout_error(timeout: ProgressDeadlineExceeded) -> ApiError {
    match timeout {
        ProgressDeadlineExceeded::Idle { timeout } => ApiError::gateway_timeout(format!(
            "context summarization timed out after {} seconds without stream progress",
            timeout.as_secs()
        )),
        ProgressDeadlineExceeded::Absolute { timeout } => ApiError::gateway_timeout(format!(
            "context summarization exceeded its {}-second absolute timeout",
            timeout.as_secs()
        )),
    }
}

pub(crate) async fn generate_context_summary(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    agent_path: &str,
    messages: &[Message],
    events: &[AgentEvent],
    source: &str,
    previous_summary_override: Option<&ContextSummary>,
    provider_override: Option<&ProviderSettings>,
    model_request: &ModelRequest,
    event_sender: Option<&AgentEventSender>,
) -> Result<ContextSummary, ApiError> {
    let settings = current_settings(state);
    let active = provider_override
        .cloned()
        .unwrap_or_else(|| settings.active_provider().clone());
    if active.effective_transport() == ProviderTransportKind::Mock {
        return Err(ApiError::bad_request(
            "real context summarization requires an OpenAI-compatible provider",
        ));
    }
    let provider = configured_provider_from_settings(&active).ok_or_else(|| {
        ApiError::bad_request(format!(
            "provider '{}' has no configured API key",
            active.id
        ))
    })?;
    let previous_summary = previous_summary_override
        .cloned()
        .or_else(|| latest_context_summary_event(events));
    let covered_message_count = messages.len();
    let covered_through_seq = events.last().map(|event| event.seq).unwrap_or_default();
    let compaction_started = Instant::now();
    let snapshot_input_tokens = model_request.token_estimate_breakdown().total;
    let request = assemble_current_context_compaction_request(
        model_request,
        events,
        previous_summary.as_ref(),
    )
    .map_err(|error| ApiError::internal(format!("context request assembly failed: {error}")))?;
    let request_id = Uuid::new_v4();
    let input_breakdown = request.logical().token_estimate_breakdown();
    emit_context_compaction_payload(
        state,
        thread_id,
        turn_id,
        event_sender,
        AgentEventPayload::ModelContextBuilt {
            request_id,
            round: 0,
            context_hash: request.manifest().context_hash.clone(),
            stable_prefix_hash: Some(request.manifest().stable_prefix_hash.clone()),
            dynamic_tail_hash: Some(request.manifest().dynamic_tail_hash.clone()),
            token_estimate: input_breakdown.total,
            purpose: ModelCallPurpose::ContextCompaction,
            token_breakdown: Some(input_breakdown.clone()),
            items: request.materialized_context().items.clone(),
        },
    );
    let request_snapshot = serde_json::to_value(request.logical())
        .map(|value| redact_model_observation(&value))
        .unwrap_or_else(|error| json!({ "serializationError": error.to_string() }));
    emit_context_compaction_payload(
        state,
        thread_id,
        turn_id,
        event_sender,
        AgentEventPayload::ModelRequest {
            request_id,
            round: 0,
            request: request_snapshot,
        },
    );
    let gateway = ProviderModelGateway::from_provider(provider);
    let prepared = gateway.prepare(request_id, request).map_err(|err| {
        ApiError::bad_gateway(format!("context request preparation failed: {err}"))
    })?;
    emit_context_compaction_payload(
        state,
        thread_id,
        turn_id,
        event_sender,
        AgentEventPayload::ProviderRequestSent {
            request_id,
            round: 0,
            attempt: 1,
            adapter: prepared.adapter.clone(),
            method: prepared.method.clone(),
            endpoint: prepared.endpoint.clone(),
            cache_trace: prepared.cache_trace.clone(),
            body: prepared.observation_body.clone(),
            checkpoint: None,
        },
    );
    let mut transport_events = Vec::new();
    let mut streamed_usage = None;
    let first_token_observed = Arc::new(AtomicBool::new(false));
    let (progress_sender, progress_receiver) = watch::channel(0_u64);
    let metric_first_token_observed = Arc::clone(&first_token_observed);
    let metric_progress_sender = progress_sender.clone();
    let mut on_metric = |metric| {
        match metric {
            ModelGatewayMetricEvent::FirstOutputTokenReceived {
                request_id: metric_request_id,
            } => {
                debug_assert_eq!(metric_request_id, request_id);
                if !metric_first_token_observed.swap(true, AtomicOrdering::SeqCst) {
                    record_progress(&metric_progress_sender);
                    emit_context_compaction_payload(
                        state,
                        thread_id,
                        turn_id,
                        event_sender,
                        AgentEventPayload::ProviderFirstTokenReceived { request_id },
                    );
                }
            }
        }
        Ok(())
    };
    let delta_progress_sender = progress_sender.clone();
    let mut on_delta = |delta: ModelStreamDelta| {
        record_progress(&delta_progress_sender);
        if let ModelStreamDelta::Usage { usage } = delta {
            streamed_usage = Some(usage);
        }
        Ok(())
    };
    let transport_first_token_observed = Arc::clone(&first_token_observed);
    let transport_progress_sender = progress_sender.clone();
    let mut on_transport = |event| {
        record_progress(&transport_progress_sender);
        let payload = match event {
            ProviderTransportEvent::ResponseHeaders { attempt, status } => {
                Some(AgentEventPayload::ProviderResponseHeadersReceived {
                    request_id,
                    round: 0,
                    attempt,
                    status,
                })
            }
            ProviderTransportEvent::OutputStarted { .. } => (!transport_first_token_observed
                .swap(true, AtomicOrdering::SeqCst))
            .then_some(AgentEventPayload::ProviderFirstTokenReceived { request_id }),
            ProviderTransportEvent::StreamProgress {
                attempt,
                output_events,
                output_bytes,
                elapsed_ms,
            } => Some(AgentEventPayload::ProviderStreamProgress {
                request_id,
                round: 0,
                attempt,
                output_events,
                output_bytes,
                elapsed_ms,
            }),
            ProviderTransportEvent::ResponseCommitStarted {
                attempt,
                output_events,
                output_bytes,
                elapsed_ms,
            } => Some(AgentEventPayload::ProviderResponseCommitStarted {
                request_id,
                round: 0,
                attempt,
                output_events,
                output_bytes,
                elapsed_ms,
            }),
            terminal_or_retry => {
                transport_events.push(terminal_or_retry);
                None
            }
        };
        if let Some(payload) = payload {
            emit_context_compaction_payload(state, thread_id, turn_id, event_sender, payload);
        }
        Ok(())
    };
    // Long-history structured checkpoints can legitimately run for minutes.
    // Bound silence independently from total runtime so a healthy streaming
    // response keeps its lease while an endlessly active response still has a
    // finite safety ceiling.
    let response_result = await_with_progress_deadlines(
        gateway.stream_prepared(prepared, &mut on_delta, &mut on_transport, &mut on_metric),
        progress_receiver,
        CONTEXT_SUMMARIZATION_IDLE_TIMEOUT,
        CONTEXT_SUMMARIZATION_ABSOLUTE_TIMEOUT,
    )
    .await;
    drop(on_delta);
    drop(on_transport);
    for observation in transport_events {
        match observation {
            ProviderTransportEvent::Retry {
                attempt,
                retry_kind,
                retry_index,
                retry_limit,
                reason,
                cache_trace,
                body,
            } => emit_context_compaction_payload(
                state,
                thread_id,
                turn_id,
                event_sender,
                AgentEventPayload::ProviderRequestRetried {
                    request_id,
                    round: 0,
                    attempt,
                    retry_kind,
                    retry_index,
                    retry_limit,
                    reason,
                    cache_trace,
                    body,
                },
            ),
            ProviderTransportEvent::Response {
                attempt,
                status,
                response_id,
                body,
            } => emit_context_compaction_payload(
                state,
                thread_id,
                turn_id,
                event_sender,
                AgentEventPayload::ProviderResponseReceived {
                    request_id,
                    round: 0,
                    attempt,
                    status,
                    response_id,
                    body,
                },
            ),
            ProviderTransportEvent::ResponseHeaders { .. }
            | ProviderTransportEvent::OutputStarted { .. }
            | ProviderTransportEvent::StreamProgress { .. }
            | ProviderTransportEvent::ResponseCommitStarted { .. } => {}
        }
    }
    let (response, mut semantic_summary_status, degradation_reason) = match response_result {
        Ok(Ok(response)) if !response.text.trim().is_empty() => (Some(response), "generated", None),
        Ok(Ok(_)) => (
            None,
            "empty_response",
            Some("context summarization provider returned empty text".to_string()),
        ),
        Ok(Err(error)) => (
            None,
            "provider_error",
            Some(format!("context summarization failed: {error}")),
        ),
        Err(timeout) => {
            let error = context_summarization_timeout_error(timeout);
            (None, "timeout", Some(error.message))
        }
    };
    if let Some(reason) = degradation_reason.as_deref() {
        tracing::warn!(
            %thread_id,
            ?turn_id,
            %request_id,
            provider_id = %active.id,
            model = %active.model,
            source,
            reason,
            "context semantic summary degraded; continuing with the local event projection"
        );
        emit_context_compaction_payload(
            state,
            thread_id,
            turn_id,
            event_sender,
            AgentEventPayload::ContextWarning {
                stage: "context_semantic_summary_degraded".to_string(),
                message: format!(
                    "Semantic summary unavailable ({reason}); generated the checkpoint from the durable local event projection."
                ),
            },
        );
    }
    let usage = response
        .as_ref()
        .and_then(|response| response.usage.as_ref())
        .or(streamed_usage.as_ref())
        .cloned();
    if let Some(usage) = usage.as_ref() {
        emit_context_compaction_payload(
            state,
            thread_id,
            turn_id,
            event_sender,
            AgentEventPayload::TokenUsage {
                request_id: Some(request_id),
                round: Some(0),
                purpose: ModelCallPurpose::ContextCompaction,
                input_tokens: usage.input_tokens as usize,
                output_tokens: usage.output_tokens as usize,
                total_tokens: usage.total_tokens as usize,
                cached_input_tokens: usage.cached_input_tokens.map(|value| value as usize),
                cache_write_tokens: usage.cache_write_tokens.map(|value| value as usize),
                reasoning_tokens: usage.reasoning_tokens.map(|value| value as usize),
                local_input_estimate: Some(input_breakdown.total),
                input_breakdown: Some(input_breakdown.clone()),
            },
        );
    }
    let provider_semantic_summary = response.as_ref().map(|response| {
        let redacted = redact_model_observation(&Value::String(response.text.trim().to_string()));
        redacted.as_str().unwrap_or_default().trim().to_string()
    });
    if let Some(summary) = provider_semantic_summary.as_deref() {
        tracing::info!(
            %thread_id,
            ?turn_id,
            %request_id,
            provider_id = %active.id,
            model = %active.model,
            source,
            response_chars = summary.chars().count(),
            response_bytes = summary.len(),
            response_fingerprint = %content_fingerprint(summary.as_bytes()),
            "context semantic summary retained in the durable checkpoint"
        );
    }
    let previous_checkpoint = previous_summary
        .as_ref()
        .and_then(|summary| summary.checkpoint.as_ref());
    let fallback_semantic_summary = previous_summary
        .as_ref()
        .and_then(|summary| semantic_summary_from_rendered_context(&summary.summary))
        .or_else(|| previous_checkpoint.map(|checkpoint| checkpoint.goal.clone()));
    let semantic_summary_source = if provider_semantic_summary.is_some() {
        "provider"
    } else if fallback_semantic_summary.is_some() {
        "previous_checkpoint"
    } else {
        "none"
    };
    let semantic_summary = provider_semantic_summary
        .filter(|summary| !summary.is_empty())
        .or(fallback_semantic_summary)
        .unwrap_or_default();
    let mut draft =
        project_checkpoint_draft(messages, events, &semantic_summary, previous_checkpoint);
    sanitize_checkpoint_draft(&mut draft, covered_through_seq, events)?;
    validate_checkpoint_draft(&draft, events)?;
    let provider_compatibility_hash = state
        .store
        .get_provider_conversation_state(thread_id, agent_path)
        .ok()
        .flatten()
        .filter(|provider_state| {
            provider_state.provider_id == active.id && provider_state.model == active.model
        })
        .map(|provider_state| provider_state.compatibility_hash);
    let mut checkpoint = merge_context_checkpoint(
        None,
        draft,
        thread_id,
        ContextCheckpointCoverage {
            through_seq: covered_through_seq,
            through_message_count: covered_message_count,
        },
        provider_compatibility_hash.or_else(|| {
            previous_checkpoint
                .and_then(|checkpoint| checkpoint.provider_compatibility_hash.clone())
        }),
    );
    checkpoint.previous_checkpoint_id = previous_checkpoint.map(|checkpoint| checkpoint.id);
    let checkpoint_budget = checkpoint_token_budget(active.resolved_context_window_tokens());
    // Reserve room for the application-owned envelope before adding semantic
    // prose. Deterministic fields retain priority when the two compete.
    trim_checkpoint_to_budget(&mut checkpoint, checkpoint_budget.saturating_sub(128));
    let checkpoint_tokens =
        estimate_tokens(&serde_json::to_string(&checkpoint).map_err(|error| {
            ApiError::internal(format!("checkpoint serialization failed: {error}"))
        })?);
    if checkpoint_tokens > checkpoint_budget {
        return Err(ApiError::bad_gateway(format!(
            "checkpoint exceeds its token budget ({checkpoint_tokens} > {checkpoint_budget})"
        )));
    }
    let empty_rendered = render_durable_context(&checkpoint, "")?;
    let semantic_budget = checkpoint_budget
        .saturating_sub(estimate_tokens(&empty_rendered))
        .min(checkpoint_budget / 4);
    let semantic_summary_was_available = !semantic_summary.is_empty();
    let mut semantic_summary = truncate_text_to_token_budget(&semantic_summary, semantic_budget);
    if semantic_summary_was_available && semantic_summary.is_empty() {
        semantic_summary_status = "dropped_for_budget";
    }
    let mut rendered_summary = render_durable_context(&checkpoint, &semantic_summary)?;
    if estimate_tokens(&rendered_summary) > checkpoint_budget {
        semantic_summary.clear();
        semantic_summary_status = "dropped_for_budget";
        rendered_summary = render_durable_context(&checkpoint, "")?;
    }
    if estimate_tokens(&rendered_summary) > checkpoint_budget {
        return Err(ApiError::bad_gateway(format!(
            "locally projected checkpoint exceeds its token budget ({} > {checkpoint_budget})",
            estimate_tokens(&rendered_summary)
        )));
    }
    // This is the representation that the next logical request actually
    // materializes. Keep the raw checkpoint JSON size as a budget metric, but
    // use the rendered checkpoint for before/after request reduction.
    let rendered_checkpoint_tokens = estimate_tokens(&rendered_summary);
    let (fact_retention_percent, active_constraint_retention_percent) =
        local_projection_retention_percentages(previous_checkpoint, &checkpoint);
    let latency_ms = compaction_started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let token_reduction_percent = snapshot_input_tokens
        .saturating_sub(rendered_checkpoint_tokens)
        .saturating_mul(100)
        / snapshot_input_tokens.max(1);
    let post_compaction_tokens = rendered_checkpoint_tokens;
    let tokens_removed = snapshot_input_tokens.saturating_sub(post_compaction_tokens);
    let remaining_percent =
        post_compaction_tokens.saturating_mul(100) / snapshot_input_tokens.max(1);
    let provider_input_tokens = usage
        .as_ref()
        .map(|usage| usage.input_tokens as usize)
        .unwrap_or_default();
    let provider_output_tokens = usage
        .as_ref()
        .map(|usage| usage.output_tokens as usize)
        .unwrap_or_default();
    let cached_input_tokens = usage
        .as_ref()
        .and_then(|usage| usage.cached_input_tokens)
        .unwrap_or_default() as usize;
    let cache_hit_percent = cached_input_tokens.saturating_mul(100) / provider_input_tokens.max(1);

    let mut summary = ContextSummary::new(
        thread_id,
        covered_through_seq,
        covered_message_count,
        rendered_summary,
    );
    summary.token_estimate = Some(rendered_checkpoint_tokens);
    summary.metadata = json!({
        "mode": "structured_local",
        "schemaVersion": CONTEXT_CHECKPOINT_SCHEMA_VERSION,
        "checkpointId": checkpoint.id,
        "checkpointTokens": checkpoint_tokens,
        "checkpointBudgetTokens": checkpoint_budget,
        "inputTokens": snapshot_input_tokens,
        "postCompactionTokens": post_compaction_tokens,
        "tokensRemoved": tokens_removed,
        "remainingPercent": remaining_percent,
        "tokenReductionPercent": token_reduction_percent,
        "providerInputTokens": provider_input_tokens,
        "providerOutputTokens": provider_output_tokens,
        "cachedInputTokens": cached_input_tokens,
        "cacheHitPercent": cache_hit_percent,
        "latencyMs": latency_ms,
        "factRetentionPercent": fact_retention_percent,
        "activeConstraintRetentionPercent": active_constraint_retention_percent,
        "source": source,
        "providerId": active.id,
        "model": active.model,
        "coveredThroughSeq": covered_through_seq,
        "coveredMessageCount": covered_message_count,
        "previousSummaryId": previous_summary.as_ref().map(|summary| summary.id),
        "semanticSummaryStatus": semantic_summary_status,
        "semanticSummarySource": semantic_summary_source,
        "semanticSummaryChars": semantic_summary.chars().count(),
        "semanticSummaryFingerprint": (!semantic_summary.is_empty()).then(|| content_fingerprint(semantic_summary.as_bytes())),
        "semanticSummaryDegradationReason": degradation_reason,
    });
    summary.checkpoint = Some(checkpoint);
    Ok(summary)
}

fn truncate_text_to_token_budget(value: &str, token_budget: usize) -> String {
    let value = value.trim();
    if value.is_empty() || token_budget == 0 {
        return String::new();
    }
    if estimate_tokens(value) <= token_budget {
        return value.to_string();
    }

    let chars = value.chars().collect::<Vec<_>>();
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let candidate = chars[..middle].iter().collect::<String>();
        if estimate_tokens(&candidate) <= token_budget {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    chars[..low].iter().collect::<String>().trim().to_string()
}

pub(crate) fn context_compaction_details(
    state: &AppState,
    thread_id: Uuid,
    summary: &ContextSummary,
) -> ContextCompactionDetails {
    let checkpoint = summary.checkpoint.as_ref();
    let number = |key: &str| {
        summary
            .metadata
            .get(key)
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize
    };
    let metrics = summary
        .metadata
        .get("source")
        .and_then(Value::as_str)
        .map(|source| ContextCompactionMetrics {
            source: source.to_string(),
            input_tokens: number("inputTokens"),
            post_compaction_tokens: number("postCompactionTokens"),
            checkpoint_tokens: number("checkpointTokens")
                .max(summary.token_estimate.unwrap_or_default()),
            tokens_removed: number("tokensRemoved"),
            remaining_percent: number("remainingPercent"),
            token_reduction_percent: number("tokenReductionPercent"),
            provider_input_tokens: number("providerInputTokens"),
            provider_output_tokens: number("providerOutputTokens"),
            cached_input_tokens: number("cachedInputTokens"),
            cache_hit_percent: number("cacheHitPercent"),
            latency_ms: summary
                .metadata
                .get("latencyMs")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            fact_retention_percent: number("factRetentionPercent"),
            active_constraint_retention_percent: number("activeConstraintRetentionPercent"),
        });
    ContextCompactionDetails {
        checkpoint_id: checkpoint.map(|checkpoint| checkpoint.id),
        mode: checkpoint
            .map(|checkpoint| checkpoint.mode)
            .unwrap_or(ContextCheckpointMode::LegacyText),
        coverage: checkpoint
            .map(|checkpoint| checkpoint.coverage.clone())
            .unwrap_or(ContextCheckpointCoverage {
                through_seq: summary.covered_through_seq,
                through_message_count: summary_message_cursor(summary),
            }),
        provider_state_checkpoint_id: state
            .store
            .get_provider_conversation_state(thread_id, "/root")
            .ok()
            .flatten()
            .and_then(|provider_state| provider_state.checkpoint_id),
        metrics,
    }
}

pub(crate) fn render_message_for_summary(message: &Message) -> String {
    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => truncate_chars(text, 12_000),
            MessagePart::ProposedPlan { text } => {
                format!("proposed_plan {}", truncate_chars(text, 12_000))
            }
            MessagePart::Image {
                content_type, data, ..
            } => format!("image {} ({} bytes)", content_type, data.len()),
            MessagePart::ImageRef { image_id } => format!("image_ref {image_id}"),
            MessagePart::ToolCall { call } => format!(
                "tool_call {} {}",
                call.name,
                truncate_chars(&call.input.to_string(), 4_000)
            ),
            MessagePart::ToolResult { result } => format!(
                "tool_result {}{} {}",
                result.call_id,
                historical_tool_artifact_reference(&result.metadata)
                    .map(|reference| format!(" artifact={reference}"))
                    .unwrap_or_default(),
                truncate_chars(&result.output, 4_000)
            ),
            MessagePart::FileRef { path } => format!("file_ref {}", path.display()),
            MessagePart::SourceRef { source, .. } => format!(
                "source_ref {} {} {} bytes{}",
                source.name,
                source.path.display(),
                source.bytes,
                if source.truncated { " truncated" } else { "" }
            ),
            MessagePart::SkillRef { skill } => format!(
                "skill_ref {} {}{}",
                skill.name,
                skill.path.display(),
                if skill.truncated { " truncated" } else { "" }
            ),
            MessagePart::TurnContext {
                collaboration_mode,
                goal_id,
                library_provider,
            } => format!(
                "turn_context mode={} goal={} library={}",
                collaboration_mode.as_str(),
                goal_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                library_provider.as_deref().unwrap_or("none")
            ),
            MessagePart::Error { message } => format!("error {}", truncate_chars(message, 4_000)),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[{} {}]\n{}",
        message.role.as_str(),
        message.created_at.to_rfc3339(),
        parts
    )
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut output = value.chars().take(max_chars).collect::<String>();
        output.push_str("\n[truncated]");
        output
    }
}

pub(crate) fn truncate_with_flag(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = value[..end].to_string();
    truncated.push_str("\n\n[output truncated]");
    (truncated, true)
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_current_context_compaction_request, historical_context_model_request,
        send_context_compaction_payload, truncate_text_to_token_budget,
    };
    use crate::{
        AgentEventPayload, ContextCheckpoint, ContextCheckpointCoverage, ContextSummary, Message,
        MessageRole,
    };
    use uuid::Uuid;

    #[test]
    fn repeated_compaction_request_contains_checkpoint_and_all_later_history_once() {
        let thread_id = Uuid::new_v4();
        let messages = vec![
            Message::text(thread_id, MessageRole::User, "old user"),
            Message::text(thread_id, MessageRole::Assistant, "old assistant"),
            Message::text(thread_id, MessageRole::User, "new user"),
        ];
        let mut previous = ContextSummary::new(thread_id, 10, 2, "checkpoint one");
        previous.checkpoint = Some(ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage {
                through_seq: 10,
                through_message_count: 2,
            },
            "checkpoint one",
        ));

        let current = historical_context_model_request(&messages, Some(&previous), &[]);
        assert_eq!(current.input.conversation.len(), 1);
        assert_eq!(current.input.conversation[0].content, "new user");
        assert!(current
            .instructions
            .items
            .iter()
            .any(|item| item.text_content().contains("checkpoint one")));

        let compaction =
            assemble_current_context_compaction_request(&current, &[], Some(&previous))
                .expect("compaction request");
        assert!(compaction.logical().previous_response_id.is_none());
        assert!(compaction.logical().final_output_json_schema.is_none());
        assert_eq!(compaction.logical().input.conversation.len(), 1);
        let compaction_instruction = compaction
            .logical()
            .instructions
            .items
            .iter()
            .find(|item| item.source == "opentopia:context_compaction")
            .expect("semantic compaction instruction");
        assert!(compaction_instruction.text_content().contains("never JSON"));
    }

    #[test]
    fn live_round_compaction_uses_the_turn_event_channel() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let payload = AgentEventPayload::ContextWarning {
            stage: "round_context_compaction".to_string(),
            message: "checkpoint started".to_string(),
        };

        send_context_compaction_payload(Some(&sender), payload)
            .expect("live compaction event is queued on the turn channel");

        assert!(matches!(
            receiver.try_recv().expect("queued compaction event"),
            AgentEventPayload::ContextWarning { stage, message }
                if stage == "round_context_compaction" && message == "checkpoint started"
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn semantic_text_is_bounded_without_requiring_a_parseable_format() {
        let input = "普通 Markdown ```json { 任意文本 } ".repeat(1_000);

        let bounded = truncate_text_to_token_budget(&input, 128);

        assert!(!bounded.is_empty());
        assert!(super::estimate_tokens(&bounded) <= 128);
        assert!(bounded.contains("```json"));
    }
}
