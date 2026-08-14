use super::{AgentCore, AutomaticReviewBatchCandidate, TurnRuntimeState, MAX_PARALLEL_TOOL_CALLS};
use crate::execution_authorization::ExecutionGrant;
use crate::guardian::GuardianApprovalAction;
use crate::model::ToolCall;
use crate::policy::{
    ApprovalsReviewer, BasicPolicyEngine, PermissionMode, PolicyDecision, PolicyEngine,
};
use crate::provider::ProviderToolCall;
use crate::sandbox::LocalSandboxConfig;
use crate::tools::ToolContext;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

impl AgentCore {
    #[cfg(test)]
    pub(super) fn parallel_tool_call_indices(
        &self,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
        permission_mode: PermissionMode,
    ) -> Vec<usize> {
        self.parallel_tool_call_indices_with_sandbox(
            calls,
            workspace_root,
            permission_mode,
            &self.sandbox_config,
        )
    }

    /// Selects independent, already-authorized calls that can safely start in parallel.
    pub(super) fn parallel_tool_call_indices_with_sandbox(
        &self,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
        permission_mode: PermissionMode,
        sandbox_config: &LocalSandboxConfig,
    ) -> Vec<usize> {
        let policy_engine = BasicPolicyEngine::new_with_sandbox_config(
            workspace_root.to_path_buf(),
            permission_mode,
            sandbox_config,
        );
        let mut resource_keys = HashMap::<String, bool>::new();
        let mut selected = Vec::new();

        for (index, provider_call) in calls.iter().enumerate() {
            if selected.len() >= MAX_PARALLEL_TOOL_CALLS {
                break;
            }
            // Invalid and disabled calls do not execute or own resources. They
            // remain in provider order and therefore do not prevent independent
            // valid calls later in the same model batch from starting.
            if !self.tool_is_allowed(&provider_call.name)
                || self.provider_tool_input_error(provider_call).is_some()
            {
                continue;
            }
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let Some(tool) = self.tools.get(&provider_call.name) else {
                continue;
            };
            let execution_policy = tool.execution_policy(&call);
            if !execution_policy.parallel_safe {
                // A tool that declines the concurrency contract is an ordering
                // barrier. Do not speculatively run later side effects across it.
                break;
            }

            // Parallel execution must not turn an interactive authorization into
            // an implicit grant. Calls whose declared intent may Ask stay on the
            // existing sequential approval path.
            let intent = tool.execution_intent(&call, workspace_root);
            let shell_is_allowed = provider_call.name == "shell"
                && provider_call
                    .arguments
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| {
                        matches!(
                            policy_engine.inspect_command(command),
                            PolicyDecision::Allow
                        )
                    });
            let network_is_approval_free = intent.network.does_not_require_network()
                || shell_is_allowed
                || permission_mode == PermissionMode::FullAccess;
            let paths_are_approval_free = intent.requested_read_paths.iter().all(|path| {
                if path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return false;
                }
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    workspace_root.join(path)
                };
                matches!(policy_engine.inspect_read(&resolved), PolicyDecision::Allow)
            }) && intent.requested_write_paths.iter().all(|path| {
                if path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return false;
                }
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    workspace_root.join(path)
                };
                matches!(
                    policy_engine.inspect_write(&resolved),
                    PolicyDecision::Allow
                )
            });
            if !network_is_approval_free || !paths_are_approval_free {
                // An approval-bound call pauses at its own provider position,
                // but it does not prevent independent, already-authorized work
                // elsewhere in the same model batch from starting.
                continue;
            }

            let writes_resource = !execution_policy.read_only;
            let conflicts = execution_policy.resource_keys.iter().any(|key| {
                resource_keys.iter().any(|(selected_key, selected_writes)| {
                    let same_resource = key == selected_key
                        || key == "*"
                        || key == "workspace:*"
                        || selected_key == "*"
                        || selected_key == "workspace:*";
                    same_resource && (writes_resource || *selected_writes)
                })
            });
            if conflicts {
                continue;
            }
            for key in execution_policy.resource_keys {
                resource_keys
                    .entry(key)
                    .and_modify(|selected_writes| *selected_writes |= writes_resource)
                    .or_insert(writes_resource);
            }
            selected.push(index);
        }
        selected
    }

    pub(super) fn approved_parallel_tool_call_indices(
        &self,
        calls: &[ProviderToolCall],
    ) -> Vec<usize> {
        let mut resource_keys = HashMap::<String, bool>::new();
        let mut selected = Vec::new();

        for (index, provider_call) in calls.iter().enumerate() {
            if selected.len() >= MAX_PARALLEL_TOOL_CALLS {
                break;
            }
            if !self.tool_is_allowed(&provider_call.name)
                || self.provider_tool_input_error(provider_call).is_some()
            {
                break;
            }
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let Some(tool) = self.tools.get(&provider_call.name) else {
                break;
            };
            let execution_policy = tool.execution_policy(&call);
            if !execution_policy.parallel_safe {
                break;
            }

            let writes_resource = !execution_policy.read_only;
            let conflicts = execution_policy.resource_keys.iter().any(|key| {
                resource_keys.iter().any(|(selected_key, selected_writes)| {
                    let same_resource = key == selected_key
                        || key == "*"
                        || key == "workspace:*"
                        || selected_key == "*"
                        || selected_key == "workspace:*";
                    same_resource && (writes_resource || *selected_writes)
                })
            });
            if conflicts {
                continue;
            }
            for key in execution_policy.resource_keys {
                resource_keys
                    .entry(key)
                    .and_modify(|selected_writes| *selected_writes |= writes_resource)
                    .or_insert(writes_resource);
            }
            selected.push(index);
        }
        selected
    }

    /// Returns only a contiguous, side-effect-free preview of calls that are
    /// definitely going to Ask. A tool that cannot decide without entering its
    /// runtime is an ordering barrier and remains on the ordinary single-call
    /// path.
    pub(super) fn automatic_review_batch_candidates(
        &self,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
        permission_mode: PermissionMode,
        sandbox_config: &LocalSandboxConfig,
    ) -> Vec<AutomaticReviewBatchCandidate> {
        if permission_mode.approvals_reviewer() != ApprovalsReviewer::AutoReview {
            return Vec::new();
        }
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            workspace_root.to_path_buf(),
            permission_mode,
            sandbox_config,
        ));
        let mut ctx = ToolContext::local_with_sandbox_config(
            workspace_root.to_path_buf(),
            policy,
            sandbox_config.clone(),
        );
        ctx.permission_mode = permission_mode;
        let mut candidates = Vec::new();
        for provider_call in calls.iter().take(MAX_PARALLEL_TOOL_CALLS) {
            if !self.tool_is_allowed(&provider_call.name)
                || self.provider_tool_input_error(provider_call).is_some()
            {
                break;
            }
            let Some(tool) = self.tools.get(&provider_call.name) else {
                break;
            };
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let action = GuardianApprovalAction::from_provider_call(provider_call, workspace_root);
            if action.reviewability_error().is_some() {
                break;
            }
            match tool.authorization_preflight(&call, &ctx) {
                Some(PolicyDecision::Ask { reason }) => {
                    candidates.push(AutomaticReviewBatchCandidate {
                        call: provider_call.clone(),
                        reason,
                        action,
                    });
                }
                Some(PolicyDecision::Allow | PolicyDecision::Deny { .. }) | None => break,
            }
        }
        if candidates.len() >= 2 {
            candidates
        } else {
            Vec::new()
        }
    }

    /// Carries explicit, turn-scoped path approvals into later calls in the same turn.
    pub(super) fn grant_turn_path_leases(
        &self,
        runtime_state: &mut TurnRuntimeState,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
    ) -> anyhow::Result<()> {
        let mut sandbox = runtime_state.sandbox_config_with_path_leases(&self.sandbox_config);
        for provider_call in calls {
            let Some(tool) = self.tools.get(&provider_call.name) else {
                continue;
            };
            let call = ToolCall::new(&provider_call.name, provider_call.arguments.clone());
            let intent = tool.execution_intent(&call, workspace_root);
            let grant = ExecutionGrant::resolve(&sandbox, workspace_root, &intent, true)?;
            for path in grant.sandbox.approved_read_paths {
                sandbox.grant_read_path(path);
            }
            for path in grant.sandbox.approved_write_paths {
                sandbox.grant_write_path(path);
            }
        }
        runtime_state.replace_path_leases_from(&sandbox);
        Ok(())
    }
}
