use super::{ContextBudget, ProviderConversationState};
use crate::effect_journal::{EffectIntent, EffectJournalRecord, EffectStatus};
use crate::enterprise::AgentTemplateVersionV1;
use crate::flow::{FlowDefinitionV1, FlowDraftV1, FlowTrialV1};
use crate::flow_runtime::FlowRunV1;
use crate::human_task::{HumanTaskStatusV1, HumanTaskV1};
use crate::model::{
    AgentEvent, Approval, ApprovalStatus, Artifact, ArtifactMetadata, ExperienceMode, GoalSnapshot,
    GoalStatus, Message, MessagePart, Project, TerminalCommandHistory, Thread,
    ThreadModelSelection, ToolResult, TurnChangeSet, TurnRecord, TurnStatus, UserInputRecord,
    UserInputRequest, UserInputResponse, UserInputStatus,
};
use crate::work_form::{WorkForm, WorkScope};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub trait SessionStore: Send + Sync + std::fmt::Debug {
    fn create_project(
        &self,
        name: String,
        workspace_root: Option<PathBuf>,
        pinned: bool,
        sort_order: i64,
    ) -> anyhow::Result<Project>;
    fn get_project(&self, id: Uuid) -> anyhow::Result<Option<Project>>;
    fn find_project_by_workspace(&self, workspace_root: &Path) -> anyhow::Result<Option<Project>>;
    fn find_or_create_project(
        &self,
        name: String,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Project>;
    fn list_projects(&self) -> anyhow::Result<Vec<Project>>;
    fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        workspace_root: Option<Option<PathBuf>>,
        pinned: Option<bool>,
        sort_order: Option<i64>,
    ) -> anyhow::Result<Option<Project>>;
    fn delete_project(&self, id: Uuid) -> anyhow::Result<bool>;
    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread>;
    fn create_thread_with_mode(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread>;
    fn create_thread_in_project(
        &self,
        title: Option<String>,
        project_id: Uuid,
    ) -> anyhow::Result<Thread>;
    fn create_thread_in_project_with_mode(
        &self,
        title: Option<String>,
        project_id: Uuid,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread>;
    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>>;
    fn effective_plugin_settings(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
        thread_id: Uuid,
    ) -> anyhow::Result<Value>;
    fn list_threads(&self) -> anyhow::Result<Vec<Thread>>;
    fn list_threads_including_archived(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<Thread>>;
    fn list_threads_for_mode(
        &self,
        include_archived: bool,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Vec<Thread>>;
    fn update_thread(
        &self,
        id: Uuid,
        title: Option<String>,
        project_id: Option<Option<Uuid>>,
        archived: Option<bool>,
    ) -> anyhow::Result<Option<Thread>>;
    /// Pins the model a thread runs with. Passing `None` clears the pin so the
    /// thread follows the active connection's default again.
    fn set_thread_model_selection(
        &self,
        id: Uuid,
        selection: Option<ThreadModelSelection>,
    ) -> anyhow::Result<Option<Thread>>;
    fn delete_thread(&self, id: Uuid) -> anyhow::Result<bool>;
    fn create_goal(
        &self,
        thread_id: Uuid,
        objective: String,
        token_budget: Option<u64>,
    ) -> anyhow::Result<GoalSnapshot>;
    fn get_goal(&self, id: Uuid) -> anyhow::Result<Option<GoalSnapshot>>;
    fn get_thread_goal(&self, thread_id: Uuid) -> anyhow::Result<Option<GoalSnapshot>>;
    fn update_goal_status(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        status: GoalStatus,
    ) -> anyhow::Result<Option<GoalSnapshot>>;
    fn update_goal_definition(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        objective: Option<String>,
        constraints: Option<Vec<String>>,
        acceptance: Option<Vec<String>>,
    ) -> anyhow::Result<Option<GoalSnapshot>>;
    fn upsert_work_form(&self, form: &WorkForm) -> anyhow::Result<WorkForm>;
    fn get_work_form(&self, form_id: Uuid) -> anyhow::Result<Option<WorkForm>>;
    fn get_work_form_for_scope(&self, scope: WorkScope) -> anyhow::Result<Option<WorkForm>>;
    fn add_goal_usage(
        &self,
        goal_id: Uuid,
        tokens: u64,
        elapsed_seconds: u64,
    ) -> anyhow::Result<Option<GoalSnapshot>>;
    fn append_message(&self, message: Message) -> anyhow::Result<Message>;
    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>>;
    fn enqueue_turn_message(&self, thread_id: Uuid, message_id: Uuid) -> anyhow::Result<()>;
    fn list_queued_turn_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Uuid>>;
    fn remove_queued_turn_message(&self, thread_id: Uuid, message_id: Uuid)
        -> anyhow::Result<bool>;
    fn insert_turn(&self, turn: TurnRecord) -> anyhow::Result<TurnRecord>;
    fn get_turn(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn get_active_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn get_latest_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn update_turn_status(
        &self,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>>;
    fn resume_turn_invocation(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn interrupt_active_turns(&self) -> anyhow::Result<usize>;
    fn upsert_turn_change_set(&self, change_set: &TurnChangeSet) -> anyhow::Result<()>;
    fn get_turn_change_set(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnChangeSet>>;
    fn list_turn_change_sets(&self, thread_id: Uuid) -> anyhow::Result<Vec<TurnChangeSet>>;
    fn mark_turn_change_set_reverted(
        &self,
        turn_id: Uuid,
        reverted_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<TurnChangeSet>>;
    fn append_event(&self, event: AgentEvent) -> anyhow::Result<AgentEvent>;
    fn append_events(&self, events: Vec<AgentEvent>) -> anyhow::Result<Vec<AgentEvent>>;
    fn list_events(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
    ) -> anyhow::Result<Vec<AgentEvent>>;
    fn prepare_effect(&self, intent: &EffectIntent) -> anyhow::Result<EffectJournalRecord>;
    fn get_effect(&self, effect_id: Uuid) -> anyhow::Result<Option<EffectJournalRecord>>;
    fn get_effect_by_idempotency_key(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        agent_path: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<Option<EffectJournalRecord>>;
    fn list_turn_effects(&self, turn_id: Uuid) -> anyhow::Result<Vec<EffectJournalRecord>>;
    fn start_effect(&self, effect_id: Uuid) -> anyhow::Result<EffectJournalRecord>;
    fn finish_effect(
        &self,
        effect_id: Uuid,
        status: EffectStatus,
        result: Option<Value>,
        error: Option<String>,
    ) -> anyhow::Result<EffectJournalRecord>;
    fn mark_running_effects_indeterminate(&self) -> anyhow::Result<usize>;
    fn insert_terminal_history(
        &self,
        history: TerminalCommandHistory,
    ) -> anyhow::Result<TerminalCommandHistory>;
    fn list_terminal_history(
        &self,
        thread_id: Uuid,
        after_seq: Option<u64>,
    ) -> anyhow::Result<Vec<TerminalCommandHistory>>;
    fn latest_terminal_history_seq(&self, thread_id: Uuid) -> anyhow::Result<u64>;
    fn insert_artifact(&self, artifact: Artifact) -> anyhow::Result<Artifact>;
    fn list_artifacts(&self, thread_id: Uuid) -> anyhow::Result<Vec<ArtifactMetadata>>;
    fn get_artifact(&self, thread_id: Uuid, artifact_id: Uuid) -> anyhow::Result<Option<Artifact>>;
    fn save_provider_conversation_state(
        &self,
        state: &ProviderConversationState,
    ) -> anyhow::Result<()>;
    fn get_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>>;
    fn take_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>>;
    fn clear_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<bool>;
    fn insert_approval(&self, approval: Approval) -> anyhow::Result<Approval>;
    fn get_approval(&self, approval_id: Uuid) -> anyhow::Result<Option<Approval>>;
    fn list_approvals(
        &self,
        thread_id: Uuid,
        status: Option<ApprovalStatus>,
    ) -> anyhow::Result<Vec<Approval>>;
    fn update_approval_status(
        &self,
        approval_id: Uuid,
        status: ApprovalStatus,
    ) -> anyhow::Result<Option<Approval>>;
    fn put_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
        continuation: Value,
    ) -> anyhow::Result<()>;
    fn get_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>>;
    fn delete_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<()>;
    fn put_user_input_request(
        &self,
        thread_id: Uuid,
        request: &UserInputRequest,
        continuation: Value,
    ) -> anyhow::Result<UserInputRecord>;
    fn get_user_input_request(&self, request_id: Uuid) -> anyhow::Result<Option<UserInputRecord>>;
    fn list_user_input_requests(
        &self,
        thread_id: Uuid,
        status: Option<UserInputStatus>,
    ) -> anyhow::Result<Vec<UserInputRecord>>;
    fn get_user_input_continuation(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>>;
    fn resolve_user_input_request(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
        response: &UserInputResponse,
    ) -> anyhow::Result<Option<UserInputRecord>>;
    fn put_turn_checkpoint(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
        wait_kind: &str,
        checkpoint: Value,
    ) -> anyhow::Result<()>;
    fn get_turn_checkpoint(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<(String, Value)>>;
    fn delete_turn_checkpoint(&self, turn_id: Uuid, thread_id: Uuid) -> anyhow::Result<bool>;
    fn put_turn_checkpoint_blob(&self, kind: &str, payload: Value) -> anyhow::Result<String>;
    fn get_turn_checkpoint_blob(&self, content_hash: &str) -> anyhow::Result<Option<Value>>;
    fn get_published_agent_template_version(
        &self,
        template_id: &str,
        version: u32,
    ) -> anyhow::Result<Option<AgentTemplateVersionV1>>;
    fn create_flow_draft(&self, draft: &FlowDraftV1) -> anyhow::Result<FlowDraftV1>;
    fn get_flow_draft(&self, draft_id: Uuid) -> anyhow::Result<Option<FlowDraftV1>>;
    fn list_flow_drafts(&self, thread_id: Option<Uuid>) -> anyhow::Result<Vec<FlowDraftV1>>;
    fn update_flow_draft(
        &self,
        draft: &FlowDraftV1,
        expected_revision: u32,
    ) -> anyhow::Result<FlowDraftV1>;
    fn bind_thread_flow_draft(&self, thread_id: Uuid, draft_id: Uuid) -> anyhow::Result<()>;
    fn get_thread_flow_draft(&self, thread_id: Uuid) -> anyhow::Result<Option<FlowDraftV1>>;
    fn insert_flow_trial(&self, trial: &FlowTrialV1) -> anyhow::Result<FlowTrialV1>;
    fn list_flow_trials(&self, draft_id: Uuid) -> anyhow::Result<Vec<FlowTrialV1>>;
    fn publish_flow_draft(
        &self,
        draft_id: Uuid,
        published_by: &str,
    ) -> anyhow::Result<FlowDefinitionV1>;
    fn search_flow_definitions(&self, query: &str) -> anyhow::Result<Vec<FlowDefinitionV1>>;
    fn get_flow_definition(
        &self,
        flow_id: &str,
        version: Option<u32>,
    ) -> anyhow::Result<Option<FlowDefinitionV1>>;
    fn insert_flow_run(&self, run: &FlowRunV1) -> anyhow::Result<FlowRunV1>;
    fn get_flow_run(&self, run_id: Uuid) -> anyhow::Result<Option<FlowRunV1>>;
    fn list_flow_runs(&self, thread_id: Uuid) -> anyhow::Result<Vec<FlowRunV1>>;
    fn update_flow_run(&self, run: &FlowRunV1, expected_revision: u32)
        -> anyhow::Result<FlowRunV1>;
    fn insert_human_task(&self, task: &HumanTaskV1) -> anyhow::Result<HumanTaskV1>;
    fn get_human_task(&self, task_id: Uuid) -> anyhow::Result<Option<HumanTaskV1>>;
    fn list_human_tasks(
        &self,
        thread_id: Option<Uuid>,
        status: Option<HumanTaskStatusV1>,
    ) -> anyhow::Result<Vec<HumanTaskV1>>;
    fn get_pending_human_task_for_flow_run(
        &self,
        flow_run_id: Uuid,
    ) -> anyhow::Result<Option<HumanTaskV1>>;
    fn update_human_task(
        &self,
        task: &HumanTaskV1,
        expected_revision: u32,
    ) -> anyhow::Result<HumanTaskV1>;
    fn update_flow_run_and_human_task(
        &self,
        run: &FlowRunV1,
        expected_run_revision: u32,
        task: &HumanTaskV1,
        expected_task_revision: Option<u32>,
    ) -> anyhow::Result<(FlowRunV1, HumanTaskV1)>;

    fn insert_large_tool_output_artifact(
        &self,
        thread_id: Uuid,
        result: &ToolResult,
        threshold_bytes: usize,
    ) -> anyhow::Result<Option<Artifact>> {
        let output_bytes = result.output.len();
        if output_bytes <= threshold_bytes {
            return Ok(None);
        }
        let artifact = Artifact::inline(
            thread_id,
            "tool_output",
            "text/plain; charset=utf-8",
            result.output.clone(),
            serde_json::json!({
                "source": "tool_result",
                "callId": result.call_id,
                "outputBytes": output_bytes,
                "thresholdBytes": threshold_bytes,
                "toolResultMetadata": result.metadata.clone(),
            }),
        );
        self.insert_artifact(artifact).map(Some)
    }

    fn get_context_budget(&self, thread_id: Uuid) -> anyhow::Result<ContextBudget> {
        let messages = self.list_messages(thread_id)?;
        let message_count = messages.len();
        let total_tokens = std::env::var("OPENTOPIA_CONTEXT_WINDOW_TOKENS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value: &usize| *value >= 4_096)
            .unwrap_or(128_000);
        let mut used_tokens: usize = 0;
        for msg in &messages {
            let message_tokens: usize = msg
                .parts
                .iter()
                .map(|part| match part {
                    MessagePart::Text { text } => crate::model_context::estimate_tokens(text),
                    MessagePart::ToolResult { result } => {
                        crate::model_context::estimate_tokens(&result.output)
                    }
                    MessagePart::ToolCall { call } => {
                        crate::model_context::estimate_tokens(&call.name)
                            .saturating_add(crate::model_context::estimate_tokens(
                                &call.input.to_string(),
                            ))
                            .saturating_add(16)
                    }
                    _ => 16,
                })
                .sum();
            used_tokens = used_tokens.saturating_add(message_tokens.saturating_add(50));
        }
        let estimated_usage = if total_tokens > 0 {
            (used_tokens * 100) / total_tokens
        } else {
            0
        };
        Ok(ContextBudget {
            total_tokens,
            used_tokens,
            message_count,
            estimated_usage,
        })
    }
}
