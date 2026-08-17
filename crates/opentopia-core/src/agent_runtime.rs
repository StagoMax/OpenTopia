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

/// One typed signal for every resumable interactive boundary.
///
/// The product layer may keep approval and structured-input persistence in
/// separate stores, but the turn runtime resumes through one control-plane
/// operation. Adding another suspension kind therefore does not add another
/// driver method.
#[derive(Debug, Clone)]
pub enum AgentResumeSignal {
    Approval {
        approval_id: Option<Uuid>,
        approved: bool,
    },
    UserInput {
        request_id: Uuid,
        response: UserInputResponse,
    },
    ExternalAction {
        observation: String,
    },
}

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

    /// Resume the exact boundary captured by a continuation.
    async fn resume_turn(
        &self,
        continuation: AgentContinuation,
        signal: AgentResumeSignal,
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

    async fn resume_turn(
        &self,
        continuation: AgentContinuation,
        signal: AgentResumeSignal,
        store: Option<Arc<dyn SessionStore>>,
        cancellation: Option<CancellationToken>,
        sender: Option<AgentEventSender>,
    ) -> anyhow::Result<AgentTurnResult> {
        self.resume_from_signal_streaming(continuation, signal, store, cancellation, sender)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentTurnOutcome;
    use crate::model::AgentEventPayload;
    use crate::policy::PermissionMode;

    fn accepts_object_safe_driver(_driver: &dyn AgentTurnDriver) {}

    #[test]
    fn agent_core_implements_object_safe_turn_driver() {
        let agent = AgentCore::default();
        accepts_object_safe_driver(&agent);
    }

    #[test]
    fn turn_driver_boundary_does_not_import_concrete_product_runtimes() {
        let source = include_str!("agent_runtime.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("agent runtime has production source");
        for forbidden in [
            "crate::background",
            "crate::browser",
            "crate::computer",
            "crate::guardian",
            "crate::mcp",
            "crate::subagents",
            "crate::tools",
        ] {
            assert!(
                !production_source.contains(forbidden),
                "turn driver boundary must not depend on {forbidden}"
            );
        }
    }

    #[test]
    fn driver_delegates_tool_batch_and_model_request_materialization() {
        let source = include_str!("agent.rs");
        let production_source = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("agent has production source");
        for migrated_concern in [
            "futures_util::future::join_all",
            "EffectIntent",
            "DurableAsyncToolResultSink",
            "let request = ModelRequest {",
            ".complete(ModelRequest {",
        ] {
            assert!(
                !production_source.contains(migrated_concern),
                "driver must delegate migrated concern `{migrated_concern}`"
            );
        }
        assert!(production_source.contains("execute_provider_batch(inputs)"));
        assert!(production_source.contains("context_assembler.compile(ContextAssemblyInput"));
    }

    #[tokio::test]
    async fn driver_event_sequence_is_preserved_behind_the_port() {
        let driver = AgentCore::default();
        let result = AgentTurnDriver::run_turn(
            &driver,
            AgentTurnInput {
                thread_id: Uuid::from_u128(1),
                user_message_id: Uuid::from_u128(2),
                workspace_root: std::env::temp_dir(),
                content: "event-sequence-golden".to_string(),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: None,
                cancellation: None,
            },
            None,
            None,
        )
        .await
        .expect("turn driver completes behind the port");

        assert!(matches!(result.outcome, AgentTurnOutcome::Completed));
        let sequence = result
            .events
            .iter()
            .map(|event| match event {
                AgentEventPayload::TurnStarted { .. } => "turn_started",
                AgentEventPayload::ModelContextBuilt { .. } => "model_context_built",
                AgentEventPayload::ModelRequest { .. } => "model_request",
                AgentEventPayload::ProviderRequestSent { .. } => "provider_request_sent",
                AgentEventPayload::ModelDelta { .. } => "model_delta",
                AgentEventPayload::ProviderResponseReceived { .. } => "provider_response_received",
                AgentEventPayload::AssistantMessage { .. } => "assistant_message",
                AgentEventPayload::TurnFinished { .. } => "turn_finished",
                _ => "unexpected",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            sequence,
            vec![
                "turn_started",
                "model_context_built",
                "model_request",
                "provider_request_sent",
                "model_delta",
                "provider_response_received",
                "assistant_message",
                "turn_finished",
            ]
        );
    }
}
