use super::{AgentCore, TurnRuntimeState};
use crate::execution_authorization::ExecutionGrant;
use crate::model::ToolCall;
use crate::policy::PermissionMode;
use crate::provider::ProviderToolCall;
use crate::sandbox::LocalSandboxConfig;
use crate::tool_runtime::ToolSchedulingInput;
use std::path::Path;

impl AgentCore {
    pub(super) fn tool_runtime_catalog(&self) -> crate::tool_runtime::ToolRuntimeCatalog {
        crate::tool_runtime::ToolRuntimeCatalog::new(
            self.tool_host.catalog.clone(),
            self.provider_tool_candidates(),
            self.capability_projection.clone(),
            self.allowed_tools.clone(),
            self.denied_tools.clone(),
            self.enabled_bundled_plugins.clone(),
        )
    }

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
            &self.tool_host.sandbox_config,
        )
    }

    pub(super) fn parallel_tool_call_indices_with_sandbox(
        &self,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
        permission_mode: PermissionMode,
        sandbox_config: &LocalSandboxConfig,
    ) -> Vec<usize> {
        let catalog = self.tool_runtime_catalog();
        self.kernel
            .tool_runtime
            .parallel_call_indices(ToolSchedulingInput {
                catalog: &catalog,
                calls,
                workspace_root,
                permission_mode,
                sandbox_config,
            })
    }

    pub(super) fn approved_parallel_tool_call_indices(
        &self,
        calls: &[ProviderToolCall],
    ) -> Vec<usize> {
        let catalog = self.tool_runtime_catalog();
        self.kernel
            .tool_runtime
            .approved_parallel_call_indices(&catalog, calls)
    }

    pub(super) fn approval_candidates(
        &self,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
        permission_mode: PermissionMode,
        sandbox_config: &LocalSandboxConfig,
    ) -> Vec<crate::tool_runtime::ToolApprovalCandidate> {
        let catalog = self.tool_runtime_catalog();
        self.kernel
            .tool_runtime
            .approval_candidates(ToolSchedulingInput {
                catalog: &catalog,
                calls,
                workspace_root,
                permission_mode,
                sandbox_config,
            })
    }

    /// Carries explicit, turn-scoped path approvals into later calls in the same turn.
    pub(super) fn grant_turn_path_leases(
        &self,
        runtime_state: &mut TurnRuntimeState,
        calls: &[ProviderToolCall],
        workspace_root: &Path,
    ) -> anyhow::Result<()> {
        let mut sandbox =
            runtime_state.sandbox_config_with_path_leases(&self.tool_host.sandbox_config);
        for provider_call in calls {
            let Some(tool) = self.tool_host.catalog.get(&provider_call.name) else {
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
