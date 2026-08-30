//! HTTP context and compaction domain.

mod checkpoint;
mod compaction;
mod conversation;
mod http;
mod progress_deadline;
mod round_compaction;
mod turn_context;

use crate::AppState;
use axum::Router;

pub(super) use checkpoint::{
    checkpoint_retention_percentages, checkpoint_token_budget, checkpoint_token_estimate,
    context_checkpoint_schema, context_summary_system_prompt, merge_context_checkpoint,
    parse_checkpoint_response, render_context_checkpoint, sanitize_checkpoint_draft,
    trim_checkpoint_to_budget, validate_checkpoint_draft, ContextCheckpointDraft,
};
pub(super) use compaction::{
    build_context_projection, context_compaction_details, context_status, context_window_tokens,
    durable_context, generate_context_summary, historical_context_model_request,
    latest_active_work_form_event, latest_context_summary_event, render_message_for_summary,
    summary_message_cursor, truncate_chars, truncate_with_flag,
};
pub(super) use conversation::{
    estimate_tokens, historical_tool_artifact_reference, model_content_part_token_estimate,
    model_conversation_message_token_estimate, model_user_message_with_attachment_manifest,
    prior_messages_for_turn, project_model_conversation, recent_conversation_tail,
};
#[cfg(test)]
pub(super) use conversation::{
    message_model_content_parts, model_conversation_message, referenced_image_message_model_content,
};
pub(crate) use http::{ContextStatusResponse, ContextUsageMetrics};
pub(crate) use round_compaction::ServerRoundContextCompactor;
#[cfg(test)]
pub(super) use turn_context::thread_context_snapshot_changed;
pub(super) use turn_context::{
    build_turn_model_context, prepare_turn_context, turn_context_reservation,
};

pub(super) fn router() -> Router<AppState> {
    http::router()
}
