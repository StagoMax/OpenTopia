use super::{
    ApplyPatchTool, BackgroundOutputTool, BrowserTool, ComputerTool, CreateSkillTool, DocumentTool,
    FilesystemTool, FollowupAgentTaskTool, InterruptAgentTool, ListAgentsTool, ListSkillsTool,
    PdfTool, ReadArtifactTool, ReadAttachmentTool, ReadSkillTool, RegisteredTool,
    RequestUserInputTool, SendAgentMessageTool, ShellTool, SpawnAgentTool, SpreadsheetDescribeTool,
    SpreadsheetExecuteTool, SpreadsheetInspectTool, SpreadsheetTool, Tool, ToolApprovalMode,
    ToolCapabilityDescriptor, ToolClass, ToolExecutionPolicy, ToolGovernance, ToolModelVisibility,
    ToolRiskLevel, ToolSideEffect, ToolSource, UpdatePlanTool, ViewAttachmentTool, WaitAgentTool,
};
use crate::bundled_plugins::bundled_plugin_catalog;
use crate::enterprise::DataClassification;
use crate::model::ToolCall;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Immutable, clone-cheap catalog of complete tool registration records.
///
/// A record owns the implementation, source, class, and governance metadata.
/// The tool implementation remains the sole source of its ID, description,
/// Schema, execution intent, and execution behavior.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    entries: Arc<BTreeMap<String, RegisteredTool>>,
}

impl ToolRegistry {
    /// Compose the complete first-party surface shipped by OpenTopia.
    pub fn with_builtins() -> Self {
        let mut registry = Self::with_core_tools();
        registry.register_bundled_plugins();
        registry
    }

    /// Compose the trusted kernel tools without optional bundled plugins.
    pub fn with_core_tools() -> Self {
        let mut registry = Self::default();
        registry.register_core(
            Arc::new(ReadAttachmentTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::Low,
                ToolSideEffect::None,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(ViewAttachmentTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::External,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(ReadArtifactTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::Low,
                ToolSideEffect::None,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(FilesystemTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::WorkspaceWrite,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(ShellTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::Process,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(BackgroundOutputTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::Low,
                ToolSideEffect::None,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        registry.register_core(
            Arc::new(ApplyPatchTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::WorkspaceWrite,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );

        for tool in [
            Arc::new(SpawnAgentTool) as Arc<dyn Tool>,
            Arc::new(SendAgentMessageTool),
            Arc::new(FollowupAgentTaskTool),
            Arc::new(InterruptAgentTool),
        ] {
            registry.register_core(
                tool,
                ToolClass::Agent,
                governed(
                    ToolRiskLevel::Medium,
                    ToolSideEffect::ControlPlane,
                    ToolApprovalMode::PolicyControlled,
                    DataClassification::Confidential,
                ),
            );
        }
        for tool in [
            Arc::new(ListAgentsTool) as Arc<dyn Tool>,
            Arc::new(WaitAgentTool),
        ] {
            registry.register_core(
                tool,
                ToolClass::Agent,
                governed(
                    ToolRiskLevel::Low,
                    ToolSideEffect::None,
                    ToolApprovalMode::PolicyControlled,
                    DataClassification::Restricted,
                ),
            );
        }
        registry.register_core(
            Arc::new(RequestUserInputTool),
            ToolClass::StructuredInput,
            governed(
                ToolRiskLevel::Medium,
                ToolSideEffect::SessionMutation,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Confidential,
            ),
        );
        registry.register_core(
            Arc::new(UpdatePlanTool),
            ToolClass::WorkForm,
            governed(
                ToolRiskLevel::Medium,
                ToolSideEffect::SessionMutation,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Confidential,
            ),
        );
        for tool in [
            Arc::new(ListSkillsTool) as Arc<dyn Tool>,
            Arc::new(ReadSkillTool),
        ] {
            registry.register_core(
                tool,
                ToolClass::Standard,
                governed(
                    ToolRiskLevel::Low,
                    ToolSideEffect::None,
                    ToolApprovalMode::PolicyControlled,
                    DataClassification::Restricted,
                ),
            );
        }
        registry.register_core(
            Arc::new(CreateSkillTool),
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::WorkspaceWrite,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Restricted,
            ),
        );
        for registration in crate::flow_tools::flow_tool_registrations() {
            registry.register_entry(registration);
        }
        for registration in crate::agent_tools::agent_tool_registrations() {
            registry.register_entry(registration);
        }
        registry
    }

    fn register_bundled_plugins(&mut self) {
        for plugin in bundled_plugin_catalog() {
            for capability in plugin.native_capabilities {
                let (tool, governance): (Arc<dyn Tool>, ToolGovernance) = match *capability {
                    "browser" => (
                        Arc::new(BrowserTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::External,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Public,
                        ),
                    ),
                    "computer" => (
                        Arc::new(ComputerTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::External,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Public,
                        ),
                    ),
                    "document" => (
                        Arc::new(DocumentTool),
                        governed(
                            ToolRiskLevel::Medium,
                            ToolSideEffect::Process,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "pdf" => (
                        Arc::new(PdfTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet" => (
                        Arc::new(SpreadsheetTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_inspect" => (
                        Arc::new(SpreadsheetInspectTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_describe" => (
                        Arc::new(SpreadsheetDescribeTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_execute" => (
                        Arc::new(SpreadsheetExecuteTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    _ => continue,
                };
                let registration = RegisteredTool::new(
                    tool,
                    ToolSource::BundledPlugin {
                        plugin_name: plugin.name.to_string(),
                    },
                    ToolClass::Standard,
                    governance,
                );
                // The v1 action-union implementation is retained as the shared
                // executor for persisted calls and the v2 protocol adapter. It
                // must never contribute its large schema to a model request.
                let registration = if *capability == "spreadsheet" {
                    registration.internal_only()
                } else {
                    registration
                };
                self.register_entry(registration);
            }
        }
    }

    fn register_core(&mut self, tool: Arc<dyn Tool>, class: ToolClass, governance: ToolGovernance) {
        self.register_entry(RegisteredTool::core(tool, class, governance));
    }

    fn register_entry(&mut self, entry: RegisteredTool) {
        let name = entry.name().to_string();
        Arc::make_mut(&mut self.entries).insert(name, entry);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.entries.get(name).map(|entry| Arc::clone(&entry.tool))
    }

    /// Registers a server-composed local tool without repeating its ID. When a
    /// tool replaces an existing record, its static class, model visibility,
    /// and governance stay attached to that registry slot.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name();
        let (class, model_visibility, governance) = self
            .entries
            .get(name)
            .map(|entry| {
                (
                    entry.class,
                    entry.model_visibility,
                    entry.governance.clone(),
                )
            })
            .unwrap_or((
                ToolClass::Standard,
                ToolModelVisibility::Visible,
                ToolGovernance::unknown(),
            ));
        let mut registration = RegisteredTool::core(tool, class, governance);
        registration.model_visibility = model_visibility;
        self.register_entry(registration);
    }

    /// Compatibility entry point for callers that still pass a duplicate ID.
    /// The assertion turns silent registry drift into an immediate failure.
    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool>) {
        assert_eq!(
            name,
            tool.name(),
            "registered tool ID must match Tool::name()"
        );
        self.register(tool);
    }

    pub fn register_mcp(&mut self, tool: Arc<dyn Tool>) {
        self.register_entry(RegisteredTool::new(
            tool,
            ToolSource::Mcp,
            ToolClass::Standard,
            governed(
                ToolRiskLevel::High,
                ToolSideEffect::External,
                ToolApprovalMode::PolicyControlled,
                DataClassification::Public,
            ),
        ));
    }

    /// Compatibility entry point for older callers. New code should use
    /// register_mcp so the implementation remains the only source of its ID.
    pub fn insert_mcp(&mut self, name: String, tool: Arc<dyn Tool>) {
        assert_eq!(
            name,
            tool.name(),
            "registered MCP ID must match Tool::name()"
        );
        self.register_mcp(tool);
    }

    /// Atomically removes the previously synchronized MCP surface while
    /// preserving core and bundled tools.
    pub fn clear_mcp(&mut self) {
        Arc::make_mut(&mut self.entries)
            .retain(|_, entry| !matches!(entry.source, ToolSource::Mcp));
    }

    pub(crate) fn retain_mcp_where(&mut self, mut retain: impl FnMut(&str) -> bool) {
        Arc::make_mut(&mut self.entries).retain(|name, entry| {
            !matches!(entry.source, ToolSource::Mcp) || retain(name.as_str())
        });
    }

    pub fn list(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub fn source(&self, name: &str) -> Option<ToolSource> {
        self.entries.get(name).map(|entry| entry.source.clone())
    }

    pub fn class(&self, name: &str) -> Option<ToolClass> {
        self.entries.get(name).map(|entry| entry.class)
    }

    pub(crate) fn is_model_visible(&self, name: &str) -> bool {
        self.entries
            .get(name)
            .is_some_and(|entry| entry.model_visibility == ToolModelVisibility::Visible)
    }

    pub fn execution_policy(&self, name: &str, call: &ToolCall) -> Option<ToolExecutionPolicy> {
        self.entries
            .get(name)
            .map(|entry| entry.tool.execution_policy(call))
    }

    pub fn capability_catalog(&self) -> Vec<ToolCapabilityDescriptor> {
        self.entries
            .values()
            .map(RegisteredTool::capability_descriptor)
            .collect()
    }
}

fn governed(
    risk: ToolRiskLevel,
    side_effect: ToolSideEffect,
    approval: ToolApprovalMode,
    max_data_classification: DataClassification,
) -> ToolGovernance {
    ToolGovernance::new(risk, side_effect, approval, max_data_classification)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_keys_are_derived_from_tool_implementations() {
        let registry = ToolRegistry::with_builtins();
        for name in registry.list() {
            let tool = registry.get(&name).expect("registered tool");
            assert_eq!(name, tool.name());
            assert!(registry.source(&name).is_some());
            assert!(registry.class(&name).is_some());
        }
    }

    #[test]
    fn legacy_spreadsheet_executor_is_internal_only() {
        let mut registry = ToolRegistry::with_builtins();

        assert!(registry.get("spreadsheet").is_some());
        assert!(!registry.is_model_visible("spreadsheet"));
        registry.register(Arc::new(SpreadsheetTool));
        assert!(!registry.is_model_visible("spreadsheet"));
        for name in [
            "spreadsheet_inspect",
            "spreadsheet_describe",
            "spreadsheet_execute",
        ] {
            assert!(registry.is_model_visible(name), "{name} should be visible");
        }
    }

    #[test]
    #[should_panic(expected = "registered tool ID must match Tool::name()")]
    fn compatibility_registration_rejects_a_drifting_id() {
        let mut registry = ToolRegistry::default();
        registry.insert("wrong_name".to_string(), Arc::new(ReadArtifactTool));
    }

    #[test]
    fn clearing_mcp_preserves_non_mcp_registration_records() {
        let mut registry = ToolRegistry::with_core_tools();
        registry.register_mcp(Arc::new(ReadArtifactTool));
        assert_eq!(registry.source("read_artifact"), Some(ToolSource::Mcp));

        registry.clear_mcp();

        assert!(registry.get("read_artifact").is_none());
        assert!(registry.get("filesystem").is_some());
        assert_eq!(registry.source("filesystem"), Some(ToolSource::Core));
    }
}
