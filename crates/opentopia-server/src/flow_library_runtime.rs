use crate::library_api::{LibraryProviderRegistry, LibrarySearchTool};
use crate::ApiError;
use opentopia_core::{
    AgentRunDraft, CompiledWorkflowV1, FlowRunV1, WorkflowAgentSpecV1, WorkflowLibraryProviderV1,
};
use serde::{Deserialize, Deserializer};
use std::sync::Arc;

/// Distinguishes an omitted activation field (preserve the prior revision's
/// provider) from an explicit JSON null (disable Library access).
#[derive(Debug, Default)]
pub(crate) struct WorkflowLibraryProviderUpdate(Option<Option<WorkflowLibraryProviderV1>>);

impl WorkflowLibraryProviderUpdate {
    pub(crate) fn resolve(
        self,
        current: Option<WorkflowLibraryProviderV1>,
    ) -> Option<WorkflowLibraryProviderV1> {
        self.0.unwrap_or(current)
    }
}

impl<'de> Deserialize<'de> for WorkflowLibraryProviderUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<WorkflowLibraryProviderV1>::deserialize(deserializer)
            .map(|provider| Self(Some(provider)))
    }
}

pub(crate) fn validate_workflow_library_provider(
    compiled: &CompiledWorkflowV1,
    provider: Option<WorkflowLibraryProviderV1>,
) -> Result<(), ApiError> {
    if provider.is_some()
        && !compiled
            .agent_specs
            .values()
            .any(|spec| spec.capabilities.allows_tool("library_search"))
    {
        return Err(ApiError::bad_request(
            "Library provider requires at least one Agent with the library_search capability",
        ));
    }
    if provider == Some(WorkflowLibraryProviderV1::GraphRag)
        && compiled
            .agent_specs
            .values()
            .any(|spec| spec.knowledge_binding.is_some())
    {
        return Err(ApiError::bad_request(
            "Graph RAG provider cannot be combined with a fixed SAG namespace binding",
        ));
    }
    Ok(())
}

pub(crate) fn register_workflow_library_tool(
    agent: &mut AgentRunDraft,
    providers: Arc<LibraryProviderRegistry>,
    run: &FlowRunV1,
    specs: &[WorkflowAgentSpecV1],
) {
    if specs.iter().any(|spec| spec.knowledge_binding.is_some()) {
        agent.register_runtime_tool(Arc::new(LibrarySearchTool::runtime_scoped(providers)));
        return;
    }
    let Some(provider) = run
        .flow_revision
        .as_ref()
        .and_then(|revision| revision.library_provider)
        .filter(|_| {
            specs
                .iter()
                .any(|spec| spec.capabilities.allows_tool("library_search"))
        })
    else {
        return;
    };
    agent.register_runtime_tool(Arc::new(LibrarySearchTool::new(providers, provider.into())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_patch_preserves_sets_and_clears() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Request {
            #[serde(default)]
            library_provider: WorkflowLibraryProviderUpdate,
        }

        let missing: Request = serde_json::from_value(json!({})).expect("missing provider");
        let set: Request =
            serde_json::from_value(json!({"libraryProvider": "graph-rag"})).expect("set provider");
        let clear: Request =
            serde_json::from_value(json!({"libraryProvider": null})).expect("clear provider");

        assert_eq!(
            missing
                .library_provider
                .resolve(Some(WorkflowLibraryProviderV1::Sag)),
            Some(WorkflowLibraryProviderV1::Sag)
        );
        assert_eq!(
            set.library_provider.resolve(None),
            Some(WorkflowLibraryProviderV1::GraphRag)
        );
        assert_eq!(
            clear
                .library_provider
                .resolve(Some(WorkflowLibraryProviderV1::Sag)),
            None
        );
    }
}
