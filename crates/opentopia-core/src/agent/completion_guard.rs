use super::{
    latest_task_plan, latest_task_plan_from_store, successful_provider_tool_call_ids,
    AgentCompletionGuardDelivery, AgentCore, FinalizationGuardIntervention, TurnEvents,
    FINALIZATION_GUARD_TOOL_NAME, MAX_FINALIZATION_GUARD_ACTIVATIONS,
};
use crate::model::{
    AgentEventPayload, ApprovalStatus, CollaborationMode, ModelContentPart, TaskEvidenceKind,
    TaskPlanStepStatus,
};
use crate::provider::{ProviderToolCall, ProviderToolResult};
use crate::store::SessionStore;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

impl AgentCore {
    /// Prevents a final response while durable runtime obligations remain unresolved.
    ///
    /// This is deliberately separate from the provider loop: model completion is
    /// only a proposal, while the harness owns readiness and evidence invariants.
    pub(super) fn apply_finalization_guard(
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

        let latest_plan = if let Some(plan) = latest_task_plan(events, provider_tool_results) {
            Some(plan)
        } else if let Some(store) = store {
            latest_task_plan_from_store(store, thread_id)?
        } else {
            None
        };
        if self.collaboration_mode == CollaborationMode::Goal && latest_plan.is_none() {
            blockers.push(json!({
                "kind": "plan_missing",
                "reason": "This collaboration mode requires a durable plan created with set_plan.",
                "goalId": self.goal.as_ref().map(|goal| goal.id),
            }));
        }
        if let Some(plan) = latest_plan.as_ref() {
            let in_progress = plan
                .steps
                .iter()
                .filter(|step| step.status == TaskPlanStepStatus::InProgress)
                .map(|step| step.title.clone())
                .collect::<Vec<_>>();
            if self.collaboration_mode != CollaborationMode::Plan && !in_progress.is_empty() {
                blockers.push(json!({
                    "kind": "plan_in_progress",
                    "steps": in_progress,
                }));
            }
            let pending = plan
                .steps
                .iter()
                .filter(|step| step.status == TaskPlanStepStatus::Pending)
                .map(|step| {
                    json!({
                        "id": step.id,
                        "title": step.title,
                        "dependencies": step.dependencies,
                    })
                })
                .collect::<Vec<_>>();
            if self.collaboration_mode != CollaborationMode::Plan && !pending.is_empty() {
                blockers.push(json!({
                    "kind": "plan_pending",
                    "steps": pending,
                    "nextRunnableStep": plan.next_runnable_step().map(|step| json!({
                        "id": step.id,
                        "title": step.title,
                        "status": step.status,
                    })),
                    "reason": "Every pending step must be completed or explicitly resolved as deferred, blocked, or cancelled before finalizing.",
                }));
            }
            if self.collaboration_mode != CollaborationMode::Plan {
                if let Some(coverage) = plan.coverage.as_ref() {
                    let successful_tool_call_ids =
                        successful_provider_tool_call_ids(store, thread_id, events)?;
                    let completed_step_ids = plan
                        .steps
                        .iter()
                        .filter(|step| step.status == TaskPlanStepStatus::Completed)
                        .map(|step| step.id.as_str())
                        .collect::<HashSet<_>>();
                    let covered_requirement_ids = coverage
                        .step_requirements
                        .values()
                        .flatten()
                        .map(String::as_str)
                        .collect::<HashSet<_>>();
                    let uncovered = coverage
                        .requirements
                        .iter()
                        .filter(|requirement| {
                            !covered_requirement_ids.contains(requirement.id.as_str())
                        })
                        .map(|requirement| requirement.id.clone())
                        .collect::<Vec<_>>();
                    if !uncovered.is_empty() {
                        blockers.push(json!({
                            "kind": "requirements_uncovered",
                            "requirementIds": uncovered,
                            "requirementsRevision": coverage.requirements_revision,
                        }));
                    }

                    let invalid_evidence = coverage
                        .evidence_refs
                        .iter()
                        .filter(|evidence| {
                            evidence.requirements_revision != coverage.requirements_revision
                                || !completed_step_ids.contains(evidence.step_id.as_str())
                                || !successful_tool_call_ids.contains(&evidence.tool_call_id)
                        })
                        .map(|evidence| {
                            json!({
                                "stepId": evidence.step_id,
                                "requirementId": evidence.requirement_id,
                                "kind": evidence.kind,
                                "toolCallId": evidence.tool_call_id,
                                "evidenceRevision": evidence.requirements_revision,
                                "currentRequirementsRevision": coverage.requirements_revision,
                                "completedStep": completed_step_ids.contains(evidence.step_id.as_str()),
                                "successfulToolResult": successful_tool_call_ids.contains(&evidence.tool_call_id),
                            })
                        })
                        .collect::<Vec<_>>();
                    if !invalid_evidence.is_empty() {
                        blockers.push(json!({
                            "kind": "plan_evidence_invalid",
                            "evidence": invalid_evidence,
                            "reason": "Evidence must reference a successful recorded tool result for a completed step at the current requirements revision.",
                        }));
                    }

                    let valid_evidence = coverage
                        .evidence_refs
                        .iter()
                        .filter(|evidence| {
                            evidence.requirements_revision == coverage.requirements_revision
                                && completed_step_ids.contains(evidence.step_id.as_str())
                                && successful_tool_call_ids.contains(&evidence.tool_call_id)
                        })
                        .collect::<Vec<_>>();
                    let missing_fulfillment = coverage
                        .requirements
                        .iter()
                        .filter(|requirement| {
                            !valid_evidence.iter().any(|evidence| {
                                evidence.requirement_id == requirement.id
                                    && matches!(
                                        evidence.kind,
                                        TaskEvidenceKind::Implementation
                                            | TaskEvidenceKind::Observation
                                    )
                            })
                        })
                        .map(|requirement| requirement.id.clone())
                        .collect::<Vec<_>>();
                    if !missing_fulfillment.is_empty() {
                        blockers.push(json!({
                            "kind": "requirement_fulfillment_evidence_missing",
                            "requirementIds": missing_fulfillment,
                            "reason": "Each requirement needs current successful implementation or observation evidence.",
                        }));
                    }
                    let missing_verification = coverage
                        .requirements
                        .iter()
                        .filter(|requirement| {
                            !valid_evidence.iter().any(|evidence| {
                                evidence.requirement_id == requirement.id
                                    && evidence.kind == TaskEvidenceKind::Verification
                            })
                        })
                        .map(|requirement| requirement.id.clone())
                        .collect::<Vec<_>>();
                    if !missing_verification.is_empty() {
                        blockers.push(json!({
                            "kind": "requirement_verification_evidence_missing",
                            "requirementIds": missing_verification,
                            "reason": "Each requirement needs current successful verification evidence; global checks alone do not prove individual coverage.",
                        }));
                    }
                }
            }
        }
        let mut agent_delivery = None;
        if let Some(scheduler) = self.subagents.as_ref() {
            let scope = self.subagent_scope(thread_id, fallback_turn_id);
            let active_agents = scheduler
                .list_descendants_scoped(&scope)
                .into_iter()
                .filter(|run| !run.status.is_terminal())
                .map(|run| {
                    json!({
                        "id": run.id,
                        "agentPath": run.agent_path,
                        "status": run.status,
                        "agentType": run.agent_type,
                        "latestTask": run.last_task_message,
                    })
                })
                .collect::<Vec<_>>();
            let mailbox_snapshot = scheduler.mailbox_snapshot_scoped(&scope);
            if !active_agents.is_empty() || !mailbox_snapshot.is_empty() {
                blockers.push(json!({
                    "kind": "descendant_agents_unresolved",
                    "activeAgents": active_agents,
                    "messages": mailbox_snapshot,
                }));
                agent_delivery = Some(AgentCompletionGuardDelivery {
                    scope,
                    messages: mailbox_snapshot,
                });
            }
        }

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
