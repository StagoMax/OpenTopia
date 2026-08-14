//! Serializable state captured at an interactive turn boundary.
//!
//! A continuation is correctness state for resuming the same turn. It is kept
//! separate from provider cursors, which are disposable cross-turn
//! optimizations.

use super::{ContextBudget, RolloutBudget, TurnRuntimeState};
use crate::model::{CollaborationMode, GoalRecord, ModelContentPart};
use crate::model_context::CompiledModelContext;
use crate::policy::PermissionMode;
use crate::provider::{
    ModelConversationMessage, ProviderToolCall, ProviderToolCandidate, ProviderToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

/// Complete state required to resume an approval or structured-input boundary
/// without repeating already committed model or tool work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContinuation {
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub workspace_root: PathBuf,
    pub context_summary: Option<String>,
    pub conversation: Vec<ModelConversationMessage>,
    pub permission_mode: PermissionMode,
    pub context_budget: Option<ContextBudget>,
    #[serde(default)]
    pub rollout_budget: Option<RolloutBudget>,
    #[serde(default)]
    pub model_context: CompiledModelContext,
    #[serde(default)]
    pub collaboration_mode: CollaborationMode,
    #[serde(default)]
    pub goal: Option<GoalRecord>,
    pub state: AgentContinuationState,
}

/// Loop-specific continuation payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentContinuationState {
    Provider {
        model_user_message: String,
        #[serde(default)]
        model_user_content: Vec<ModelContentPart>,
        tool_candidates: Vec<ProviderToolCandidate>,
        provider_tool_calls: Vec<ProviderToolCall>,
        provider_tool_results: Vec<ProviderToolResult>,
        pending_tool_calls: Vec<ProviderToolCall>,
        #[serde(default)]
        compacted_tool_history: String,
        #[serde(default)]
        provider_response_items: Vec<Value>,
        #[serde(default = "default_continuation_model_rounds")]
        model_rounds: usize,
        #[serde(default)]
        rollout_reviews: usize,
        #[serde(default)]
        runtime_state: TurnRuntimeState,
        #[serde(default)]
        branch_developer_instructions: Option<String>,
        #[serde(default)]
        provider_compatibility_hash: String,
    },
}

fn default_continuation_model_rounds() -> usize {
    1
}
