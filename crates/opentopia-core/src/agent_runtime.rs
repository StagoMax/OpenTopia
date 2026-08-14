//! Stable execution boundary between product lifecycle code and the concrete agent kernel.
//!
//! The server may still configure an [`AgentCore`] before a turn starts, but it
//! drives and resumes turns through [`AgentTurnDriver`]. This keeps HTTP,
//! persistence, and SSE ownership outside the loop while allowing another
//! trusted driver implementation to be introduced without changing those
//! product-layer call sites.

use crate::agent::{
    AgentContinuation, AgentCore, AgentEventSender, AgentTurnInput, AgentTurnResult,
};
use crate::model::UserInputResponse;
use crate::model_context::CompiledModelContext;
use crate::store::SessionStore;
use async_trait::async_trait;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Trusted, object-safe entry point for starting and resuming one agent turn.
///
/// This trait intentionally contains only control-loop operations. Agent
/// configuration and product lifecycle remain separate responsibilities.
#[async_trait]
pub trait AgentTurnDriver: Send + Sync {
    /// Start a new turn with an optional precompiled model context.
    async fn run_turn(
        &self,
        input: AgentTurnInput,
        model_context: Option<CompiledModelContext>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult>;

    /// Resume the exact approval boundary captured by a continuation.
    async fn resume_after_approval(
        &self,
        continuation: AgentContinuation,
        approved: bool,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult>;

    /// Resume the exact structured-input boundary captured by a continuation.
    async fn resume_after_user_input(
        &self,
        continuation: AgentContinuation,
        request_id: Uuid,
        response: UserInputResponse,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult>;
}

#[async_trait]
impl AgentTurnDriver for AgentCore {
    async fn run_turn(
        &self,
        input: AgentTurnInput,
        model_context: Option<CompiledModelContext>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.run_turn_detailed_streaming_with_context(input, model_context, sender)
            .await
    }

    async fn resume_after_approval(
        &self,
        continuation: AgentContinuation,
        approved: bool,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.resume_turn_streaming(continuation, approved, store, cancellation, sender)
            .await
    }

    async fn resume_after_user_input(
        &self,
        continuation: AgentContinuation,
        request_id: Uuid,
        response: UserInputResponse,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.resume_turn_with_user_input_streaming(
            continuation,
            request_id,
            response,
            store,
            cancellation,
            sender,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts_object_safe_driver(_driver: &dyn AgentTurnDriver) {}

    #[test]
    fn agent_core_implements_object_safe_turn_driver() {
        let agent = AgentCore::default();
        accepts_object_safe_driver(&agent);
    }
}
