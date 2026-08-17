use super::{
    ApplyPatchTool, BackgroundOutputTool, BrowserTool, ComputerTool, CreateSkillTool, DocumentTool,
    FilesystemTool, FollowupAgentTaskTool, InterruptAgentTool, ListAgentsTool, ListSkillsTool,
    PdfTool, ReadArtifactTool, ReadAttachmentTool, ReadSkillTool, RequestUserInputTool,
    SendAgentMessageTool, SetPlanTool, ShellTool, SpawnAgentTool, SpreadsheetTool, Tool,
    ToolApprovalMode, ToolCapabilityDescriptor, ToolExecutionPolicy, ToolRiskLevel, ToolSideEffect,
    UpdatePlanTool, ViewAttachmentTool, WaitAgentTool,
};
use crate::bundled_plugins::bundled_plugin_catalog;
use crate::enterprise::DataClassification;
use crate::model::ToolCall;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Immutable, clone-cheap catalog of tools available to an agent runtime.
///
/// Composition lives in this module so individual tool implementations remain
/// independent from the product's default tool surface. Runtime policy and
/// capability projection can then reason over one consistent catalog.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<BTreeMap<String, Arc<dyn Tool>>>,
    sources: Arc<BTreeMap<String, ToolSource>>,
    classes: Arc<BTreeMap<String, ToolClass>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    Core,
    BundledPlugin { plugin_name: String },
    Mcp,
}

/// Product-neutral scheduling and visibility class carried by the catalog.
/// The turn driver consumes this metadata instead of recognizing tool names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolClass {
    Standard,
    Subagent,
    StructuredInput,
    WorkForm,
    Flow,
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
        let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        tools.insert("read_attachment".to_string(), Arc::new(ReadAttachmentTool));
        tools.insert("view_attachment".to_string(), Arc::new(ViewAttachmentTool));
        tools.insert("read_artifact".to_string(), Arc::new(ReadArtifactTool));
        tools.insert("filesystem".to_string(), Arc::new(FilesystemTool));
        tools.insert("shell".to_string(), Arc::new(ShellTool));
        tools.insert(
            "background_output".to_string(),
            Arc::new(BackgroundOutputTool),
        );
        tools.insert("apply_patch".to_string(), Arc::new(ApplyPatchTool));
        tools.insert("spawn_agent".to_string(), Arc::new(SpawnAgentTool));
        tools.insert("send_message".to_string(), Arc::new(SendAgentMessageTool));
        tools.insert("followup_task".to_string(), Arc::new(FollowupAgentTaskTool));
        tools.insert("interrupt_agent".to_string(), Arc::new(InterruptAgentTool));
        tools.insert("list_agents".to_string(), Arc::new(ListAgentsTool));
        tools.insert("wait_agent".to_string(), Arc::new(WaitAgentTool));
        tools.insert(
            "request_user_input".to_string(),
            Arc::new(RequestUserInputTool),
        );
        tools.insert("set_plan".to_string(), Arc::new(SetPlanTool));
        tools.insert("update_plan".to_string(), Arc::new(UpdatePlanTool));
        tools.insert("list_skills".to_string(), Arc::new(ListSkillsTool));
        tools.insert("read_skill".to_string(), Arc::new(ReadSkillTool));
        tools.insert("create_skill".to_string(), Arc::new(CreateSkillTool));
        for (name, tool) in crate::flow_tools::flow_tools() {
            tools.insert(name, tool);
        }
        let sources = tools
            .keys()
            .cloned()
            .map(|name| (name, ToolSource::Core))
            .collect();
        let classes = tools
            .keys()
            .cloned()
            .map(|name| {
                let class = core_tool_class(&name);
                (name, class)
            })
            .collect();
        Self {
            tools: Arc::new(tools),
            sources: Arc::new(sources),
            classes: Arc::new(classes),
        }
    }

    fn register_bundled_plugins(&mut self) {
        for plugin in bundled_plugin_catalog() {
            for capability in plugin.native_capabilities {
                let tool: Arc<dyn Tool> = match *capability {
                    "browser" => Arc::new(BrowserTool),
                    "computer" => Arc::new(ComputerTool),
                    "document" => Arc::new(DocumentTool),
                    "pdf" => Arc::new(PdfTool),
                    "spreadsheet" => Arc::new(SpreadsheetTool),
                    _ => continue,
                };
                self.insert_with_source(
                    (*capability).to_string(),
                    tool,
                    ToolSource::BundledPlugin {
                        plugin_name: plugin.name.to_string(),
                    },
                );
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool>) {
        self.insert_with_source(name, tool, ToolSource::Core);
    }

    pub fn insert_mcp(&mut self, name: String, tool: Arc<dyn Tool>) {
        self.insert_with_source(name, tool, ToolSource::Mcp);
    }

    /// Atomically removes the previously synchronized MCP surface while
    /// preserving core and bundled tools. AgentCore calls this before replacing
    /// the enabled server set so stale wrappers cannot remain model-visible.
    pub fn clear_mcp(&mut self) {
        let mcp_names = self
            .sources
            .iter()
            .filter_map(|(name, source)| matches!(source, &ToolSource::Mcp).then(|| name.clone()))
            .collect::<Vec<_>>();
        if mcp_names.is_empty() {
            return;
        }
        let tools = Arc::make_mut(&mut self.tools);
        let sources = Arc::make_mut(&mut self.sources);
        let classes = Arc::make_mut(&mut self.classes);
        for name in mcp_names {
            tools.remove(&name);
            sources.remove(&name);
            classes.remove(&name);
        }
    }

    fn insert_with_source(&mut self, name: String, tool: Arc<dyn Tool>, source: ToolSource) {
        let class = if matches!(source, ToolSource::Core) {
            core_tool_class(&name)
        } else {
            ToolClass::Standard
        };
        Arc::make_mut(&mut self.tools).insert(name.clone(), tool);
        Arc::make_mut(&mut self.sources).insert(name.clone(), source);
        Arc::make_mut(&mut self.classes).insert(name, class);
    }

    pub fn list(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn source(&self, name: &str) -> Option<ToolSource> {
        self.sources.get(name).cloned()
    }

    pub fn class(&self, name: &str) -> Option<ToolClass> {
        self.classes.get(name).copied()
    }

    pub fn execution_policy(&self, name: &str, call: &ToolCall) -> Option<ToolExecutionPolicy> {
        self.tools.get(name).map(|tool| tool.execution_policy(call))
    }

    pub fn capability_catalog(&self) -> Vec<ToolCapabilityDescriptor> {
        self.tools
            .iter()
            .map(|(name, tool)| {
                let source = self.sources.get(name).cloned().unwrap_or(ToolSource::Core);
                let (risk, potential_side_effects, approval, max_data_classification) =
                    tool_governance_metadata(name, &source);
                ToolCapabilityDescriptor {
                    name: name.clone(),
                    description: tool.description().to_string(),
                    input_schema: tool.schema(),
                    source: match &source {
                        ToolSource::Core => "core".to_string(),
                        ToolSource::BundledPlugin { plugin_name } => {
                            format!("bundled_plugin:{plugin_name}")
                        }
                        ToolSource::Mcp => "mcp".to_string(),
                    },
                    risk,
                    potential_side_effects,
                    approval,
                    max_data_classification,
                }
            })
            .collect()
    }
}

fn core_tool_class(name: &str) -> ToolClass {
    match name {
        "spawn_agent" | "send_message" | "followup_task" | "interrupt_agent" | "list_agents"
        | "wait_agent" => ToolClass::Subagent,
        "request_user_input" => ToolClass::StructuredInput,
        "set_plan" | "update_plan" => ToolClass::WorkForm,
        name if name.starts_with("flow_") => ToolClass::Flow,
        _ => ToolClass::Standard,
    }
}

fn tool_governance_metadata(
    name: &str,
    source: &ToolSource,
) -> (
    ToolRiskLevel,
    Vec<ToolSideEffect>,
    ToolApprovalMode,
    DataClassification,
) {
    if matches!(source, ToolSource::Mcp) {
        return (
            ToolRiskLevel::High,
            vec![ToolSideEffect::External],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        );
    }
    match name {
        "read_attachment" | "read_artifact" | "background_output" | "list_agents"
        | "wait_agent" | "list_skills" | "read_skill" | "flow_search" | "flow_inspect"
        | "library_search" | "pdf" => (
            ToolRiskLevel::Low,
            vec![ToolSideEffect::None],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "view_attachment" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::External],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "filesystem" | "apply_patch" | "create_skill" | "spreadsheet" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::WorkspaceWrite],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "shell" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::Process],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "document" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::Process],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "browser" | "computer" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::External],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        ),
        "spawn_agent" | "send_message" | "followup_task" | "interrupt_agent" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::ControlPlane],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        "request_user_input" | "set_plan" | "update_plan" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::SessionMutation],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        "flow_validate" | "flow_simulate" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::SessionMutation],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        "flow_create" | "flow_update" | "flow_publish" | "flow_run" | "flow_pause"
        | "flow_resume" | "flow_cancel" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::ControlPlane],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        "flow_status" => (
            ToolRiskLevel::Low,
            vec![ToolSideEffect::None],
            ToolApprovalMode::Never,
            DataClassification::Confidential,
        ),
        _ => (
            ToolRiskLevel::Unknown,
            vec![ToolSideEffect::Unknown],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        ),
    }
}
