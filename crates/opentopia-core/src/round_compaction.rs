use crate::model::{ContextCompactionDetails, ContextSummary};
use crate::provider::ModelRequest;
use crate::AgentEventSender;
use async_trait::async_trait;
use std::collections::HashSet;
use uuid::Uuid;

pub fn context_compact_threshold_percent() -> usize {
    std::env::var("OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|value: usize| value.clamp(50, 95))
        .unwrap_or(80)
}

/// Immutable snapshot of the exact logical request captured immediately before
/// a provider round. The compactor summarizes this request directly; it does
/// not reconstruct or catch up a second history stream.
#[derive(Debug, Clone)]
pub struct RoundContextCompactionRequest {
    pub thread_id: Uuid,
    pub turn_id: Uuid,
    pub agent_path: String,
    pub round: usize,
    pub estimated_input_tokens: usize,
    pub reserved_generation_tokens: usize,
    pub context_window_tokens: usize,
    pub model_request: ModelRequest,
    /// The owning turn's single ordered event channel. A compactor must use
    /// this channel for live lifecycle events instead of persisting them on a
    /// second path, otherwise durable sequence numbers can violate causality.
    pub event_sender: Option<AgentEventSender>,
}

/// A durable checkpoint plus the completed tool ledger entries present in the
/// exact request that produced it. Pending calls absent from that request stay
/// live; completed calls included in it can be replaced by the checkpoint.
#[derive(Debug, Clone)]
pub struct RoundContextCompactionResult {
    pub summary: ContextSummary,
    pub details: Option<ContextCompactionDetails>,
    pub covered_tool_call_ids: HashSet<String>,
}

#[async_trait]
pub trait RoundContextCompactor: Send + Sync + std::fmt::Debug {
    async fn compact(
        &self,
        request: RoundContextCompactionRequest,
    ) -> anyhow::Result<RoundContextCompactionResult>;
}
