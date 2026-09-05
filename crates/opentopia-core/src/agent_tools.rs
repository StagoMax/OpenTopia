use crate::enterprise::{
    AgentKnowledgeBindingV1, AgentModelPolicyV1, AgentRiskClassV1, AgentTemplateSpecV1,
    CapabilityProjection, DataClassification, KnowledgeLibraryProviderV1,
};
use crate::model::{ExperienceMode, ToolCall, ToolResult};
use crate::tool_input_contracts::{
    ToolAgentBudget, ToolAgentModelPolicy, ToolCapabilityProjection, ToolConnectionBinding,
    ToolExecutionResourceGrant,
};
use crate::tools::{
    derived_tool_input_error, derived_tool_schema, RegisteredTool, Tool, ToolApprovalMode,
    ToolClass, ToolExecutionPolicy, ToolGovernance, ToolInvocationContext, ToolRiskLevel,
    ToolSideEffect,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
enum AgentToolAction {
    Search,
    Create,
}

pub(crate) fn agent_tool_registrations() -> Vec<RegisteredTool> {
    [AgentToolAction::Search, AgentToolAction::Create]
        .into_iter()
        .map(|action| {
            let governance = match action {
                AgentToolAction::Search => ToolGovernance::new(
                    ToolRiskLevel::Low,
                    ToolSideEffect::None,
                    ToolApprovalMode::PolicyControlled,
                    DataClassification::Restricted,
                ),
                AgentToolAction::Create => ToolGovernance::new(
                    ToolRiskLevel::Medium,
                    ToolSideEffect::ControlPlane,
                    ToolApprovalMode::PolicyControlled,
                    DataClassification::Confidential,
                ),
            };
            RegisteredTool::core(Arc::new(AgentTool { action }), ToolClass::Flow, governance)
        })
        .collect()
}

struct AgentTool {
    action: AgentToolAction,
}

impl AgentTool {
    fn store<'a>(
        &self,
        ctx: &'a ToolInvocationContext,
    ) -> anyhow::Result<&'a Arc<dyn crate::SessionStore>> {
        ctx.state
            .as_ref()
            .map(crate::tool_state::ToolStateStore::flow_session_store)
            .ok_or_else(|| anyhow::anyhow!("{} requires a persistent SessionStore", self.name()))
    }

    fn require_flow_thread(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        let thread_id = ctx
            .thread_id
            .ok_or_else(|| anyhow::anyhow!("{} requires an active thread", self.name()))?;
        let thread = self
            .store(ctx)?
            .get_thread(thread_id)?
            .ok_or_else(|| anyhow::anyhow!("active thread not found"))?;
        anyhow::ensure!(
            thread.experience_mode == ExperienceMode::Flow,
            "{} is only available in Flow mode",
            self.name()
        );
        Ok(())
    }

    fn result(call_id: uuid::Uuid, value: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::text(
            call_id,
            serde_json::to_string_pretty(&value)?,
            value,
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct AgentSearchInput {
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct AgentCreateInput {
    /// Stable kebab-case Agent identifier.
    #[schemars(length(min = 1))]
    template_id: String,
    #[schemars(length(min = 1))]
    name: String,
    #[schemars(length(min = 1))]
    owner: String,
    description: String,
    #[schemars(length(min = 1))]
    instructions: String,
    #[serde(default)]
    capabilities: Option<ToolCapabilityProjection>,
    #[serde(default)]
    connection_bindings: Vec<ToolConnectionBinding>,
    #[serde(default)]
    knowledge_namespaces: BTreeSet<String>,
    #[serde(default)]
    knowledge_provider: Option<KnowledgeLibraryProviderV1>,
    #[serde(default)]
    resource_grants: Vec<ToolExecutionResourceGrant>,
    #[serde(default)]
    model_policy: Option<ToolAgentModelPolicy>,
    #[serde(default)]
    state_schema: Option<Value>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    budget: Option<ToolAgentBudget>,
    #[serde(default)]
    risk_class: Option<AgentRiskClassV1>,
}

#[async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        match self.action {
            AgentToolAction::Search => "agent_search",
            AgentToolAction::Create => "agent_create",
        }
    }

    fn description(&self) -> &str {
        match self.action {
            AgentToolAction::Search => "Inspect reusable Agent configurations and the configured Connection catalog before creating a duplicate Agent.",
            AgentToolAction::Create => "Create a draft Agent configuration from the user's natural-language requirements. Agent is the product concept; the persisted immutable template version is an internal compatibility detail. Only request capabilities visible in the current ExecutionContext. Do not publish automatically.",
        }
    }

    fn schema(&self) -> Value {
        match self.action {
            AgentToolAction::Search => derived_tool_schema::<AgentSearchInput>(),
            AgentToolAction::Create => derived_tool_schema::<AgentCreateInput>(),
        }
    }

    fn has_derived_input_schema(&self) -> bool {
        true
    }

    fn input_error(&self, input: &Value) -> Option<String> {
        match self.action {
            AgentToolAction::Search => derived_tool_input_error::<AgentSearchInput>(input),
            AgentToolAction::Create => derived_tool_input_error::<AgentCreateInput>(input),
        }
    }

    fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
        match self.action {
            AgentToolAction::Search => {
                ToolExecutionPolicy::read_only(vec!["agents:catalog".to_string()])
            }
            AgentToolAction::Create => {
                let id = call
                    .input
                    .get("template_id")
                    .and_then(Value::as_str)
                    .unwrap_or("draft")
                    .trim();
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: true,
                    side_effect: ToolSideEffect::ControlPlane,
                    resource_keys: vec![format!("agent:{id}")],
                }
            }
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        self.require_flow_thread(&ctx)?;
        let store = self.store(&ctx)?;
        match self.action {
            AgentToolAction::Search => {
                let input: AgentSearchInput = serde_json::from_value(call.input)?;
                let query = input.query.trim().to_lowercase();
                let agents = store
                    .list_agent_template_versions(false)?
                    .into_iter()
                    .filter(|agent| {
                        query.is_empty()
                            || agent.template_id.to_lowercase().contains(&query)
                            || agent.name.to_lowercase().contains(&query)
                            || agent.spec.description.to_lowercase().contains(&query)
                    })
                    .collect::<Vec<_>>();
                let connections = store.list_connections(None, None)?;
                Self::result(
                    call.id,
                    json!({"agents": agents, "connections": connections}),
                )
            }
            AgentToolAction::Create => {
                let input: AgentCreateInput = serde_json::from_value(call.input)?;
                anyhow::ensure!(!input.template_id.trim().is_empty(), "Agent id is required");
                anyhow::ensure!(!input.name.trim().is_empty(), "Agent name is required");
                anyhow::ensure!(!input.owner.trim().is_empty(), "Agent owner is required");
                anyhow::ensure!(
                    !input.instructions.trim().is_empty(),
                    "Agent instructions are required"
                );
                let requested = input
                    .capabilities
                    .map(Into::into)
                    .unwrap_or_else(CapabilityProjection::deny_all);
                let capabilities = requested.intersect(&ctx.capability_projection);
                let knowledge_provider = input.knowledge_provider.or_else(|| {
                    (!input.knowledge_namespaces.is_empty())
                        .then_some(KnowledgeLibraryProviderV1::Sag)
                });
                let knowledge_binding =
                    knowledge_provider.map(|provider| AgentKnowledgeBindingV1 {
                        provider,
                        namespaces: input.knowledge_namespaces,
                    });
                if knowledge_binding.is_some() {
                    anyhow::ensure!(
                        ctx.capability_projection.allows_tool("library_search"),
                        "the current ExecutionContext does not allow knowledge library access"
                    );
                }
                let spec = AgentTemplateSpecV1 {
                    description: input.description.trim().to_string(),
                    instructions: input.instructions.trim().to_string(),
                    capabilities,
                    resource_grants: input
                        .resource_grants
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    model_policy: input
                        .model_policy
                        .map(Into::into)
                        .unwrap_or_else(AgentModelPolicyV1::deny_all),
                    state_schema: input.state_schema.unwrap_or_else(|| {
                        json!({"type": "object", "properties": {}, "additionalProperties": false})
                    }),
                    output_schema: input
                        .output_schema
                        .unwrap_or_else(|| json!({"type": "object"})),
                    allow_all_delegates: false,
                    delegate_template_ids: BTreeSet::new(),
                    budget: input.budget.map(Into::into).unwrap_or_default(),
                    risk_class: input.risk_class.unwrap_or(AgentRiskClassV1::Medium),
                    connection_bindings: input
                        .connection_bindings
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    knowledge_binding,
                };
                let agent = store.create_agent_template_version(
                    input.template_id.trim().to_string(),
                    input.name.trim().to_string(),
                    input.owner.trim().to_string(),
                    spec,
                )?;
                Self::result(call.id, json!({"agent": agent, "status": "draft"}))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_schema_centers_the_product_agent_language() {
        let tool = AgentTool {
            action: AgentToolAction::Create,
        };
        assert_eq!(tool.name(), "agent_create");
        assert!(tool.description().contains("Agent is the product concept"));
        assert!(tool.schema()["required"]
            .as_array()
            .is_some_and(|required| required.contains(&Value::String("template_id".to_string()))));
    }

    #[test]
    fn create_input_uses_one_strict_snake_case_contract() {
        let tool = AgentTool {
            action: AgentToolAction::Create,
        };
        let input = json!({
            "template_id": "review-agent",
            "name": "Review agent",
            "owner": "engineering",
            "description": "Reviews changes",
            "instructions": "Review the requested changes.",
            "capabilities": { "allow_all_tools": true }
        });
        assert_eq!(tool.input_error(&input), None);

        let mut legacy = input;
        legacy["templateId"] = legacy["template_id"].take();
        assert!(tool.input_error(&legacy).is_some());
    }
}
