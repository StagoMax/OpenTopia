use crate::library_api::{LibraryProviderRegistry, LibrarySearchTool};
use opentopia_core::{AgentRunDraft, WorkflowAgentSpecV1};
use std::sync::Arc;

/// Registers one provider-neutral tool in the Flow harness. Each Agent node
/// projects its own immutable knowledge binding into the invocation context,
/// so a Flow can safely compose Agents backed by different libraries.
pub(crate) fn register_workflow_library_tool(
    agent: &mut AgentRunDraft,
    providers: Arc<LibraryProviderRegistry>,
    specs: &[WorkflowAgentSpecV1],
) {
    if specs.iter().any(|spec| spec.knowledge_binding.is_some()) {
        agent.register_runtime_tool(Arc::new(LibrarySearchTool::runtime_bound(providers)));
    }
}
