use super::*;

#[test]
fn browser_handoff_classifies_sensitive_page_controls() {
    let node: crate::browser::BrowserNode = serde_json::from_value(json!({
        "nodeRef": Uuid::new_v4().to_string(),
        "role": "button",
        "name": "Place order",
        "tagName": "button",
        "bounds": { "x": 0.0, "y": 0.0, "width": 20.0, "height": 20.0 },
        "href": null,
        "formAction": "/checkout",
        "formMethod": "post",
        "inputType": null,
        "editable": false,
        "requiresUserAction": true,
        "userActionReason": "Please review and complete the payment yourself."
    }))
    .expect("deserialize browser node");

    let handoff =
        browser_handoff_for_node("click", &node, None).expect("sensitive control requires handoff");
    assert_eq!(handoff.action, "click");
    assert!(handoff.reason.contains("payment"));
}

#[test]
fn browser_destinations_are_reduced_to_canonical_policy_hosts() {
    assert_eq!(
        browser_destination_host("https://EXAMPLE.com:8443/path?q=1#section").unwrap(),
        "example.com"
    );
    assert!(browser_destination_host("javascript:alert(1)").is_err());
    assert!(browser_destination_host("https://user:secret@example.com/").is_err());
    assert!(browser_destination_host("/relative/path").is_err());
}

#[test]
fn browser_allowed_domains_feed_the_session_network_grant() {
    let workspace =
        std::env::temp_dir().join(format!("opentopia-browser-grant-{}", Uuid::new_v4()));
    let thread_id = Uuid::new_v4();
    let store = Arc::new(crate::store::SqliteSessionStore::open(":memory:").unwrap());
    store
        .put_plugin_settings(
            "browser-automation",
            &crate::plugin_control::PluginControlScope::thread(thread_id),
            &json!({ "allowedDomains": ["STATIC.Example.COM."] }),
        )
        .unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace.clone(),
        PermissionMode::Auto,
    ));
    let mut context = ToolInvocationContext::local(workspace, policy);
    context.state = Some(ToolStateStore::new(store));
    context.thread_id = Some(thread_id);

    assert_eq!(
        configured_browser_hosts(&context).unwrap(),
        HashSet::from(["static.example.com".to_string()])
    );
}

#[test]
fn preserves_typed_mcp_content_and_structured_content() {
    let parts = mcp_content_parts(
        &[
            json!({ "type": "text", "text": "observed" }),
            json!({
                "type": "image",
                "mimeType": "image/png",
                "data": "iVBORw=="
            }),
            json!({
                "type": "resource",
                "resource": {
                    "uri": "file:///workspace/report.pdf",
                    "mimeType": "application/pdf",
                    "name": "report.pdf",
                    "text": "First page"
                }
            }),
        ],
        Some(&json!({ "count": 1 })),
    );

    assert_eq!(parts[0], ModelContentPart::text("observed"));
    assert_eq!(
        parts[1],
        ModelContentPart::image("image/png", vec![0x89, b'P', b'N', b'G'])
    );
    assert_eq!(
        parts[2],
        ModelContentPart::resource(
            "file:///workspace/report.pdf",
            Some("application/pdf".to_string()),
            Some("report.pdf".to_string()),
        )
    );
    assert_eq!(parts[3], ModelContentPart::text("First page"));
    assert_eq!(parts[4], ModelContentPart::json(json!({ "count": 1 })));
}

#[test]
fn rejects_invalid_mcp_base64_without_losing_the_original_json() {
    assert_eq!(decode_mcp_base64("not-base64"), None);
    let parts = mcp_content_parts(
        &[json!({ "type": "image", "mimeType": "image/png", "data": "bad" })],
        None,
    );
    assert_eq!(
        parts,
        vec![ModelContentPart::json(json!({
            "type": "image",
            "mimeType": "image/png",
            "data": "bad"
        }))]
    );
}

#[tokio::test]
async fn request_user_input_builds_a_valid_plan_decision_request() {
    let workspace_root = std::env::current_dir().unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let mut context = ToolInvocationContext::local(workspace_root, policy);
    context.collaboration_mode = CollaborationMode::Plan;
    let result = RequestUserInputTool
        .execute(
            ToolCall::new(
                "request_user_input",
                json!({
                    "questions": [{
                        "id": "storage",
                        "header": "Storage",
                        "question": "Which persistence strategy should the plan use?",
                        "options": [
                            {
                                "id": "sqlite",
                                "label": "SQLite",
                                "description": "Durable local state with migrations.",
                                "recommended": true
                            },
                            {
                                "id": "memory",
                                "label": "In memory",
                                "description": "Simpler but lost on restart."
                            }
                        ]
                    }]
                }),
            ),
            context,
        )
        .await
        .expect("request user input");

    let request: UserInputRequest =
        serde_json::from_value(result.metadata["userInputRequest"].clone()).unwrap();
    assert_eq!(request.questions.len(), 1);
    assert_eq!(request.questions[0].options.len(), 2);
    assert!(request.questions[0].options[0].recommended);
    assert!(request.questions[0].allow_custom);
}

#[tokio::test]
async fn request_user_input_compact_shape_generates_ids_and_recommendation() {
    let workspace_root = std::env::current_dir().unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let mut context = ToolInvocationContext::local(workspace_root, policy);
    context.collaboration_mode = CollaborationMode::Plan;
    let result = RequestUserInputTool
        .execute(
            ToolCall::new(
                "request_user_input",
                json!({
                    "questions": [{
                        "header": "Storage",
                        "question": "Which persistence strategy should the plan use?",
                        "recommended": 0,
                        "options": [
                            { "label": "SQLite", "description": "Durable local state." },
                            { "label": "Memory", "description": "Process-local state." }
                        ]
                    }]
                }),
            ),
            context,
        )
        .await
        .expect("compact request user input");

    let request: UserInputRequest =
        serde_json::from_value(result.metadata["userInputRequest"].clone()).unwrap();
    assert_eq!(request.questions[0].id, "q1");
    assert_eq!(request.questions[0].options[0].id, "o1");
    assert!(request.questions[0].options[0].recommended);
    assert!(request.questions[0].allow_custom);

    let schema = RequestUserInputTool.schema();
    let encoded = schema.to_string();
    assert!(encoded.contains("recommended"));
    assert!(encoded.contains("allow_custom"));
}

#[test]
fn request_user_input_rejects_non_plan_contexts() {
    let workspace_root = std::env::current_dir().unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    for mode in [CollaborationMode::Default, CollaborationMode::Goal] {
        let mut context = ToolInvocationContext::local(workspace_root.clone(), policy.clone());
        context.collaboration_mode = mode;
        let error =
            <RequestUserInputTool as TypedTool>::validate_context(&RequestUserInputTool, &context)
                .expect_err("non-Plan mode must reject structured user input");
        assert!(error
            .to_string()
            .contains("request_user_input is only available in Plan mode"));
    }
}
