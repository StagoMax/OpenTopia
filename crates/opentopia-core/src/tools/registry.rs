use super::{
    ApplyPatchTool, BackgroundOutputTool, BrowserTool, ComputerTool, CreateSkillTool, DocumentTool,
    FilesystemTool, FollowupAgentTaskTool, InterruptAgentTool, ListAgentsTool, ListSkillsTool,
    PdfTool, ReadArtifactTool, ReadAttachmentTool, ReadSkillTool, RegisteredTool,
    RequestUserInputTool, SendAgentMessageTool, ShellTool, SpawnAgentTool,
    SpreadsheetConvertRangesTool, SpreadsheetCopyRangesTool, SpreadsheetCopyRowsTool,
    SpreadsheetCopySheetTool, SpreadsheetDeleteRowsTool, SpreadsheetDeleteSheetTool,
    SpreadsheetExportDelimitedTool, SpreadsheetFillRangesTool, SpreadsheetFilterRowsTool,
    SpreadsheetFindTool, SpreadsheetInspectTool, SpreadsheetReadRangesTool,
    SpreadsheetValidateTool, SpreadsheetWriteRangeTool, Tool, ToolApprovalMode,
    ToolCapabilityDescriptor, ToolClass, ToolExecutionPolicy, ToolGovernance, ToolRiskLevel,
    ToolSideEffect, ToolSource, UpdatePlanTool, ViewAttachmentTool, WaitAgentTool,
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
                    "word_document" => (
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
                    "spreadsheet_inspect" => (
                        Arc::new(SpreadsheetInspectTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_read_ranges" => (
                        Arc::new(SpreadsheetReadRangesTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_find" => (
                        Arc::new(SpreadsheetFindTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_filter_rows" => (
                        Arc::new(SpreadsheetFilterRowsTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_validate" => (
                        Arc::new(SpreadsheetValidateTool),
                        governed(
                            ToolRiskLevel::Low,
                            ToolSideEffect::None,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_write_range" => (
                        Arc::new(SpreadsheetWriteRangeTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_copy_ranges" => (
                        Arc::new(SpreadsheetCopyRangesTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_copy_rows" => (
                        Arc::new(SpreadsheetCopyRowsTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_fill_ranges" => (
                        Arc::new(SpreadsheetFillRangesTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_convert_ranges" => (
                        Arc::new(SpreadsheetConvertRangesTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_export_delimited" => (
                        Arc::new(SpreadsheetExportDelimitedTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_copy_sheet" => (
                        Arc::new(SpreadsheetCopySheetTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_delete_rows" => (
                        Arc::new(SpreadsheetDeleteRowsTool),
                        governed(
                            ToolRiskLevel::High,
                            ToolSideEffect::WorkspaceWrite,
                            ToolApprovalMode::PolicyControlled,
                            DataClassification::Restricted,
                        ),
                    ),
                    "spreadsheet_delete_sheet" => (
                        Arc::new(SpreadsheetDeleteSheetTool),
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
    /// tool replaces an existing record, its static class and governance stay
    /// attached to that registry slot.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name();
        let (class, governance) = self
            .entries
            .get(name)
            .map(|entry| (entry.class, entry.governance.clone()))
            .unwrap_or((ToolClass::Standard, ToolGovernance::unknown()));
        let registration = RegisteredTool::core(tool, class, governance);
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

    pub fn provider_contract_loader(&self, name: &str) -> Option<&str> {
        self.entries
            .get(name)
            .and_then(|entry| entry.tool.provider_contract_loader())
    }

    pub fn is_provider_contract_loader(&self, name: &str) -> bool {
        let Some(loader_source) = self.source(name) else {
            return false;
        };
        if loader_source == ToolSource::Mcp {
            return false;
        }
        self.entries.values().any(|entry| {
            entry.source == loader_source && entry.tool.provider_contract_loader() == Some(name)
        })
    }

    pub(crate) fn is_model_visible(&self, name: &str) -> bool {
        self.entries.contains_key(name)
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
    fn spreadsheet_operations_are_independent_tools() {
        let registry = ToolRegistry::with_builtins();

        assert!(registry.get("spreadsheet").is_none());
        for name in [
            "spreadsheet_inspect",
            "spreadsheet_read_ranges",
            "spreadsheet_copy_ranges",
            "spreadsheet_convert_ranges",
        ] {
            assert!(registry.is_model_visible(name), "{name} should be visible");
            assert_eq!(registry.provider_contract_loader(name), None);
        }
        assert!(registry.get("document_open").is_none());
        assert!(registry.get("document_get_operation_schemas").is_none());
        assert!(registry.get("document_execute").is_none());
        assert!(!registry.is_provider_contract_loader("filesystem"));
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
