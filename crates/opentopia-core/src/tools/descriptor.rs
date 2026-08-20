use super::{Tool, ToolSideEffect};
use crate::enterprise::DataClassification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

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
    Agent,
    StructuredInput,
    WorkForm,
    Flow,
}

/// Whether a registered implementation belongs in the model-facing catalog.
///
/// Internal tools remain executable so persisted calls and compatibility
/// adapters can reuse their implementation, but no disclosure policy may add
/// their schema to a provider request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolModelVisibility {
    Visible,
    InternalOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    Low,
    Medium,
    High,
    Critical,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalMode {
    Never,
    PolicyControlled,
    Always,
}

/// Registry metadata is control-plane data. It is intentionally kept out of
/// the provider function schema so governance does not consume model tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCapabilityDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub source: String,
    pub risk: ToolRiskLevel,
    pub potential_side_effects: Vec<ToolSideEffect>,
    pub approval: ToolApprovalMode,
    pub max_data_classification: DataClassification,
}

/// Static governance defaults. Per-call authorization remains driven by
/// ToolExecutionIntent and the policy gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolGovernance {
    pub(crate) risk: ToolRiskLevel,
    pub(crate) potential_side_effects: Vec<ToolSideEffect>,
    pub(crate) approval: ToolApprovalMode,
    pub(crate) max_data_classification: DataClassification,
}

impl ToolGovernance {
    pub(crate) fn new(
        risk: ToolRiskLevel,
        side_effect: ToolSideEffect,
        approval: ToolApprovalMode,
        max_data_classification: DataClassification,
    ) -> Self {
        Self {
            risk,
            potential_side_effects: vec![side_effect],
            approval,
            max_data_classification,
        }
    }

    pub(crate) fn unknown() -> Self {
        Self::new(
            ToolRiskLevel::Unknown,
            ToolSideEffect::Unknown,
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        )
    }
}

/// The single registry record for a tool. Its ID, description, and Schema are
/// always delegated to the implementation, so registration cannot drift from
/// the model-facing contract.
#[derive(Clone)]
pub(crate) struct RegisteredTool {
    pub(crate) tool: Arc<dyn Tool>,
    pub(crate) source: ToolSource,
    pub(crate) class: ToolClass,
    pub(crate) model_visibility: ToolModelVisibility,
    pub(crate) governance: ToolGovernance,
}

impl RegisteredTool {
    pub(crate) fn new(
        tool: Arc<dyn Tool>,
        source: ToolSource,
        class: ToolClass,
        governance: ToolGovernance,
    ) -> Self {
        Self {
            tool,
            source,
            class,
            model_visibility: ToolModelVisibility::Visible,
            governance,
        }
    }

    pub(crate) fn core(tool: Arc<dyn Tool>, class: ToolClass, governance: ToolGovernance) -> Self {
        Self::new(tool, ToolSource::Core, class, governance)
    }

    pub(crate) fn internal_only(mut self) -> Self {
        self.model_visibility = ToolModelVisibility::InternalOnly;
        self
    }

    pub(crate) fn name(&self) -> &str {
        self.tool.name()
    }

    pub(crate) fn capability_descriptor(&self) -> ToolCapabilityDescriptor {
        ToolCapabilityDescriptor {
            name: self.name().to_string(),
            description: self.tool.description().to_string(),
            input_schema: self.tool.schema(),
            source: match &self.source {
                ToolSource::Core => "core".to_string(),
                ToolSource::BundledPlugin { plugin_name } => {
                    format!("bundled_plugin:{plugin_name}")
                }
                ToolSource::Mcp => "mcp".to_string(),
            },
            risk: self.governance.risk,
            potential_side_effects: self.governance.potential_side_effects.clone(),
            approval: self.governance.approval,
            max_data_classification: self.governance.max_data_classification,
        }
    }
}
