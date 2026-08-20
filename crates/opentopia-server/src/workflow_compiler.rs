use crate::agent_connection_access::resolve_agent_template_connection_access;
use opentopia_core::{
    CompiledWorkflowV1, FlowDefinitionV1, GraphNodeKindV1, SessionStore, SqliteSessionStore,
    WorkflowAgentSpecV1,
};

pub(super) fn compile_published_workflow(
    store: &SqliteSessionStore,
    definition: &FlowDefinitionV1,
) -> anyhow::Result<CompiledWorkflowV1> {
    let mut agent_specs = Vec::new();
    for node in definition
        .graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKindV1::Agent)
    {
        let template_id = node
            .config
            .get("reference")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Agent node {} has no template reference", node.id))?;
        let template_version = node
            .config
            .get("templateVersion")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                anyhow::anyhow!("Agent node {} has no valid template version", node.id)
            })?;
        let template = store
            .get_published_agent_template_version(template_id, template_version)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "published Agent template not found for node {}: {}@{}",
                    node.id,
                    template_id,
                    template_version
                )
            })?;
        let resolved = resolve_agent_template_connection_access(store, &template.spec)?
            .require_valid()
            .map_err(|message| {
                anyhow::anyhow!(
                    "Agent node {} Connection access is invalid: {message}",
                    node.id
                )
            })?;
        agent_specs.push(WorkflowAgentSpecV1::compile(
            node.id.clone(),
            &template,
            &resolved,
        )?);
    }
    Ok(CompiledWorkflowV1::compile(definition, agent_specs)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::{
        AgentBudgetV1, AgentModelPolicyV1, AgentRiskClassV1, AgentTemplateSpecV1,
        CapabilityProjection, FlowBudgetV1, FlowDefinitionV1, FlowSourceV1, GraphDefinitionV1,
        GraphNodeV1,
    };
    use serde_json::json;
    use std::collections::BTreeSet;
    use uuid::Uuid;

    #[test]
    fn compiler_pins_a_published_agent_template_without_thread_identity() {
        let store = SqliteSessionStore::open(":memory:").expect("store");
        let draft = store
            .create_agent_template_version(
                "reviewer".to_string(),
                "Reviewer".to_string(),
                "owner".to_string(),
                AgentTemplateSpecV1 {
                    description: String::new(),
                    instructions: "Review the input".to_string(),
                    capabilities: CapabilityProjection::deny_all(),
                    resource_grants: Vec::new(),
                    model_policy: AgentModelPolicyV1::unrestricted(),
                    state_schema: json!({"type": "object"}),
                    output_schema: json!({"type": "object"}),
                    allow_all_delegates: false,
                    delegate_template_ids: BTreeSet::new(),
                    budget: AgentBudgetV1::default(),
                    risk_class: AgentRiskClassV1::Low,
                    connection_bindings: Vec::new(),
                },
            )
            .expect("template draft");
        let (template, _) = store
            .publish_agent_template_version(&draft.template_id, draft.version, &draft.owner, true)
            .expect("publish template");
        let definition = FlowDefinitionV1 {
            schema_version: 1,
            id: Uuid::new_v4(),
            flow_id: "review-flow".to_string(),
            name: "Review flow".to_string(),
            version: 1,
            owner: "owner".to_string(),
            description: String::new(),
            categories: BTreeSet::new(),
            source: FlowSourceV1::NaturalLanguage {
                description: "review".to_string(),
            },
            graph: GraphDefinitionV1 {
                schema_version: 1,
                entry_node_id: "agent".to_string(),
                nodes: vec![
                    GraphNodeV1 {
                        id: "agent".to_string(),
                        label: "Agent".to_string(),
                        kind: GraphNodeKindV1::Agent,
                        config: json!({
                            "reference": template.template_id,
                            "templateVersion": template.version,
                        }),
                        input_schema: json!({"type": "object"}),
                        output_schema: json!({"type": "object"}),
                    },
                    GraphNodeV1 {
                        id: "output".to_string(),
                        label: "Output".to_string(),
                        kind: GraphNodeKindV1::Output,
                        config: json!({}),
                        input_schema: json!({"type": "object"}),
                        output_schema: json!({"type": "object"}),
                    },
                ],
                edges: vec![opentopia_core::GraphEdgeV1 {
                    from: "agent".to_string(),
                    to: "output".to_string(),
                    condition: None,
                    allowed_fields: BTreeSet::new(),
                    data_classification: opentopia_core::DataClassification::Internal,
                    on_error: None,
                    loop_policy: None,
                }],
            },
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            capabilities: CapabilityProjection::deny_all(),
            budget: FlowBudgetV1::default(),
            risk_class: AgentRiskClassV1::Low,
            content_hash: "definition-hash".to_string(),
            published_at: chrono::Utc::now(),
            published_by: "publisher".to_string(),
        };

        let compiled = compile_published_workflow(&store, &definition).expect("compile workflow");
        let agent = compiled.agent_spec("agent").expect("compiled Agent");
        assert_eq!(agent.template_content_hash, template.content_hash);
        assert!(matches!(
            agent.connection_authority,
            opentopia_core::RuntimeConnectionAuthorityV1::Structured { ref operations }
                if operations.is_empty()
        ));
    }
}
