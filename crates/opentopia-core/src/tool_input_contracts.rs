//! Model-facing input DTOs for control-plane tools.
//!
//! Persisted Flow and Agent records retain their storage representation. These
//! DTOs keep the public tool ABI uniformly snake_case and convert into the
//! domain types at the tool boundary.

use crate::enterprise::{
    AgentBudgetV1, AgentModelBindingV1, AgentModelPolicyV1, AgentRiskClassV1, CapabilityProjection,
    DataClassification, ExecutionResourceGrantV1, ResourceKind,
};
use crate::enterprise_connection_grants::{ConnectionBindingV1, OperationGrantV1};
use crate::flow::{
    FlowBudgetV1, FlowSourceV1, FlowSpecV1, GraphDefinitionV1, GraphEdgeV1, GraphLoopPolicyV1,
    GraphNodeKindV1, GraphNodeV1, LoopExhaustionActionV1,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;
use uuid::Uuid;

fn object_schema() -> Value {
    json!({"type": "object"})
}

fn internal_classification() -> DataClassification {
    DataClassification::Internal
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields, default)]
pub(crate) struct ToolCapabilityProjection {
    allow_all_tools: bool,
    tools: BTreeSet<String>,
    allow_all_skills: bool,
    skills: BTreeSet<String>,
    allow_all_plugins: bool,
    plugins: BTreeSet<String>,
    allow_all_mcp_servers: bool,
    mcp_servers: BTreeSet<String>,
    allow_all_workspace_roots: bool,
    workspace_roots: BTreeSet<PathBuf>,
}

impl From<ToolCapabilityProjection> for CapabilityProjection {
    fn from(input: ToolCapabilityProjection) -> Self {
        Self {
            allow_all_tools: input.allow_all_tools,
            tools: input.tools,
            allow_all_skills: input.allow_all_skills,
            skills: input.skills,
            allow_all_plugins: input.allow_all_plugins,
            plugins: input.plugins,
            allow_all_mcp_servers: input.allow_all_mcp_servers,
            mcp_servers: input.mcp_servers,
            allow_all_workspace_roots: input.allow_all_workspace_roots,
            workspace_roots: input.workspace_roots,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolOperationGrant {
    operation_id: String,
}

impl From<ToolOperationGrant> for OperationGrantV1 {
    fn from(input: ToolOperationGrant) -> Self {
        Self {
            operation_id: input.operation_id,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolConnectionBinding {
    connection_id: Uuid,
    capability_revision: u32,
    #[serde(default)]
    operation_grants: Vec<ToolOperationGrant>,
}

impl From<ToolConnectionBinding> for ConnectionBindingV1 {
    fn from(input: ToolConnectionBinding) -> Self {
        Self {
            connection_id: input.connection_id,
            capability_revision: input.capability_revision,
            operation_grants: input.operation_grants.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolExecutionResourceGrant {
    binding_id: String,
    kind: ResourceKind,
    resource: String,
    can_read: bool,
    can_write: bool,
    max_data_classification: DataClassification,
}

impl From<ToolExecutionResourceGrant> for ExecutionResourceGrantV1 {
    fn from(input: ToolExecutionResourceGrant) -> Self {
        Self {
            binding_id: input.binding_id,
            kind: input.kind,
            resource: input.resource,
            can_read: input.can_read,
            can_write: input.can_write,
            max_data_classification: input.max_data_classification,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolAgentModelBinding {
    provider_id: String,
    model_id: String,
}

impl From<ToolAgentModelBinding> for AgentModelBindingV1 {
    fn from(input: ToolAgentModelBinding) -> Self {
        Self {
            provider_id: input.provider_id,
            model_id: input.model_id,
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields, default)]
pub(crate) struct ToolAgentModelPolicy {
    allow_all_models: bool,
    allowed_models: BTreeSet<ToolAgentModelBinding>,
}

impl From<ToolAgentModelPolicy> for AgentModelPolicyV1 {
    fn from(input: ToolAgentModelPolicy) -> Self {
        Self {
            allow_all_models: input.allow_all_models,
            allowed_models: input.allowed_models.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolAgentBudget {
    max_turns: u32,
    max_tool_calls: u32,
    max_duration_seconds: u64,
}

impl From<ToolAgentBudget> for AgentBudgetV1 {
    fn from(input: ToolAgentBudget) -> Self {
        Self {
            max_turns: input.max_turns,
            max_tool_calls: input.max_tool_calls,
            max_duration_seconds: input.max_duration_seconds,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum ToolFlowSource {
    NaturalLanguage { description: String },
    RunTrace { run_id: Uuid, trace_hash: String },
}

impl From<ToolFlowSource> for FlowSourceV1 {
    fn from(input: ToolFlowSource) -> Self {
        match input {
            ToolFlowSource::NaturalLanguage { description } => {
                Self::NaturalLanguage { description }
            }
            ToolFlowSource::RunTrace { run_id, trace_hash } => {
                Self::RunTrace { run_id, trace_hash }
            }
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolGraphNode {
    id: String,
    label: String,
    kind: GraphNodeKindV1,
    #[serde(default)]
    config: Value,
    #[serde(default = "object_schema")]
    input_schema: Value,
    #[serde(default = "object_schema")]
    output_schema: Value,
}

impl From<ToolGraphNode> for GraphNodeV1 {
    fn from(input: ToolGraphNode) -> Self {
        Self {
            id: input.id,
            label: input.label,
            kind: input.kind,
            config: input.config,
            input_schema: input.input_schema,
            output_schema: input.output_schema,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolGraphLoopPolicy {
    max_iterations: u32,
    continue_condition: String,
    on_exhausted: LoopExhaustionActionV1,
}

impl From<ToolGraphLoopPolicy> for GraphLoopPolicyV1 {
    fn from(input: ToolGraphLoopPolicy) -> Self {
        Self {
            max_iterations: input.max_iterations,
            continue_condition: input.continue_condition,
            on_exhausted: input.on_exhausted,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolGraphEdge {
    from: String,
    to: String,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    allowed_fields: BTreeSet<String>,
    #[serde(default = "internal_classification")]
    data_classification: DataClassification,
    #[serde(default)]
    on_error: Option<String>,
    #[serde(default)]
    loop_policy: Option<ToolGraphLoopPolicy>,
}

impl From<ToolGraphEdge> for GraphEdgeV1 {
    fn from(input: ToolGraphEdge) -> Self {
        Self {
            from: input.from,
            to: input.to,
            condition: input.condition,
            allowed_fields: input.allowed_fields,
            data_classification: input.data_classification,
            on_error: input.on_error,
            loop_policy: input.loop_policy.map(Into::into),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolGraphDefinition {
    schema_version: u16,
    entry_node_id: String,
    nodes: Vec<ToolGraphNode>,
    edges: Vec<ToolGraphEdge>,
}

impl From<ToolGraphDefinition> for GraphDefinitionV1 {
    fn from(input: ToolGraphDefinition) -> Self {
        Self {
            schema_version: input.schema_version,
            entry_node_id: input.entry_node_id,
            nodes: input.nodes.into_iter().map(Into::into).collect(),
            edges: input.edges.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolFlowBudget {
    max_node_executions: u32,
    max_tool_calls: u32,
    max_duration_seconds: u64,
    max_loop_iterations: u32,
}

impl Default for ToolFlowBudget {
    fn default() -> Self {
        let defaults = FlowBudgetV1::default();
        Self {
            max_node_executions: defaults.max_node_executions,
            max_tool_calls: defaults.max_tool_calls,
            max_duration_seconds: defaults.max_duration_seconds,
            max_loop_iterations: defaults.max_loop_iterations,
        }
    }
}

impl From<ToolFlowBudget> for FlowBudgetV1 {
    fn from(input: ToolFlowBudget) -> Self {
        Self {
            max_node_executions: input.max_node_executions,
            max_tool_calls: input.max_tool_calls,
            max_duration_seconds: input.max_duration_seconds,
            max_loop_iterations: input.max_loop_iterations,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) struct ToolFlowSpec {
    /// Stable kebab-case Flow identifier.
    flow_id: String,
    name: String,
    description: String,
    owner: String,
    #[serde(default)]
    categories: BTreeSet<String>,
    source: ToolFlowSource,
    #[serde(default = "object_schema")]
    input_schema: Value,
    #[serde(default = "object_schema")]
    output_schema: Value,
    graph: ToolGraphDefinition,
    #[serde(default)]
    requested_capabilities: ToolCapabilityProjection,
    #[serde(default)]
    budget: ToolFlowBudget,
    risk_class: AgentRiskClassV1,
    #[serde(default)]
    pending_decisions: Vec<String>,
}

impl From<ToolFlowSpec> for FlowSpecV1 {
    fn from(input: ToolFlowSpec) -> Self {
        Self {
            flow_id: input.flow_id,
            name: input.name,
            description: input.description,
            owner: input.owner,
            categories: input.categories,
            source: input.source.into(),
            input_schema: input.input_schema,
            output_schema: input.output_schema,
            graph: input.graph.into(),
            requested_capabilities: input.requested_capabilities.into(),
            budget: input.budget.into(),
            risk_class: input.risk_class,
            pending_decisions: input.pending_decisions,
        }
    }
}
