#[test]
fn workflow_deployment_store_is_queryable_and_cas_protected() {
    use crate::flow::{
        simulate_flow, validate_flow_spec, FlowBudgetV1, FlowDraftStatusV1, FlowDraftV1,
        FlowSourceV1, FlowSpecV1, GraphDefinitionV1, GraphNodeKindV1, GraphNodeV1,
    };
    use crate::{
        CompiledWorkflowV1, WorkflowDeploymentStatusV1, WorkflowDeploymentStoreError,
        WorkflowDeploymentV1,
    };
    use serde_json::json;

    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread_with_mode(
            Some("deployment store".to_string()),
            PathBuf::from("."),
            crate::model::ExperienceMode::Flow,
        )
        .expect("create Flow thread");
    let capabilities = CapabilityProjection::deny_all();
    let spec = FlowSpecV1 {
        flow_id: "deployable-flow".to_string(),
        name: "Deployable flow".to_string(),
        owner: "owner".to_string(),
        description: String::new(),
        categories: BTreeSet::new(),
        source: FlowSourceV1::NaturalLanguage {
            description: "output input".to_string(),
        },
        graph: GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "output".to_string(),
            nodes: vec![GraphNodeV1 {
                id: "output".to_string(),
                label: "Output".to_string(),
                kind: GraphNodeKindV1::Output,
                config: json!({}),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
            }],
            edges: Vec::new(),
        },
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        requested_capabilities: capabilities.clone(),
        budget: FlowBudgetV1::default(),
        risk_class: AgentRiskClassV1::Low,
        pending_decisions: Vec::new(),
    };
    let mut draft = FlowDraftV1::new(thread.id, spec, &capabilities);
    store.create_flow_draft(&draft).expect("create Flow draft");
    let report = validate_flow_spec(&draft.spec, &capabilities);
    assert!(report.valid);
    draft.last_validation = Some(report);
    draft.status = FlowDraftStatusV1::ReadyToPublish;
    store
        .update_flow_draft(&draft, draft.revision)
        .expect("validate Flow draft");
    let trial = simulate_flow(&draft, json!({}), &capabilities);
    store.insert_flow_trial(&trial).expect("persist Flow trial");
    let definition = store
        .publish_flow_draft(draft.id, "publisher")
        .expect("publish Flow definition");
    let compiled = CompiledWorkflowV1::compile(&definition, Vec::new()).expect("compile workflow");
    let deployment =
        WorkflowDeploymentV1::new("Production", "production", compiled, "release-manager")
            .expect("deployment");
    let inserted = store
        .insert_workflow_deployment(&deployment)
        .expect("insert deployment");

    assert_eq!(
        store
            .list_workflow_deployments(
                Some("deployable-flow"),
                Some(WorkflowDeploymentStatusV1::Active)
            )
            .expect("list active deployment"),
        vec![inserted.clone()]
    );

    let mut disabled = inserted.clone();
    disabled.disable();
    let updated = store
        .update_workflow_deployment(&disabled, inserted.revision)
        .expect("disable deployment");
    assert_eq!(updated.status, WorkflowDeploymentStatusV1::Disabled);
    assert_eq!(
        store
            .get_workflow_deployment(inserted.id)
            .expect("get deployment"),
        Some(updated.clone())
    );

    let conflict = store
        .update_workflow_deployment(&disabled, inserted.revision)
        .expect_err("stale revision must conflict");
    assert!(matches!(
        conflict.downcast_ref::<WorkflowDeploymentStoreError>(),
        Some(WorkflowDeploymentStoreError::RevisionConflict(revision))
            if *revision == updated.revision
    ));
}
