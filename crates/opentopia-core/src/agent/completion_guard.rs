use super::{
    AgentCompletionGuardDelivery, AgentCore, FinalizationGuardIntervention, TurnEvents,
    FINALIZATION_GUARD_TOOL_NAME, MAX_FINALIZATION_GUARD_ACTIVATIONS,
};
use crate::background::BackgroundScope;
use crate::completion_runtime::CompletionSignal;
use crate::model::{AgentEventPayload, ApprovalStatus, ModelContentPart};
use crate::provider::{ProviderToolCall, ProviderToolResult};
use crate::store::SessionStore;
use crate::work_form::{WorkForm, WorkScope};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

impl AgentCore {
    /// Prevents a final response while durable runtime obligations remain unresolved.
    ///
    /// This is deliberately separate from the provider loop: model completion is
    /// only a proposal, while the harness owns readiness and evidence invariants.
    pub(super) async fn apply_finalization_guard(
        &self,
        thread_id: Uuid,
        fallback_turn_id: Uuid,
        store: Option<&Arc<dyn SessionStore>>,
        pending_tool_calls: &[ProviderToolCall],
        provider_tool_calls: &mut Vec<ProviderToolCall>,
        provider_tool_results: &mut Vec<ProviderToolResult>,
        provider_response_items: &mut Vec<Value>,
        events: &mut TurnEvents,
    ) -> anyhow::Result<Option<FinalizationGuardIntervention>> {
        let mut blockers = Vec::new();
        if !pending_tool_calls.is_empty() {
            blockers.push(json!({
                "kind": "pending_tool_calls",
                "count": pending_tool_calls.len(),
            }));
        }

        if let Some(store) = store {
            let pending_approvals =
                store.list_approvals(thread_id, Some(ApprovalStatus::Pending))?;
            if !pending_approvals.is_empty() {
                blockers.push(json!({
                    "kind": "pending_approvals",
                    "approvalIds": pending_approvals.iter().map(|approval| approval.approval_id).collect::<Vec<_>>(),
                }));
            }
        }

        let turn_id = self.agent_turn_id.unwrap_or(fallback_turn_id);
        let mut registered_forms = Vec::new();
        if let Some(store) = store {
            if let Some(form) = store.get_work_form_for_scope(WorkScope::Turn(turn_id))? {
                registered_forms.push(form);
            }
            if let Some(goal_id) = self.goal.as_ref().map(|goal| goal.id) {
                if let Some(form) = store.get_work_form_for_scope(WorkScope::Goal(goal_id))? {
                    registered_forms.push(form);
                }
            }
        }
        if registered_forms.is_empty() {
            if let Some(goal) = self.goal.as_ref() {
                registered_forms.push(WorkForm::empty_goal(
                    thread_id,
                    goal.id,
                    goal.objective.clone(),
                ));
            }
        }
        let mut agent_delivery = None;
        if let Some(collaboration) = self.collaboration.as_ref() {
            let snapshot = collaboration.completion_snapshot().await?;
            if !snapshot.active_descendants.is_empty() || !snapshot.pending_messages.is_empty() {
                blockers.push(json!({
                    "kind": "descendant_agents_unresolved",
                    "activeAgents": snapshot.active_descendants,
                    "messages": &snapshot.pending_messages,
                }));
                agent_delivery = Some(AgentCompletionGuardDelivery {
                    messages: snapshot.pending_messages,
                });
            }
        }

        let mut completion_signals = blockers
            .into_iter()
            .map(|details| {
                let source_id = details
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("legacy_runtime_state")
                    .to_string();
                let message = details
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("The runtime finalization check is unresolved.")
                    .to_string();
                CompletionSignal::blocking(source_id, message, details)
            })
            .collect::<Vec<_>>();
        completion_signals.extend(self.completion_registry.signals(&registered_forms));
        let background_scope = BackgroundScope {
            thread_id,
            agent_path: self.agent_path.clone(),
        };
        completion_signals.extend(
            self.tool_host
                .background
                .list(&background_scope)
                .into_iter()
                .filter(|job| !job.status.is_terminal())
                .map(|job| {
                    CompletionSignal::advisory(
                        format!("background:{}", job.job_id),
                        format!(
                            "Background job `{}` is still running; it does not block this turn, and its terminal tool result will be appended automatically.",
                            job.command
                        ),
                        json!({
                            "kind": "background_job_running",
                            "jobId": job.job_id,
                            "command": job.command,
                            "status": job.status,
                        }),
                    )
                }),
        );
        let completion_report = self.completion_gate.check(completion_signals);
        for reminder in completion_report.reminders {
            events.push(AgentEventPayload::ContextWarning {
                stage: "completion_advisory".to_string(),
                message: reminder.message,
            });
        }
        let blockers = completion_report
            .blockers
            .into_iter()
            .map(|signal| signal.details)
            .collect::<Vec<_>>();

        if blockers.is_empty() {
            return Ok(None);
        }

        let prior_activations = provider_tool_calls
            .iter()
            .filter(|call| call.name == FINALIZATION_GUARD_TOOL_NAME)
            .count();
        if prior_activations >= MAX_FINALIZATION_GUARD_ACTIVATIONS {
            anyhow::bail!(
                "finalization guard remained unresolved after {MAX_FINALIZATION_GUARD_ACTIVATIONS} model retries: {}",
                serde_json::to_string(&blockers)?
            );
        }

        let payload = json!({
            "status": "completion_blocked",
            "reason": "The runtime finalization checks are not yet satisfied.",
            "agentPath": self.agent_path,
            "blockers": blockers,
            "requiredAction": [
                "Resolve the reported runtime state using the appropriate tool, plan update, or explicit user request.",
                "Only return a final response after the runtime state is ready."
            ]
        });
        let call_id = format!("completion_guard_{}", Uuid::new_v4());
        let call = ProviderToolCall {
            id: call_id.clone(),
            name: FINALIZATION_GUARD_TOOL_NAME.to_string(),
            arguments: json!({ "agentPath": self.agent_path }),
        };
        let output = serde_json::to_string_pretty(&payload)?;
        provider_response_items.push(json!({
            "type": "function_call",
            "call_id": call_id,
            "name": FINALIZATION_GUARD_TOOL_NAME,
            "arguments": call.arguments.to_string(),
        }));
        provider_tool_calls.push(call);
        provider_tool_results.push(ProviderToolResult {
            call_id,
            name: FINALIZATION_GUARD_TOOL_NAME.to_string(),
            output,
            content: vec![ModelContentPart::json(payload)],
            is_error: false,
            metadata: json!({
                "runtimeGuard": "finalization",
                "success": true,
            }),
        });
        events.push(AgentEventPayload::ContextWarning {
            stage: "finalization_guard".to_string(),
            message: "Final response deferred because runtime readiness checks are unresolved."
                .to_string(),
        });
        Ok(Some(FinalizationGuardIntervention { agent_delivery }))
    }
}
