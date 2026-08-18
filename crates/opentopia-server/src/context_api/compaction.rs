use super::{
    checkpoint_retention_percentages, checkpoint_token_budget, context_checkpoint_schema,
    context_summary_system_prompt, merge_context_checkpoint, parse_checkpoint_response,
    render_context_checkpoint, sanitize_checkpoint_draft, trim_checkpoint_to_budget,
    validate_checkpoint_draft, ContextCheckpointDraft, ContextStatusResponse, ContextUsageMetrics,
};
use super::{estimate_tokens, historical_tool_artifact_reference, recent_conversation_tail};
use crate::{
    assemble_one_shot_model_request, configured_provider_from_settings, current_settings,
    publish_payload, redact_model_observation, AgentEvent, AgentEventPayload, ApiError, AppState,
    ContextCheckpointCoverage, ContextCheckpointMode, ContextCompactionDetails,
    ContextCompactionMetrics, ContextProjection, ContextSummary, Message, MessagePart,
    ModelCallPurpose, ModelGateway, ModelStreamDelta, ProviderConversationState,
    ProviderModelGateway, ProviderSettings, ProviderTransportEvent, ProviderTransportKind,
    SessionStore, CONTEXT_CHECKPOINT_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use uuid::Uuid;

pub(crate) fn context_window_tokens(state: &AppState) -> usize {
    current_settings(state)
        .active_provider()
        .resolved_context_window_tokens()
}

pub(crate) fn context_compact_threshold_percent() -> usize {
    std::env::var("OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|value: usize| value.clamp(50, 95))
        .unwrap_or(80)
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
                MessagePart::Text { text } => opentopia_core::estimate_model_context_tokens(text),
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

pub(crate) async fn generate_context_summary(
    state: &AppState,
    thread_id: Uuid,
    messages: &[Message],
    events: &[AgentEvent],
    source: &str,
    previous_summary_override: Option<&ContextSummary>,
) -> Result<ContextSummary, ApiError> {
    let settings = current_settings(state);
    let mut active = settings.active_provider().clone();
    if active.effective_transport() == ProviderTransportKind::Mock {
        return Err(ApiError::bad_request(
            "real context summarization requires an OpenAI-compatible provider",
        ));
    }
    // Compaction is an exceptional one-shot boundary; the resulting
    // checkpoint starts a new agent cache lineage instead of caching this
    // summarizer's changing input.
    active.prompt_cache_policy = None;
    let provider = configured_provider_from_settings(&active).ok_or_else(|| {
        ApiError::bad_request(format!(
            "provider '{}' has no configured API key",
            active.id
        ))
    })?;
    let previous_summary = previous_summary_override
        .cloned()
        .or_else(|| latest_context_summary_event(events));
    let snapshot = build_context_snapshot_with_limit(
        messages,
        events,
        previous_summary.as_ref(),
        context_snapshot_char_budget(active.resolved_context_window_tokens()),
    );
    let snapshot_input_tokens = estimate_tokens(&snapshot.prompt);
    let compaction_started = Instant::now();
    let request = assemble_one_shot_model_request(
        "opentopia:context_compaction",
        context_summary_system_prompt(),
        snapshot.prompt,
        Some(context_checkpoint_schema()),
    )
    .map_err(|error| ApiError::internal(format!("context request assembly failed: {error}")))?;
    let request_id = Uuid::new_v4();
    let input_breakdown = request.logical().token_estimate_breakdown();
    publish_payload(
        state,
        thread_id,
        None,
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
    publish_payload(
        state,
        thread_id,
        None,
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
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ProviderRequestSent {
            request_id,
            round: 0,
            attempt: 1,
            adapter: prepared.adapter.clone(),
            method: prepared.method.clone(),
            endpoint: prepared.endpoint.clone(),
            body: prepared.observation_body.clone(),
        },
    );
    let mut transport_events = Vec::new();
    let mut streamed_usage = None;
    let mut on_delta = |delta| {
        if let ModelStreamDelta::Usage { usage } = delta {
            streamed_usage = Some(usage);
        }
        Ok(())
    };
    let mut on_transport = |event| {
        transport_events.push(event);
        Ok(())
    };
    let response_result = timeout(
        Duration::from_secs(90),
        gateway.stream_prepared(prepared, &mut on_delta, &mut on_transport),
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
                body,
            } => publish_payload(
                state,
                thread_id,
                None,
                AgentEventPayload::ProviderRequestRetried {
                    request_id,
                    round: 0,
                    attempt,
                    retry_kind,
                    retry_index,
                    retry_limit,
                    reason,
                    body,
                },
            ),
            ProviderTransportEvent::Response {
                attempt,
                status,
                response_id,
                body,
            } => publish_payload(
                state,
                thread_id,
                None,
                AgentEventPayload::ProviderResponseReceived {
                    request_id,
                    round: 0,
                    attempt,
                    status,
                    response_id,
                    body,
                },
            ),
        }
    }
    let response = response_result
        .map_err(|_| ApiError::gateway_timeout("context summarization timed out"))?
        .map_err(|err| ApiError::bad_gateway(format!("context summarization failed: {err}")))?;
    let usage = response.usage.as_ref().or(streamed_usage.as_ref());
    if let Some(usage) = usage {
        publish_payload(
            state,
            thread_id,
            None,
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
    if response.text.trim().is_empty() {
        return Err(ApiError::bad_gateway(
            "context summarization provider returned empty text",
        ));
    }

    let checkpoint_value = parse_checkpoint_response(&response.text)?;
    let checkpoint_value = redact_model_observation(&checkpoint_value);
    let mut draft: ContextCheckpointDraft = serde_json::from_value(checkpoint_value)
        .map_err(|error| ApiError::bad_gateway(format!("invalid checkpoint payload: {error}")))?;
    sanitize_checkpoint_draft(&mut draft, snapshot.covered_through_seq)?;
    validate_checkpoint_draft(&draft, events)?;
    let provider_compatibility_hash = state
        .store
        .get_provider_conversation_state(thread_id, "/root")
        .ok()
        .flatten()
        .filter(|provider_state| {
            provider_state.provider_id == active.id && provider_state.model == active.model
        })
        .map(|provider_state| provider_state.compatibility_hash);
    let mut checkpoint = merge_context_checkpoint(
        previous_summary
            .as_ref()
            .and_then(|summary| summary.checkpoint.as_ref()),
        draft,
        thread_id,
        ContextCheckpointCoverage {
            through_seq: snapshot.covered_through_seq,
            through_message_count: snapshot.covered_message_count,
        },
        provider_compatibility_hash,
    );
    let checkpoint_budget = checkpoint_token_budget(active.resolved_context_window_tokens());
    trim_checkpoint_to_budget(&mut checkpoint, checkpoint_budget);
    let checkpoint_tokens =
        estimate_tokens(&serde_json::to_string(&checkpoint).map_err(|error| {
            ApiError::internal(format!("checkpoint serialization failed: {error}"))
        })?);
    if checkpoint_tokens > checkpoint_budget {
        return Err(ApiError::bad_gateway(format!(
            "checkpoint exceeds its token budget ({checkpoint_tokens} > {checkpoint_budget})"
        )));
    }
    let (fact_retention_percent, active_constraint_retention_percent) =
        checkpoint_retention_percentages(
            previous_summary
                .as_ref()
                .and_then(|summary| summary.checkpoint.as_ref()),
            &checkpoint,
        );
    let rendered_summary = render_context_checkpoint(&checkpoint);
    let latency_ms = compaction_started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let token_reduction_percent = snapshot_input_tokens
        .saturating_sub(checkpoint_tokens)
        .saturating_mul(100)
        / snapshot_input_tokens.max(1);

    let mut summary = ContextSummary::new(
        thread_id,
        snapshot.covered_through_seq,
        snapshot.covered_message_count,
        rendered_summary,
    );
    summary.token_estimate = Some(estimate_tokens(&summary.summary));
    summary.metadata = json!({
        "mode": "structured_local",
        "schemaVersion": CONTEXT_CHECKPOINT_SCHEMA_VERSION,
        "checkpointId": checkpoint.id,
        "checkpointTokens": checkpoint_tokens,
        "checkpointBudgetTokens": checkpoint_budget,
        "inputTokens": snapshot_input_tokens,
        "tokenReductionPercent": token_reduction_percent,
        "latencyMs": latency_ms,
        "factRetentionPercent": fact_retention_percent,
        "activeConstraintRetentionPercent": active_constraint_retention_percent,
        "source": source,
        "providerId": active.id,
        "model": active.model,
        "coveredThroughSeq": snapshot.covered_through_seq,
        "coveredMessageCount": snapshot.covered_message_count,
        "previousSummaryId": previous_summary.as_ref().map(|summary| summary.id),
    });
    summary.checkpoint = Some(checkpoint);
    Ok(summary)
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
            checkpoint_tokens: number("checkpointTokens")
                .max(summary.token_estimate.unwrap_or_default()),
            token_reduction_percent: number("tokenReductionPercent"),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextSnapshotInput {
    pub(crate) prompt: String,
    pub(crate) covered_message_count: usize,
    pub(crate) covered_through_seq: i64,
}

#[cfg(test)]
pub(crate) fn build_context_snapshot(
    messages: &[Message],
    events: &[AgentEvent],
    previous_summary: Option<&ContextSummary>,
) -> ContextSnapshotInput {
    build_context_snapshot_with_limit(messages, events, previous_summary, 96_000)
}

fn build_context_snapshot_with_limit(
    messages: &[Message],
    events: &[AgentEvent],
    previous_summary: Option<&ContextSummary>,
    max_snapshot_chars: usize,
) -> ContextSnapshotInput {
    let max_snapshot_chars = max_snapshot_chars.max(2_048);
    let mut sections = Vec::new();
    let mut used = 0usize;
    let message_cursor = previous_summary
        .map(summary_message_cursor)
        .unwrap_or_default()
        .min(messages.len());
    let event_cursor = previous_summary
        .map(|summary| summary.covered_through_seq)
        .unwrap_or_default();

    if let Some(previous) = previous_summary {
        let previous_state = previous
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| serde_json::to_string(checkpoint).ok())
            .unwrap_or_else(|| previous.summary.clone());
        let rendered = format!(
            "PREVIOUS DURABLE CHECKPOINT (merge with new evidence; preserve unresolved facts and statuses)\n{}",
            truncate_chars(&previous_state, max_snapshot_chars / 4)
        );
        used = rendered.chars().count();
        sections.push(rendered);
    }

    let mut covered_message_count = message_cursor;
    for message in messages.iter().skip(message_cursor) {
        let rendered = truncate_chars(&render_message_for_summary(message), max_snapshot_chars / 2);
        let chars = rendered.chars().count();
        let remaining = max_snapshot_chars.saturating_sub(used);
        if remaining == 0 || chars > remaining {
            break;
        }
        used = used.saturating_add(chars);
        sections.push(rendered);
        covered_message_count += 1;
    }

    let mut event_lines = Vec::new();
    let mut covered_through_seq = event_cursor;
    for event in events
        .iter()
        .filter(|event| event.seq > event_cursor)
        .take(160)
    {
        let rendered = match &event.payload {
            AgentEventPayload::ThreadContextSnapshot { .. }
            | AgentEventPayload::TurnContextSnapshot { .. }
            | AgentEventPayload::ModelContextBuilt { .. }
            | AgentEventPayload::ModelRequest { .. }
            | AgentEventPayload::ProviderRequestSent { .. }
            | AgentEventPayload::ProviderRequestRetried { .. }
            | AgentEventPayload::ProviderResponseReceived { .. }
            | AgentEventPayload::ModelDelta { .. }
            | AgentEventPayload::ReasoningDelta { .. }
            | AgentEventPayload::AssistantMessage { .. }
            | AgentEventPayload::TurnStarted { .. }
            | AgentEventPayload::ContextCompacted { .. }
            | AgentEventPayload::ContextProjectionBuilt { .. }
            | AgentEventPayload::ProviderContextStateUpdated { .. }
            | AgentEventPayload::ContextWarning { .. } => {
                covered_through_seq = event.seq;
                continue;
            }
            payload => serde_json::to_string(payload)
                .unwrap_or_else(|_| format!("{{\"type\":\"{}\"}}", payload.kind())),
        };
        let line = format!("seq={} {}", event.seq, truncate_chars(&rendered, 2_000));
        let line_chars = line.chars().count();
        if used.saturating_add(line_chars) > max_snapshot_chars {
            break;
        }
        used = used.saturating_add(line_chars);
        covered_through_seq = event.seq;
        event_lines.push(line);
    }

    ContextSnapshotInput {
        prompt: format!(
            "Update the durable summary from this contiguous session snapshot. New messages and events are ordered oldest to newest.\n\nSUMMARY AND NEW MESSAGES\n{}\n\nNEW IMPORTANT EVENTS\n{}",
            sections.join("\n\n"),
            event_lines.join("\n")
        ),
        covered_message_count,
        covered_through_seq,
    }
}

fn context_snapshot_char_budget(context_window: usize) -> usize {
    (context_window / 2).clamp(2_048, 384_000)
}

pub(crate) fn render_message_for_summary(message: &Message) -> String {
    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => truncate_chars(text, 12_000),
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
            MessagePart::SourceRef { source } => format!(
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
