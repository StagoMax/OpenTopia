use super::*;
use std::collections::BTreeSet;

fn document_id(result: &ToolResult) -> Uuid {
    serde_json::from_str::<Value>(&result.output).unwrap()["documentId"]
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("document id")
}

fn selection_id(result: &ToolResult) -> Uuid {
    serde_json::from_str::<Value>(&result.output).unwrap()["selectionId"]
        .as_str()
        .and_then(|value| Uuid::parse_str(value).ok())
        .expect("selection id")
}

fn context(workspace_root: &Path, policy: Arc<BasicPolicyEngine>) -> ToolInvocationContext {
    ToolInvocationContext::local(workspace_root.to_path_buf(), policy)
}

async fn open_document(
    workspace_root: &Path,
    policy: Arc<BasicPolicyEngine>,
    path: &str,
    mode: &str,
) -> Uuid {
    let result = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": path },
                    "mode": mode,
                }),
            ),
            context(workspace_root, policy),
        )
        .await
        .expect("open document");
    document_id(&result)
}

async fn load_operations(
    workspace_root: &Path,
    policy: Arc<BasicPolicyEngine>,
    document_id: Uuid,
    operations: &[&str],
) -> ToolResult {
    DocumentGetOperationSchemasTool
        .execute(
            ToolCall::new(
                "document_get_operation_schemas",
                json!({ "documentId": document_id, "operations": operations }),
            ),
            context(workspace_root, policy),
        )
        .await
        .expect("load document operation schemas")
}

async fn execute_operation(
    workspace_root: &Path,
    policy: Arc<BasicPolicyEngine>,
    document_id: Uuid,
    operation: &str,
    arguments: Value,
) -> ToolResult {
    DocumentExecuteTool
        .execute(
            ToolCall::new(
                "document_execute",
                json!({
                    "documentId": document_id,
                    "operation": operation,
                    "arguments": arguments,
                }),
            ),
            context(workspace_root, policy),
        )
        .await
        .unwrap_or_else(|error| panic!("execute {operation}: {error:#}"))
}

#[tokio::test]
async fn document_protocol_loads_selected_atomic_schemas_and_uses_a_fixed_executor() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-document-protocol-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    let id = open_document(&workspace_root, policy.clone(), "protocol.xlsx", "create").await;
    let described = load_operations(
        &workspace_root,
        policy.clone(),
        id,
        &["write", "write_rows", "read_range"],
    )
    .await;
    let payload: Value = serde_json::from_str(&described.output).unwrap();
    assert_eq!(payload["operations"].as_array().unwrap().len(), 3);
    assert!(payload["operations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|operation| operation["argumentsSchema"].get("oneOf").is_none()));

    let loaded: Vec<crate::provider::ProviderToolContractLoad> = serde_json::from_value(
        described.metadata[crate::provider::PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY].clone(),
    )
    .unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "document_execute");
    assert!(loaded[0].input_schema.get("oneOf").is_none());
    assert_eq!(
        loaded[0].input_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "arguments".to_string(),
            "documentId".to_string(),
            "operation".to_string(),
        ])
    );

    execute_operation(
        &workspace_root,
        policy.clone(),
        id,
        "write",
        json!({
            "sheets": [{
                "name": "Summary",
                "cells": [{
                    "address": { "row": 0, "column": 0 },
                    "value": { "type": "string", "value": "initial" }
                }]
            }]
        }),
    )
    .await;
    execute_operation(
        &workspace_root,
        policy.clone(),
        id,
        "write_rows",
        json!({
            "sheet": "Summary",
            "start": { "row": 1, "column": 0 },
            "rows": [[{ "type": "string", "value": "updated" }]]
        }),
    )
    .await;
    let read = execute_operation(
        &workspace_root,
        policy,
        id,
        "read_range",
        json!({
            "sheet": "Summary",
            "range": {
                "start": { "row": 0, "column": 0 },
                "end": { "row": 1, "column": 0 }
            }
        }),
    )
    .await;
    assert!(read.output.contains("initial"));
    assert!(read.output.contains("updated"));
    assert!(workspace_root.join("protocol.xlsx").is_file());

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn server_side_selection_composes_filter_projection_constants_and_conversions() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-selection-pipeline-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    let source = open_document(&workspace_root, policy.clone(), "source.xlsx", "create").await;
    load_operations(&workspace_root, policy.clone(), source, &["write"]).await;
    execute_operation(
        &workspace_root,
        policy.clone(),
        source,
        "write",
        json!({
            "sheets": [{
                "name": "Orders",
                "cells": [
                    { "address": { "row": 0, "column": 0 }, "value": { "type": "string", "value": "Order ID" } },
                    { "address": { "row": 0, "column": 1 }, "value": { "type": "string", "value": "Created Time" } },
                    { "address": { "row": 0, "column": 2 }, "value": { "type": "string", "value": "Order Amount" } },
                    { "address": { "row": 0, "column": 3 }, "value": { "type": "string", "value": "Status" } },
                    { "address": { "row": 1, "column": 0 }, "value": { "type": "string", "value": "Platform unique order ID." } },
                    { "address": { "row": 1, "column": 1 }, "value": { "type": "string", "value": "Created time description" } },
                    { "address": { "row": 1, "column": 2 }, "value": { "type": "string", "value": "Amount description" } },
                    { "address": { "row": 1, "column": 3 }, "value": { "type": "string", "value": "Status description" } },
                    { "address": { "row": 2, "column": 0 }, "value": { "type": "integer", "value": 335 } },
                    { "address": { "row": 2, "column": 1 }, "value": { "type": "string", "value": "08/19/2026 02:37:00 PM" } },
                    { "address": { "row": 2, "column": 2 }, "value": { "type": "string", "value": "USD 1,234.50" } },
                    { "address": { "row": 2, "column": 3 }, "value": { "type": "string", "value": "paid" } },
                    { "address": { "row": 3, "column": 0 }, "value": { "type": "integer", "value": 336 } },
                    { "address": { "row": 3, "column": 1 }, "value": { "type": "string", "value": "08/19/2026 02:38:00 PM" } },
                    { "address": { "row": 3, "column": 2 }, "value": { "type": "string", "value": "USD 10.00" } },
                    { "address": { "row": 3, "column": 3 }, "value": { "type": "string", "value": "cancelled" } }
                ]
            }]
        }),
    )
    .await;

    let source = open_document(&workspace_root, policy.clone(), "source.xlsx", "read").await;
    load_operations(
        &workspace_root,
        policy.clone(),
        source,
        &[
            "filter_rows",
            "select_columns",
            "convert_column",
            "set_constant_column",
        ],
    )
    .await;
    let filtered = execute_operation(
        &workspace_root,
        policy.clone(),
        source,
        "filter_rows",
        json!({
            "sheet": "Orders",
            "range": {
                "start": { "row": 1, "column": 0 },
                "end": { "row": 3, "column": 3 }
            },
            "conditions": [
                {
                    "column": 0,
                    "operator": "not_equals",
                    "value": { "type": "string", "value": "Platform unique order ID." }
                },
                {
                    "column": 3,
                    "operator": "not_contains",
                    "value": { "type": "string", "value": "cancel" }
                }
            ]
        }),
    )
    .await;
    let filtered_value: Value = serde_json::from_str(&filtered.output).unwrap();
    assert_eq!(filtered_value["rowCount"], 1);
    assert!(filtered.output.len() < 4000);

    let projected = execute_operation(
        &workspace_root,
        policy.clone(),
        source,
        "select_columns",
        json!({
            "selectionId": selection_id(&filtered),
            "columns": [0, 1, 2, 2]
        }),
    )
    .await;
    let mut selection = selection_id(&projected);
    for (column, transform) in [
        (0, json!({ "type": "as_string" })),
        (
            1,
            json!({ "type": "parse_date_time", "format": "%m/%d/%Y %I:%M:%S %p" }),
        ),
        (2, json!({ "type": "extract_currency_code" })),
        (3, json!({ "type": "parse_number", "extract": true })),
    ] {
        let converted = execute_operation(
            &workspace_root,
            policy.clone(),
            source,
            "convert_column",
            json!({ "selectionId": selection, "column": column, "transform": transform }),
        )
        .await;
        selection = selection_id(&converted);
    }
    for value in ["www.tiktokshop.com", "J&T", "tiktokshop"] {
        let constant = execute_operation(
            &workspace_root,
            policy.clone(),
            source,
            "set_constant_column",
            json!({
                "selectionId": selection,
                "column": 4,
                "value": { "type": "string", "value": value },
                "mode": "insert"
            }),
        )
        .await;
        selection = selection_id(&constant);
    }

    let target = open_document(&workspace_root, policy.clone(), "target.xlsx", "create").await;
    load_operations(
        &workspace_root,
        policy.clone(),
        target,
        &["write", "write_selection", "read_range"],
    )
    .await;
    execute_operation(
        &workspace_root,
        policy.clone(),
        target,
        "write",
        json!({
            "sheets": [{
                "name": "Details",
                "cells": [{
                    "address": { "row": 0, "column": 0 },
                    "value": { "type": "string", "value": "Order ID" }
                }]
            }]
        }),
    )
    .await;
    let written = execute_operation(
        &workspace_root,
        policy.clone(),
        target,
        "write_selection",
        json!({
            "selectionId": selection,
            "sheet": "Details",
            "start": { "row": 1, "column": 0 }
        }),
    )
    .await;
    assert_eq!(written.metadata["success"], true, "{}", written.output);
    assert_eq!(written.metadata["rowsWritten"], 1);
    let read = execute_operation(
        &workspace_root,
        policy,
        target,
        "read_range",
        json!({
            "sheet": "Details",
            "range": {
                "start": { "row": 1, "column": 0 },
                "end": { "row": 1, "column": 6 }
            }
        }),
    )
    .await;
    assert!(read.output.contains("335"), "{}", read.output);
    assert!(read.output.contains("USD"));
    assert!(read.output.contains("1234.5"));
    assert!(read.output.contains("J&T"));
    assert!(!read.output.contains("Created time description"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn filtered_selections_keep_all_1166_rows_but_return_only_a_small_preview() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-selection-limit-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    let source = open_document(&workspace_root, policy.clone(), "orders.xlsx", "create").await;
    load_operations(&workspace_root, policy.clone(), source, &["write_rows"]).await;
    let mut rows = vec![json!([{ "type": "string", "value": "Order ID" }])];
    rows.extend((0..1166).map(|row| json!([{ "type": "integer", "value": row + 1 }])));
    execute_operation(
        &workspace_root,
        policy.clone(),
        source,
        "write_rows",
        json!({
            "sheet": "Orders",
            "start": { "row": 0, "column": 0 },
            "rows": rows
        }),
    )
    .await;

    let source = open_document(&workspace_root, policy.clone(), "orders.xlsx", "read").await;
    load_operations(&workspace_root, policy.clone(), source, &["filter_rows"]).await;
    let filtered = execute_operation(
        &workspace_root,
        policy,
        source,
        "filter_rows",
        json!({
            "sheet": "Orders",
            "range": {
                "start": { "row": 1, "column": 0 },
                "end": { "row": 1166, "column": 0 }
            },
            "conditions": [{ "column": 0, "operator": "is_not_blank" }]
        }),
    )
    .await;
    let payload: Value = serde_json::from_str(&filtered.output).unwrap();
    assert_eq!(payload["rowCount"], 1166);
    assert_eq!(payload["previewRows"].as_array().unwrap().len(), 5);
    assert_eq!(payload["previewTruncated"], true);
    assert_eq!(payload["truncated"], false);
    assert!(filtered.output.len() < 4_000);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn immutable_attachments_are_rejected_before_an_edit_session_is_created() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-only-document-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let error = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "attachment", "attachmentId": Uuid::new_v4() },
                    "mode": "edit"
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect_err("immutable attachments must not produce edit handles");
    assert!(error.to_string().contains("attachments are immutable"));
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_only_sandboxes_reject_edit_sessions_in_the_document_layer() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-only-sandbox-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        crate::sandbox::LocalSandboxConfig::disabled()
            .with_sandbox_mode(crate::sandbox::SandboxMode::ReadOnly),
    );
    let error = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": "target.xlsx" },
                    "mode": "edit"
                }),
            ),
            context,
        )
        .await
        .expect_err("read-only sandboxes must not produce edit handles");
    assert!(error.to_string().contains("read-only sandbox"));
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn read_only_files_can_be_read_but_cannot_produce_edit_sessions() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-only-file-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    let created = open_document(&workspace_root, policy.clone(), "locked.xlsx", "create").await;
    load_operations(&workspace_root, policy.clone(), created, &["write"]).await;
    execute_operation(
        &workspace_root,
        policy.clone(),
        created,
        "write",
        json!({
            "sheets": [{
                "name": "Sheet1",
                "cells": [{
                    "address": { "row": 0, "column": 0 },
                    "value": { "type": "string", "value": "readable" }
                }]
            }]
        }),
    )
    .await;

    let path = workspace_root.join("locked.xlsx");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();

    let readable = open_document(&workspace_root, policy.clone(), "locked.xlsx", "read").await;
    load_operations(&workspace_root, policy.clone(), readable, &["read_range"]).await;
    let read = execute_operation(
        &workspace_root,
        policy.clone(),
        readable,
        "read_range",
        json!({
            "sheet": "Sheet1",
            "range": {
                "start": { "row": 0, "column": 0 },
                "end": { "row": 0, "column": 0 }
            }
        }),
    )
    .await;
    assert!(read.output.contains("readable"));

    let error = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": "locked.xlsx" },
                    "mode": "edit"
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect_err("a read-only file must not produce an edit handle");
    assert!(error.to_string().contains("read-only"));

    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn absolute_paths_outside_the_workspace_are_left_to_the_active_policy() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-document-workspace-{}", Uuid::new_v4()));
    let external_root =
        std::env::temp_dir().join(format!("opentopia-document-external-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&external_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let external_path = external_root.join("outside.xlsx");
    let external = external_path.to_string_lossy().into_owned();

    let unrestricted_context = || {
        ToolInvocationContext::local_with_sandbox_config(
            workspace_root.clone(),
            policy.clone(),
            crate::sandbox::LocalSandboxConfig::danger_full_access(),
        )
    };
    let opened = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": external },
                    "mode": "create"
                }),
            ),
            unrestricted_context(),
        )
        .await
        .expect("open external document");
    let document = document_id(&opened);
    DocumentGetOperationSchemasTool
        .execute(
            ToolCall::new(
                "document_get_operation_schemas",
                json!({ "documentId": document, "operations": ["write"] }),
            ),
            unrestricted_context(),
        )
        .await
        .expect("load external write schema");
    let written = DocumentExecuteTool
        .execute(
            ToolCall::new(
                "document_execute",
                json!({
                    "documentId": document,
                    "operation": "write",
                    "arguments": {
                        "sheets": [{
                            "name": "Sheet1",
                            "cells": [{
                                "address": { "row": 0, "column": 0 },
                                "value": { "type": "string", "value": "outside workspace" }
                            }]
                        }]
                    }
                }),
            ),
            unrestricted_context(),
        )
        .await
        .expect("write external document");
    assert_eq!(written.metadata["success"], true, "{}", written.output);
    assert!(external_path.is_file());

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(external_root).unwrap();
}
