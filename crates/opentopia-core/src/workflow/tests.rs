use super::*;
use crate::enterprise::{AgentBudgetV1, AgentTemplateSpecV1, DataClassification};
use crate::enterprise_connection_grants::{OperationGrantV1, ResolvedConnectionBindingV1};
use crate::flow::{
    definition_from_draft, FlowDraftV1, FlowSourceV1, FlowSpecV1, GraphEdgeV1, GraphNodeV1,
};
use serde_json::json;

fn template_with_operation() -> (AgentTemplateVersionV1, ResolvedConnectionBindingV1) {
    let connection_id = Uuid::new_v4();
    let server_id = Uuid::new_v4();
    let operation_id = format!("connection:{connection_id}:tool:review_lead");
    let binding = ConnectionBindingV1 {
        connection_id,
        capability_revision: 7,
        operation_grants: vec![OperationGrantV1 {
            operation_id: operation_id.clone(),
        }],
    };
    let operation = ExecutionConnectionOperationV1 {
        connection_id,
        capability_revision: 7,
        operation_id: operation_id.clone(),
        mcp_server_id: server_id,
        provider_tool_name: "review_lead".to_string(),
        model_tool_name: "mcp_review_lead_fixed".to_string(),
        pinned_operation_fingerprint: "sha256:fixed".to_string(),
    };
    let mut template = AgentTemplateVersionV1::new_draft(
        "lead-reviewer",
        1,
        "Lead reviewer",
        "sales-owner",
        AgentTemplateSpecV1 {
            description: "Review CRM leads".to_string(),
            instructions: "Review the lead and return JSON.".to_string(),
            capabilities: CapabilityProjection::deny_all(),
            resource_grants: Vec::new(),
            model_policy: AgentModelPolicyV1::unrestricted(),
            state_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            allow_all_delegates: false,
            delegate_template_ids: BTreeSet::new(),
            budget: AgentBudgetV1::default(),
            risk_class: AgentRiskClassV1::Medium,
            connection_bindings: vec![binding.clone()],
            knowledge_binding: None,
        },
    )
    .expect("draft template");
    template.status = AgentTemplateStatusV1::Published;
    template.published_at = Some(Utc::now());
    template.published_by = Some("sales-owner".to_string());
    (
        template,
        ResolvedConnectionBindingV1 {
            binding,
            operations: BTreeMap::from([(operation_id, operation)]),
        },
    )
}

fn definition(template: &AgentTemplateVersionV1) -> FlowDefinitionV1 {
    let graph = GraphDefinitionV1 {
        schema_version: 1,
        entry_node_id: "review".to_string(),
        nodes: vec![
            GraphNodeV1 {
                id: "review".to_string(),
                label: "Review lead".to_string(),
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
        edges: vec![GraphEdgeV1 {
            from: "review".to_string(),
            to: "output".to_string(),
            condition: None,
            allowed_fields: BTreeSet::new(),
            data_classification: DataClassification::Internal,
            on_error: None,
            loop_policy: None,
        }],
    };
    let spec = FlowSpecV1 {
        flow_id: "lead-review".to_string(),
        name: "Lead Review".to_string(),
        description: "Review a CRM lead".to_string(),
        owner: "sales-owner".to_string(),
        categories: BTreeSet::new(),
        source: FlowSourceV1::NaturalLanguage {
            description: "Review leads".to_string(),
        },
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        graph,
        requested_capabilities: CapabilityProjection::deny_all(),
        budget: FlowBudgetV1::default(),
        risk_class: AgentRiskClassV1::Medium,
        pending_decisions: Vec::new(),
    };
    definition_from_draft(
        &FlowDraftV1::new(Uuid::new_v4(), spec, &CapabilityProjection::unrestricted()),
        1,
        "publisher",
    )
}

#[test]
fn compiler_freezes_node_identity_and_operation_union() {
    let (template, binding) = template_with_operation();
    let agent =
        WorkflowAgentSpecV1::compile("review", &template, &[binding]).expect("compile Agent node");
    let compiled = CompiledWorkflowV1::compile(&definition(&template), vec![agent.clone()])
        .expect("compile workflow");

    assert_eq!(
        compiled
            .agent_spec("review")
            .map(|spec| spec.template_content_hash.as_str()),
        Some(template.content_hash.as_str())
    );
    assert!(compiled
        .harness_capabilities
        .allows_tool("mcp_review_lead_fixed"));
    assert!(matches!(
        compiled.harness_connection_authority,
        RuntimeConnectionAuthorityV1::Structured { ref operations }
            if operations == agent.operations()
    ));
}

#[test]
fn flow_revision_is_immutable_and_manual_inbox_scoped() {
    let (template, binding) = template_with_operation();
    let agent =
        WorkflowAgentSpecV1::compile("review", &template, &[binding]).expect("compile Agent node");
    let compiled =
        CompiledWorkflowV1::compile(&definition(&template), vec![agent]).expect("compile workflow");
    let flow = ActiveFlowV1::new(
        "Lead review production",
        Uuid::new_v4(),
        compiled,
        "release-manager",
    )
    .expect("active Flow");
    let restored: ActiveFlowV1 =
        serde_json::from_str(&serde_json::to_string(&flow).expect("serialize Flow"))
            .expect("restore Flow");

    assert_eq!(restored, flow);
    assert_eq!(
        restored.active_revision.trigger,
        WorkflowTriggerSpecV1::Manual
    );
    assert_eq!(restored.active_revision.output, WorkflowOutputSpecV1::Inbox);
}

#[test]
fn flow_revision_freezes_provider_without_binding_a_database() {
    let (template, binding) = template_with_operation();
    let agent =
        WorkflowAgentSpecV1::compile("review", &template, &[binding]).expect("compile Agent node");
    let compiled =
        CompiledWorkflowV1::compile(&definition(&template), vec![agent]).expect("compile workflow");
    let flow = ActiveFlowV1::new_with_runtime_options(
        "Lead review with Graph RAG",
        Uuid::new_v4(),
        compiled,
        WorkflowTriggerSpecV1::Manual,
        crate::WorkflowIngressPolicyV1::RequireReview,
        WorkflowOutputSpecV1::Inbox,
        WorkflowOutputReviewPolicyV1::ExplicitNodesOnly,
        Some(WorkflowLibraryProviderV1::GraphRag),
        "release-manager",
    )
    .expect("active Flow");
    let serialized = serde_json::to_value(&flow).expect("serialize Flow");
    let restored: ActiveFlowV1 = serde_json::from_value(serialized.clone()).expect("restore Flow");

    assert_eq!(
        serialized["activeRevision"]["libraryProvider"],
        json!("graph-rag")
    );
    assert_eq!(
        restored.active_revision.library_provider,
        Some(WorkflowLibraryProviderV1::GraphRag)
    );
}
