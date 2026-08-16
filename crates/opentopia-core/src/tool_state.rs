use crate::effect_journal::{EffectIntent, EffectJournalRecord, EffectStatus};
use crate::model::{AgentEvent, Artifact, Message, ToolResult, TurnRecord};
use crate::store::SessionStore;
use crate::work_form::{WorkForm, WorkScope};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

/// Capability-limited persistence port exposed to tool execution.
///
/// Product orchestration may own a broad `SessionStore`, but tools receive only
/// the state operations required by execution, artifacts, WorkForms, and
/// durable asynchronous results. This prevents a tool implementation from
/// becoming coupled to unrelated thread, settings, agent, or project storage.
#[derive(Clone)]
pub struct ToolStateStore {
    inner: Arc<dyn SessionStore>,
}

impl ToolStateStore {
    pub fn new(inner: Arc<dyn SessionStore>) -> Self {
        Self { inner }
    }

    pub fn effective_plugin_settings(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
        thread_id: Uuid,
    ) -> anyhow::Result<Value> {
        self.inner
            .effective_plugin_settings(plugin_id, workspace_root, thread_id)
    }

    pub fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>> {
        self.inner.list_messages(thread_id)
    }

    pub fn append_message(&self, message: Message) -> anyhow::Result<Message> {
        self.inner.append_message(message)
    }

    pub fn append_event(&self, event: AgentEvent) -> anyhow::Result<AgentEvent> {
        self.inner.append_event(event)
    }

    pub fn get_active_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        self.inner.get_active_turn(thread_id)
    }

    pub fn clear_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<bool> {
        self.inner
            .clear_provider_conversation_state(thread_id, agent_path)
    }

    pub fn insert_artifact(&self, artifact: Artifact) -> anyhow::Result<Artifact> {
        self.inner.insert_artifact(artifact)
    }

    pub fn get_artifact(
        &self,
        thread_id: Uuid,
        artifact_id: Uuid,
    ) -> anyhow::Result<Option<Artifact>> {
        self.inner.get_artifact(thread_id, artifact_id)
    }

    pub fn insert_large_tool_output_artifact(
        &self,
        thread_id: Uuid,
        result: &ToolResult,
        threshold_bytes: usize,
    ) -> anyhow::Result<Option<Artifact>> {
        self.inner
            .insert_large_tool_output_artifact(thread_id, result, threshold_bytes)
    }

    pub fn upsert_work_form(&self, form: &WorkForm) -> anyhow::Result<WorkForm> {
        self.inner.upsert_work_form(form)
    }

    pub fn get_work_form_for_scope(&self, scope: WorkScope) -> anyhow::Result<Option<WorkForm>> {
        self.inner.get_work_form_for_scope(scope)
    }

    pub fn prepare_effect(&self, intent: &EffectIntent) -> anyhow::Result<EffectJournalRecord> {
        self.inner.prepare_effect(intent)
    }

    pub fn start_effect(&self, effect_id: Uuid) -> anyhow::Result<EffectJournalRecord> {
        self.inner.start_effect(effect_id)
    }

    pub fn finish_effect(
        &self,
        effect_id: Uuid,
        status: EffectStatus,
        result: Option<Value>,
        error: Option<String>,
    ) -> anyhow::Result<EffectJournalRecord> {
        self.inner.finish_effect(effect_id, status, result, error)
    }

    /// Temporary bridge for the independently owned Flow subsystem. Flow is
    /// intentionally outside the P6 refactor and keeps its current store API.
    pub(crate) fn flow_session_store(&self) -> &Arc<dyn SessionStore> {
        &self.inner
    }
}
