//! Immutable workflow compilation and deployment snapshots.
//!
//! A published Flow definition describes graph intent. This module freezes the
//! executable identity of every Agent node so a Run never re-resolves mutable
//! templates or inherits the account identity of its root Flow session.

use crate::enterprise::{
    capabilities_with_connection_operations, AgentModelPolicyV1, AgentRiskClassV1,
    AgentTemplateStatusV1, AgentTemplateVersionV1, CapabilityProjection, ExecutionResourceGrantV1,
    ENTERPRISE_SCHEMA_VERSION_V1,
};
use crate::enterprise_connection_grants::{
    resolved_bindings_match, ConnectionBindingV1, ExecutionConnectionOperationV1,
    ResolvedConnectionBindingV1,
};
use crate::flow::{
    compile_flow, FlowBudgetV1, FlowDefinitionV1, GraphDefinitionV1, GraphNodeKindV1,
};
use crate::model_context::content_fingerprint;
use crate::RuntimeConnectionAuthorityV1;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

pub use crate::workflow_automation::{WorkflowOutputSpecV1, WorkflowTriggerSpecV1};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentSpecV1 {
    pub node_id: String,
    pub template_id: String,
    pub template_version: u32,
    pub template_content_hash: String,
    pub name: String,
    pub owner: String,
    pub instructions: String,
    pub capabilities: CapabilityProjection,
    pub resource_grants: Vec<ExecutionResourceGrantV1>,
    pub model_policy: AgentModelPolicyV1,
    pub state_schema: Value,
    pub output_schema: Value,
    pub risk_class: AgentRiskClassV1,
    pub connection_bindings: Vec<ConnectionBindingV1>,
    pub connection_authority: RuntimeConnectionAuthorityV1,
}

impl WorkflowAgentSpecV1 {
    pub fn compile(
        node_id: impl Into<String>,
        template: &AgentTemplateVersionV1,
        resolved_connection_bindings: &[ResolvedConnectionBindingV1],
    ) -> Result<Self, WorkflowCompileError> {
        let node_id = node_id.into();
        template
            .validate()
            .map_err(|error| WorkflowCompileError::InvalidAgentTemplate {
                node_id: node_id.clone(),
                message: error.to_string(),
            })?;
        if template.status != AgentTemplateStatusV1::Published {
            return Err(WorkflowCompileError::AgentTemplateNotPublished {
                node_id,
                template_id: template.template_id.clone(),
                template_version: template.version,
            });
        }
        if template.content_hash != template.calculate_content_hash() {
            return Err(WorkflowCompileError::AgentTemplateContentHashMismatch {
                node_id,
                template_id: template.template_id.clone(),
                template_version: template.version,
            });
        }
        if template.spec.connection_bindings.is_empty()
            && (template.spec.capabilities.allow_all_mcp_servers
                || !template.spec.capabilities.mcp_servers.is_empty())
        {
            return Err(WorkflowCompileError::LegacyMcpAgentTemplate {
                node_id,
                template_id: template.template_id.clone(),
                template_version: template.version,
            });
        }
        if !resolved_bindings_match(
            &template.spec.connection_bindings,
            resolved_connection_bindings,
        ) {
            return Err(WorkflowCompileError::InvalidResolvedConnectionBindings {
                node_id,
                template_id: template.template_id.clone(),
                template_version: template.version,
            });
        }

        let mut operations = resolved_connection_bindings
            .iter()
            .flat_map(|binding| binding.operations.values().cloned())
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            (left.connection_id, left.operation_id.as_str())
                .cmp(&(right.connection_id, right.operation_id.as_str()))
        });
        let capabilities =
            capabilities_with_connection_operations(&template.spec.capabilities, &operations);

        Ok(Self {
            node_id,
            template_id: template.template_id.clone(),
            template_version: template.version,
            template_content_hash: template.content_hash.clone(),
            name: template.name.clone(),
            owner: template.owner.clone(),
            instructions: template.spec.instructions.clone(),
            capabilities,
            resource_grants: template.spec.resource_grants.clone(),
            model_policy: template.spec.model_policy.clone(),
            state_schema: template.spec.state_schema.clone(),
            output_schema: template.spec.output_schema.clone(),
            risk_class: template.spec.risk_class,
            connection_bindings: template.spec.connection_bindings.clone(),
            // Structured(empty) is intentional: a node with no Connection
            // access must not fall back to mutable Thread MCP state.
            connection_authority: RuntimeConnectionAuthorityV1::Structured { operations },
        })
    }

    pub fn operations(&self) -> &[ExecutionConnectionOperationV1] {
        match &self.connection_authority {
            RuntimeConnectionAuthorityV1::Structured { operations } => operations,
            RuntimeConnectionAuthorityV1::DenyAll | RuntimeConnectionAuthorityV1::LegacyMcp => &[],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledWorkflowV1 {
    pub schema_version: u16,
    pub definition_id: Uuid,
    pub flow_id: String,
    pub flow_version: u32,
    pub definition_content_hash: String,
    pub graph: GraphDefinitionV1,
    pub input_schema: Value,
    pub output_schema: Value,
    pub root_capabilities: CapabilityProjection,
    pub harness_capabilities: CapabilityProjection,
    pub harness_connection_authority: RuntimeConnectionAuthorityV1,
    pub budget: FlowBudgetV1,
    pub agent_specs: BTreeMap<String, WorkflowAgentSpecV1>,
    pub content_hash: String,
}

impl CompiledWorkflowV1 {
    pub fn compile(
        definition: &FlowDefinitionV1,
        agent_specs: Vec<WorkflowAgentSpecV1>,
    ) -> Result<Self, WorkflowCompileError> {
        if let Err(report) = compile_flow(&definition.to_spec(), &definition.capabilities) {
            return Err(WorkflowCompileError::InvalidFlowDefinition {
                codes: report.issues.into_iter().map(|issue| issue.code).collect(),
            });
        }

        let graph_agents = definition
            .graph
            .nodes
            .iter()
            .filter(|node| node.kind == GraphNodeKindV1::Agent)
            .map(|node| (node.id.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        let mut compiled_agents = BTreeMap::new();
        for agent_spec in agent_specs {
            let node = graph_agents
                .get(agent_spec.node_id.as_str())
                .ok_or_else(|| {
                    WorkflowCompileError::UnexpectedAgentSpec(agent_spec.node_id.clone())
                })?;
            let template_id = node
                .config
                .get("reference")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let template_version = node
                .config
                .get("templateVersion")
                .and_then(Value::as_u64)
                .and_then(|version| u32::try_from(version).ok())
                .unwrap_or_default();
            if agent_spec.template_id != template_id
                || agent_spec.template_version != template_version
            {
                return Err(WorkflowCompileError::AgentSpecIdentityMismatch {
                    node_id: agent_spec.node_id,
                });
            }
            let node_id = agent_spec.node_id.clone();
            if compiled_agents
                .insert(node_id.clone(), agent_spec)
                .is_some()
            {
                return Err(WorkflowCompileError::DuplicateAgentSpec(node_id));
            }
        }
        for node_id in graph_agents.keys() {
            if !compiled_agents.contains_key(*node_id) {
                return Err(WorkflowCompileError::MissingAgentSpec(
                    (*node_id).to_string(),
                ));
            }
        }

        let mut harness_capabilities = definition.capabilities.clone();
        let mut operations_by_alias = BTreeMap::<String, ExecutionConnectionOperationV1>::new();
        for agent_spec in compiled_agents.values() {
            harness_capabilities = harness_capabilities.union(&agent_spec.capabilities);
            for operation in agent_spec.operations() {
                if let Some(existing) = operations_by_alias.get(&operation.model_tool_name) {
                    if existing != operation {
                        return Err(WorkflowCompileError::OperationAliasConflict {
                            model_tool_name: operation.model_tool_name.clone(),
                        });
                    }
                    continue;
                }
                operations_by_alias.insert(operation.model_tool_name.clone(), operation.clone());
            }
        }
        let harness_connection_authority = RuntimeConnectionAuthorityV1::Structured {
            operations: operations_by_alias.into_values().collect(),
        };
        let content_hash = compiled_workflow_hash(
            definition,
            &compiled_agents,
            &harness_capabilities,
            &harness_connection_authority,
        );
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            definition_id: definition.id,
            flow_id: definition.flow_id.clone(),
            flow_version: definition.version,
            definition_content_hash: definition.content_hash.clone(),
            graph: definition.graph.clone(),
            input_schema: definition.input_schema.clone(),
            output_schema: definition.output_schema.clone(),
            root_capabilities: definition.capabilities.clone(),
            harness_capabilities,
            harness_connection_authority,
            budget: definition.budget.clone(),
            agent_specs: compiled_agents,
            content_hash,
        })
    }

    pub fn agent_spec(&self, node_id: &str) -> Option<&WorkflowAgentSpecV1> {
        self.agent_specs.get(node_id)
    }

    pub fn template_dependencies(&self) -> BTreeSet<(String, u32, String)> {
        self.agent_specs
            .values()
            .map(|spec| {
                (
                    spec.template_id.clone(),
                    spec.template_version,
                    spec.template_content_hash.clone(),
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentSnapshotV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub compiled_workflow: CompiledWorkflowV1,
    pub trigger: WorkflowTriggerSpecV1,
    pub output: WorkflowOutputSpecV1,
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

impl DeploymentSnapshotV1 {
    pub fn new(compiled_workflow: CompiledWorkflowV1, created_by: impl Into<String>) -> Self {
        Self::new_with_io(
            compiled_workflow,
            WorkflowTriggerSpecV1::Manual,
            WorkflowOutputSpecV1::Inbox,
            created_by,
        )
    }

    pub fn new_with_io(
        compiled_workflow: CompiledWorkflowV1,
        trigger: WorkflowTriggerSpecV1,
        output: WorkflowOutputSpecV1,
        created_by: impl Into<String>,
    ) -> Self {
        let created_by = created_by.into();
        let id = Uuid::new_v4();
        let created_at = Utc::now();
        let bytes = serde_json::to_vec(&(id, &compiled_workflow, &trigger, &output, &created_by))
            .unwrap_or_default();
        Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id,
            compiled_workflow,
            trigger,
            output,
            content_hash: content_fingerprint(&bytes),
            created_at,
            created_by,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDeploymentStatusV1 {
    Active,
    Disabled,
}

impl WorkflowDeploymentStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDeploymentV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub revision: u32,
    pub name: String,
    pub environment: String,
    pub status: WorkflowDeploymentStatusV1,
    pub snapshot: DeploymentSnapshotV1,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
}

impl WorkflowDeploymentV1 {
    pub fn new(
        name: impl Into<String>,
        environment: impl Into<String>,
        compiled_workflow: CompiledWorkflowV1,
        created_by: impl Into<String>,
    ) -> Result<Self, WorkflowCompileError> {
        Self::new_with_io(
            name,
            environment,
            compiled_workflow,
            WorkflowTriggerSpecV1::Manual,
            WorkflowOutputSpecV1::Inbox,
            created_by,
        )
    }

    pub fn new_with_io(
        name: impl Into<String>,
        environment: impl Into<String>,
        compiled_workflow: CompiledWorkflowV1,
        trigger: WorkflowTriggerSpecV1,
        output: WorkflowOutputSpecV1,
        created_by: impl Into<String>,
    ) -> Result<Self, WorkflowCompileError> {
        let name = name.into().trim().to_string();
        let environment = environment.into().trim().to_string();
        let created_by = created_by.into().trim().to_string();
        if name.is_empty() || environment.is_empty() || created_by.is_empty() {
            return Err(WorkflowCompileError::InvalidDeploymentIdentity);
        }
        let now = Utc::now();
        Ok(Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            revision: 1,
            name,
            environment,
            status: WorkflowDeploymentStatusV1::Active,
            snapshot: DeploymentSnapshotV1::new_with_io(
                compiled_workflow,
                trigger,
                output,
                created_by.clone(),
            ),
            created_at: now,
            updated_at: now,
            created_by,
        })
    }

    pub fn disable(&mut self) {
        self.status = WorkflowDeploymentStatusV1::Disabled;
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowCompileError {
    #[error("Flow definition failed validation: {codes:?}")]
    InvalidFlowDefinition { codes: Vec<String> },
    #[error("invalid Agent template for node {node_id}: {message}")]
    InvalidAgentTemplate { node_id: String, message: String },
    #[error(
        "Agent template is not published for node {node_id}: {template_id}@{template_version}"
    )]
    AgentTemplateNotPublished {
        node_id: String,
        template_id: String,
        template_version: u32,
    },
    #[error(
        "Agent template content hash changed for node {node_id}: {template_id}@{template_version}"
    )]
    AgentTemplateContentHashMismatch {
        node_id: String,
        template_id: String,
        template_version: u32,
    },
    #[error("Agent node {node_id} uses legacy MCP grants; migrate {template_id}@{template_version} to structured Connection operations before deployment")]
    LegacyMcpAgentTemplate {
        node_id: String,
        template_id: String,
        template_version: u32,
    },
    #[error("resolved Connection bindings do not match Agent node {node_id}: {template_id}@{template_version}")]
    InvalidResolvedConnectionBindings {
        node_id: String,
        template_id: String,
        template_version: u32,
    },
    #[error("compiled Agent spec has no matching graph node: {0}")]
    UnexpectedAgentSpec(String),
    #[error("graph Agent node has no compiled Agent spec: {0}")]
    MissingAgentSpec(String),
    #[error("duplicate compiled Agent spec: {0}")]
    DuplicateAgentSpec(String),
    #[error("compiled Agent identity does not match graph node {node_id}")]
    AgentSpecIdentityMismatch { node_id: String },
    #[error("Connection operation alias resolves to different routes: {model_tool_name}")]
    OperationAliasConflict { model_tool_name: String },
    #[error("deployment name, environment, and createdBy are required")]
    InvalidDeploymentIdentity,
}

fn compiled_workflow_hash(
    definition: &FlowDefinitionV1,
    agent_specs: &BTreeMap<String, WorkflowAgentSpecV1>,
    harness_capabilities: &CapabilityProjection,
    harness_connection_authority: &RuntimeConnectionAuthorityV1,
) -> String {
    let bytes = serde_json::to_vec(&(
        definition.id,
        &definition.content_hash,
        agent_specs,
        harness_capabilities,
        harness_connection_authority,
    ))
    .unwrap_or_default();
    content_fingerprint(&bytes)
}

#[cfg(test)]
#[path = "workflow/tests.rs"]
mod tests;
