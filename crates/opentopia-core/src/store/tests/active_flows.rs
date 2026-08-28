// Covers the only user-facing automation aggregate after the Flow cutover.
#[test]
fn active_flow_store_freezes_cases_and_is_cas_protected() {
    use crate::flow::{FlowBudgetV1, GraphDefinitionV1, GraphNodeKindV1, GraphNodeV1};
    use crate::{
        ActiveFlowStoreError, ActiveFlowV1, CompiledWorkflowV1, FlowCaseStatusV1, FlowCaseV1,
        FlowRunV1, FlowStatusV1, RuntimeConnectionAuthorityV1, WorkflowDeliveryReceiptV1,
        WorkflowEvaluationV1,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    let store = SqliteSessionStore::open(":memory:").expect("open store");
    let thread = store
        .create_thread_with_mode(
            Some("active Flow store".to_string()),
            std::path::PathBuf::from("."),
            crate::model::ExperienceMode::Flow,
        )
        .expect("create Flow thread");
    let compiled = CompiledWorkflowV1 {
        schema_version: 1,
        definition_id: uuid::Uuid::new_v4(),
        flow_id: "review-flow".to_string(),
        flow_version: 1,
        definition_content_hash: "definition".to_string(),
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
        root_capabilities: CapabilityProjection::deny_all(),
        harness_capabilities: CapabilityProjection::deny_all(),
        harness_connection_authority: RuntimeConnectionAuthorityV1::Structured {
            operations: Vec::new(),
        },
        budget: FlowBudgetV1::default(),
        agent_specs: BTreeMap::new(),
        content_hash: "compiled".to_string(),
    };
    let flow = ActiveFlowV1::new("Review Flow", thread.id, compiled, "flow-owner")
        .expect("active Flow");
    let inserted = store.insert_active_flow(&flow).expect("insert active Flow");

    assert_eq!(
        store
            .list_active_flows(Some(FlowStatusV1::Active))
            .expect("list active Flows"),
        vec![inserted.clone()]
    );

    let mut case = FlowCaseV1::accepted(&inserted, "case-42", &json!({"caseId": 42}))
        .expect("accept case");
    store.insert_flow_case(&case).expect("insert case");
    assert_eq!(case.flow_revision_id, inserted.active_revision.id);
    assert_eq!(case.flow_revision, inserted.active_revision);

    let run = FlowRunV1::new_from_flow(&inserted, case.input.clone()).expect("Flow Run");
    store.insert_flow_run(&run).expect("insert Flow Run");
    case.flow_run_id = Some(run.id);
    case.status = FlowCaseStatusV1::Started;
    case.updated_at = chrono::Utc::now();
    store.update_flow_case(&case).expect("start case");

    let receipt = WorkflowDeliveryReceiptV1::pending(
        run.id,
        inserted.active_revision.id,
        "inbox",
    );
    store
        .insert_workflow_delivery_receipt(&receipt)
        .expect("insert receipt");
    let evaluation = WorkflowEvaluationV1::new(
        run.id,
        inserted.active_revision.id,
        "fixture-evaluator",
        0.9,
        true,
        vec!["quality".to_string()],
        None,
    )
    .expect("evaluation");
    store
        .insert_workflow_evaluation(&evaluation)
        .expect("insert evaluation");

    let mut paused = inserted.clone();
    paused.pause();
    let updated = store
        .update_active_flow(&paused, inserted.revision)
        .expect("pause Flow");
    assert_eq!(updated.status, FlowStatusV1::Paused);
    assert_eq!(
        store
            .get_active_flow(&inserted.flow_id)
            .expect("get active Flow"),
        Some(updated.clone())
    );

    let conflict = store
        .update_active_flow(&paused, inserted.revision)
        .expect_err("stale revision must conflict");
    assert!(matches!(
        conflict.downcast_ref::<ActiveFlowStoreError>(),
        Some(ActiveFlowStoreError::RevisionConflict(revision))
            if *revision == updated.revision
    ));
}
