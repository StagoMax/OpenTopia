use super::*;

#[tokio::test]
async fn spreadsheet_protocol_progressively_describes_and_executes_offline_workbooks() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-sheet-protocol-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));

    let created = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "operation": "write",
                    "arguments": {
                        "outputPath": "protocol.xlsx",
                        "sheets": [{
                            "name": "Summary",
                            "cells": [{
                                "address": { "row": 0, "column": 0 },
                                "value": { "type": "string", "value": "initial" }
                            }]
                        }]
                    }
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("create workbook through protocol");
    assert_eq!(created.metadata["success"], true);

    let create_contract = SpreadsheetDescribeTool
        .execute(
            ToolCall::new("spreadsheet_describe", json!({ "operations": ["write"] })),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("describe creation without an existing resource");
    let create_contracts: Vec<crate::provider::ProviderToolContractLoad> = serde_json::from_value(
        create_contract.metadata[crate::provider::PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY]
            .clone(),
    )
    .expect("creation contract load");
    assert_eq!(
        crate::provider::tool_input_schema_error(
            &create_contracts[0].input_schema,
            &json!({
                "operation": "write",
                "arguments": {
                    "outputPath": "new.xlsx",
                    "sheets": [{
                        "name": "Summary",
                        "cells": [{
                            "address": { "row": 0, "column": 0 },
                            "value": { "type": "string", "value": "created" }
                        }]
                    }]
                }
            }),
            "arguments"
        ),
        None
    );

    let resource = json!({ "kind": "file", "path": "protocol.xlsx" });
    let inspected = SpreadsheetInspectTool
        .execute(
            ToolCall::new("spreadsheet_inspect", json!({ "resource": resource })),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("inspect protocol workbook");
    assert!(inspected.output.contains("Summary"));

    let described = SpreadsheetDescribeTool
        .execute(
            ToolCall::new(
                "spreadsheet_describe",
                json!({
                    "resource": resource,
                    "operations": ["write_rows", "read_range"]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("describe selected protocol operations");
    assert!(described.output.contains("write_rows"));
    assert!(described.output.contains("argumentsSchema"));

    let loaded_contracts: Vec<crate::provider::ProviderToolContractLoad> = serde_json::from_value(
        described.metadata[crate::provider::PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY].clone(),
    )
    .expect("describe must return a loadable execute contract");
    assert_eq!(loaded_contracts.len(), 1);
    assert_eq!(loaded_contracts[0].name, "spreadsheet_execute");
    let loaded_execute_schema = &loaded_contracts[0].input_schema;
    assert_eq!(
        loaded_execute_schema, &create_contracts[0].input_schema,
        "describe selections must not replace the exposed execute contract"
    );
    assert_eq!(
        crate::provider::tool_input_schema_error(
            loaded_execute_schema,
            &json!({
                "resource": resource,
                "operation": "read_columns",
                "arguments": {
                    "sheet": "Summary",
                    "startRow": 0,
                    "rowCount": 2,
                    "startColumn": 0,
                    "columnCount": 1
                }
            }),
            "arguments"
        ),
        None,
        "a later describe selection must retain the read_columns contract"
    );
    assert!(crate::provider::tool_input_schema_error(
        loaded_execute_schema,
        &json!({
            "resource": resource,
            "operation": "read_range",
            "arguments": {}
        }),
        "arguments"
    )
    .is_some());
    assert!(crate::provider::tool_input_schema_error(
        loaded_execute_schema,
        &json!({
            "resource": null,
            "operation": "read_range",
            "arguments": {
                "sheet": "Summary",
                "range": {
                    "start": { "row": 0, "column": 0 },
                    "end": { "row": 1, "column": 0 }
                }
            }
        }),
        "arguments"
    )
    .is_some());

    let described_value: Value = serde_json::from_str(&described.output).unwrap();
    let operation_schema = |operation: &str| {
        described_value["operations"]
            .as_array()
            .and_then(|operations| {
                operations
                    .iter()
                    .find(|candidate| candidate["operation"] == operation)
            })
            .map(|contract| &contract["argumentsSchema"])
            .unwrap_or_else(|| panic!("missing described contract for {operation}"))
    };
    let write_arguments = json!({
        "outputPath": "protocol.xlsx",
        "sheet": "Summary",
        "start": { "row": 1, "column": 0 },
        "rows": [[{ "type": "string", "value": "updated" }]]
    });
    let read_arguments = json!({
        "sheet": "Summary",
        "range": {
            "start": { "row": 0, "column": 0 },
            "end": { "row": 1, "column": 0 }
        }
    });
    assert_eq!(
        crate::provider::tool_input_schema_error(
            loaded_execute_schema,
            &json!({
                "resource": resource,
                "operation": "read_range",
                "arguments": read_arguments
            }),
            "arguments"
        ),
        None,
        "the loaded provider contract must accept the same payload as execution"
    );
    for (operation, arguments, binding) in [
        ("write_rows", &write_arguments, "sourcePath"),
        ("read_range", &read_arguments, "path"),
    ] {
        let schema = operation_schema(operation);
        assert_eq!(
            crate::provider::tool_input_schema_error(
                schema,
                arguments,
                "spreadsheet_execute.arguments"
            ),
            None,
            "described {operation} schema must accept the execute payload"
        );
        assert!(schema["properties"].get("action").is_none());
        assert!(schema["properties"].get(binding).is_none());

        if operation == "read_range" {
            assert!(schema["properties"].get("attachmentId").is_none());
        }

        let forbidden_fields = if operation == "read_range" {
            vec!["action", binding, "attachmentId"]
        } else {
            vec!["action", binding]
        };
        for forbidden in forbidden_fields {
            let mut invalid = arguments.clone();
            invalid[forbidden] = json!(operation);
            assert!(crate::provider::tool_input_schema_error(
                schema,
                &invalid,
                "spreadsheet_execute.arguments"
            )
            .is_some());
        }
    }
    assert_eq!(
        SpreadsheetExecuteTool.schema()["properties"]["arguments"]["type"],
        "object"
    );

    SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": resource,
                    "operation": "write_rows",
                    "arguments": write_arguments
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("mutate protocol workbook");

    let read = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": resource,
                    "operation": "read_range",
                    "arguments": read_arguments
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("read protocol workbook");
    assert!(read.output.contains("initial"));
    assert!(read.output.contains("updated"));

    let rejected_action = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": resource,
                    "operation": "read_range",
                    "arguments": {
                        "action": "read_range",
                        "sheet": "Summary",
                        "range": {
                            "start": { "row": 0, "column": 0 },
                            "end": { "row": 1, "column": 0 }
                        }
                    }
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect_err("projected contract must reject a model-supplied action");
    assert!(rejected_action.to_string().contains("arguments.action"));

    let inspect_schema = SpreadsheetInspectTool.schema();
    let inspect_schema_text = serde_json::to_string(&inspect_schema).unwrap();
    assert!(inspect_schema_text.contains("\"file\""));
    assert!(!inspect_schema_text.contains("workspaceFile"));
    assert!(inspect_schema_text.contains("attachment"));
    assert!(inspect_schema_text.contains("attachmentId"));
    assert!(!inspect_schema_text.contains("attachment_id"));
    assert!(inspect_schema
        .pointer("/properties/delimitedFormat")
        .is_some());
    assert!(inspect_schema.pointer("/properties/format").is_none());
    assert!(!inspect_schema_text.contains("liveSession"));

    let attachment_id = Uuid::new_v4();
    let mut legacy_resource_key = json!({
        "resource": {
            "kind": "attachment",
            "attachment_id": attachment_id
        }
    });
    let normalizations =
        crate::provider::normalize_tool_argument_keys(&inspect_schema, &mut legacy_resource_key);
    assert_eq!(
        legacy_resource_key["resource"]["attachmentId"],
        json!(attachment_id)
    );
    assert!(legacy_resource_key["resource"]
        .get("attachment_id")
        .is_none());
    assert_eq!(normalizations.len(), 1);
    assert!(SpreadsheetInspectTool
        .input_error(&legacy_resource_key)
        .is_none());

    assert!(SpreadsheetDescribeTool
        .input_error(&json!({
            "resource": resource,
            "operations": []
        }))
        .is_some());
    assert!(SpreadsheetInspectTool
        .input_error(&json!({
            "resource": {
                "kind": "liveSession",
                "sessionId": "session-1",
                "documentId": "workbook-1"
            }
        }))
        .is_some());

    let attachment_resource = json!({
        "kind": "attachment",
        "attachmentId": Uuid::new_v4()
    });
    let attachment_contract = SpreadsheetDescribeTool
        .execute(
            ToolCall::new(
                "spreadsheet_describe",
                json!({
                    "resource": attachment_resource,
                    "operations": ["read_range"]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("describe an attachment observation");
    assert!(attachment_contract.output.contains("offlineAttachment"));

    let attachment_mutation = SpreadsheetDescribeTool
        .execute(
            ToolCall::new(
                "spreadsheet_describe",
                json!({
                    "resource": attachment_resource,
                    "operations": ["write_rows"]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy),
        )
        .await
        .expect("describe an attachment-backed mutation");
    let attachment_mutation: Value = serde_json::from_str(&attachment_mutation.output).unwrap();
    assert_eq!(attachment_mutation["resource"]["kind"], "attachment");
    assert!(attachment_mutation["resource"]
        .get("writeSupported")
        .is_none());
    assert_eq!(
        attachment_mutation["operations"][0]["primaryResourceBinding"],
        "sourcePath"
    );
    assert!(attachment_mutation["operations"][0]["notes"]
        .as_array()
        .is_some_and(|notes| notes.iter().any(|note| note
            .as_str()
            .is_some_and(|note| note.contains("resource is a read input")))));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn spreadsheet_protocol_uses_an_immutable_attachment_as_a_mutation_input() {
    let case_root =
        std::env::temp_dir().join(format!("opentopia-sheet-attachment-{}", Uuid::new_v4()));
    let workspace_root = case_root.join("workspace");
    let attachment_root = case_root.join("selected-files");
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&attachment_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let sandbox = crate::sandbox::LocalSandboxConfig::default();

    SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "operation": "write",
                    "arguments": {
                        "outputPath": "template.xlsx",
                        "sheets": [{
                            "name": "Summary",
                            "cells": [{
                                "address": { "row": 0, "column": 0 },
                                "value": { "type": "string", "value": "template" }
                            }]
                        }]
                    }
                }),
            ),
            ToolInvocationContext::local_with_sandbox_config(
                workspace_root.clone(),
                policy.clone(),
                sandbox.clone(),
            ),
        )
        .await
        .expect("create attachment fixture");
    let attachment_path = attachment_root.join("template.xlsx");
    fs::rename(workspace_root.join("template.xlsx"), &attachment_path).unwrap();
    let attachment_path = attachment_path.canonicalize().unwrap();

    let store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::open(":memory:").expect("open memory store"));
    let thread = store
        .create_thread(
            Some("spreadsheet attachment".to_string()),
            workspace_root.clone(),
        )
        .expect("create thread");
    let attachment_id = Uuid::new_v4();
    let mut message = Message::text(thread.id, MessageRole::User, "update the selected template");
    message.parts.push(MessagePart::SourceRef {
        source: ContextSourceRef {
            id: attachment_id,
            path: attachment_path.clone(),
            name: "template.xlsx".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                .to_string(),
            bytes: fs::metadata(&attachment_path).unwrap().len(),
            truncated: false,
        },
        inline: Some(true),
    });
    store.append_message(message).expect("persist attachment");

    let mut context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy.clone(),
        sandbox.clone(),
    );
    context.state = Some(ToolStateStore::new(store));
    context.thread_id = Some(thread.id);
    let result = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": {
                        "kind": "attachment",
                        "attachmentId": attachment_id
                    },
                    "operation": "write_rows",
                    "arguments": {
                        "outputPath": "attachment-copy.xlsx",
                        "sheet": "Summary",
                        "start": { "row": 1, "column": 0 },
                        "rows": [[{ "type": "string", "value": "written in one mutation" }]]
                    }
                }),
            ),
            context,
        )
        .await
        .expect("write a separate output from the immutable attachment input");
    assert_eq!(result.metadata["success"], true);

    let output = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "read_range",
                    "path": "attachment-copy.xlsx",
                    "sheet": "Summary",
                    "range": {
                        "start": { "row": 0, "column": 0 },
                        "end": { "row": 1, "column": 0 }
                    }
                }),
            ),
            ToolInvocationContext::local_with_sandbox_config(
                workspace_root.clone(),
                policy.clone(),
                sandbox.clone(),
            ),
        )
        .await
        .expect("read generated output");
    assert!(output.output.contains("template"));
    assert!(output.output.contains("written in one mutation"));

    let original = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "read_range",
                    "path": attachment_path.to_string_lossy(),
                    "sheet": "Summary",
                    "range": {
                        "start": { "row": 0, "column": 0 },
                        "end": { "row": 1, "column": 0 }
                    }
                }),
            ),
            ToolInvocationContext::local_with_sandbox_config(workspace_root, policy, sandbox),
        )
        .await
        .expect("read original attachment");
    assert!(original.output.contains("template"));
    assert!(!original.output.contains("written in one mutation"));

    fs::remove_dir_all(case_root).unwrap();
}

#[tokio::test]
async fn transfer_rows_handles_full_scale_filter_mapping_constants_and_conversions() {
    use crate::spreadsheet::{
        execute_spreadsheet, write_workbook, CellAddress, CellUpdate, FillTemplateRequest,
        SheetWriteRequest, SpreadsheetAction, SpreadsheetCellInput, SpreadsheetRequest,
        WriteWorkbookRequest,
    };

    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-transfer-workspace-{}", Uuid::new_v4()));
    let case_root =
        std::env::temp_dir().join(format!("opentopia-transfer-external-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&case_root).unwrap();
    let source_template = case_root.join("source-template.xlsx");
    let source_csv = case_root.join("source.csv");
    let source = case_root.join("source.xlsx");
    let template = case_root.join("template.xlsx");
    let output = case_root.join("output.xlsx");

    let mut source_headers = vec![
        "Order ID".to_string(),
        "Order Status".to_string(),
        "Created Time".to_string(),
        "Order Amount".to_string(),
        "Product Name".to_string(),
        "Quantity".to_string(),
        "SKU Unit Original Price".to_string(),
        "Tracking ID".to_string(),
    ];
    source_headers.extend((source_headers.len()..59).map(|index| format!("Extra {index}")));
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: source_template.clone(),
        sheets: vec![SheetWriteRequest {
            name: "OrderSKUList".to_string(),
            visibility: None,
            cells: source_headers
                .iter()
                .enumerate()
                .map(|(column, header)| CellUpdate {
                    address: CellAddress {
                        row: 0,
                        column: column as u32,
                    },
                    value: SpreadsheetCellInput::String(header.clone()),
                })
                .collect(),
        }],
    })
    .expect("create source template");

    let mut csv = String::new();
    csv.push_str(&source_headers.join(","));
    csv.push('\n');
    let mut description = vec![String::new(); 59];
    description[0] = "Platform unique order ID.".to_string();
    description[1] = "Order status.".to_string();
    description[2] = "Order created time.".to_string();
    csv.push_str(&description.join(","));
    csv.push('\n');
    for index in 0..1_500 {
        let mut row = vec![String::new(); 59];
        row[0] = (index + 1).to_string();
        row[1] = if index < 330 {
            "Cancelled by seller".to_string()
        } else if index < 334 {
            "Unpaid".to_string()
        } else {
            "Paid".to_string()
        };
        row[2] = "08/19/2026 02:37:00 AM".to_string();
        row[3] = "USD 1234.50".to_string();
        row[4] = format!("Product {}", index + 1);
        row[5] = "2".to_string();
        row[6] = "USD 10.25".to_string();
        row[7] = format!("TRACK-{}", index + 1);
        csv.push_str(&row.join(","));
        csv.push('\n');
    }
    fs::write(&source_csv, csv).unwrap();
    execute_spreadsheet(SpreadsheetRequest {
        action: SpreadsheetAction::FillTemplate(FillTemplateRequest {
            source: source_csv,
            source_format: None,
            template: source_template,
            output: source.clone(),
            target_sheet: "OrderSKUList".to_string(),
            source_header_row: 0,
            target_header_row: 0,
            target_start_row: Some(1),
            mappings: Vec::new(),
            rstrip_tabs: false,
        }),
    })
    .expect("materialize full-scale source workbook");

    let target_headers = [
        "Order ID",
        "Created Date",
        "Currency",
        "Amount",
        "Product",
        "Quantity",
        "Unit Price",
        "Website",
        "Tracking",
        "Logistics",
        "Platform",
    ];
    let mut target_cells = vec![CellUpdate {
        address: CellAddress { row: 0, column: 0 },
        value: SpreadsheetCellInput::String("template-version".to_string()),
    }];
    target_cells.extend(
        target_headers
            .iter()
            .enumerate()
            .map(|(column, header)| CellUpdate {
                address: CellAddress {
                    row: 1,
                    column: column as u32,
                },
                value: SpreadsheetCellInput::String((*header).to_string()),
            }),
    );
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: template.clone(),
        sheets: vec![SheetWriteRequest {
            name: "订单明细".to_string(),
            visibility: None,
            cells: target_cells,
        }],
    })
    .expect("create target template");
    let source_before = fs::read(&source).unwrap();
    let template_before = fs::read(&template).unwrap();

    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let sandbox = crate::sandbox::LocalSandboxConfig::danger_full_access();
    let transferred = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": { "kind": "file", "path": source.to_string_lossy() },
                    "operation": "transfer_rows",
                    "arguments": {
                        "sourceSheet": "OrderSKUList",
                        "templatePath": template.to_string_lossy(),
                        "outputPath": output.to_string_lossy(),
                        "targetSheet": "订单明细",
                        "sourceHeaderRow": 0,
                        "sourceStartRow": 1,
                        "targetHeaderRow": 1,
                        "targetStartRow": 2,
                        "filters": [
                            {
                                "source": { "by": "header", "name": "Order ID" },
                                "operator": "not_equals",
                                "value": { "type": "string", "value": "Platform unique order ID." }
                            },
                            {
                                "source": { "by": "header", "name": "Order Status" },
                                "operator": "not_contains",
                                "value": { "type": "string", "value": "cancel" }
                            },
                            {
                                "source": { "by": "header", "name": "Order Status" },
                                "operator": "not_equals",
                                "value": { "type": "string", "value": "unpaid" }
                            }
                        ],
                        "columns": [
                            transfer_source_column("Order ID", "Order ID", json!([{ "type": "as_string" }])),
                            transfer_source_column("Created Time", "Created Date", json!([{
                                "type": "parse_date_time",
                                "format": "%m/%d/%Y %I:%M:%S %p"
                            }])),
                            transfer_source_column("Order Amount", "Currency", json!([{
                                "type": "extract_currency_code"
                            }])),
                            transfer_source_column("Order Amount", "Amount", json!([{
                                "type": "parse_number",
                                "extract": true
                            }])),
                            transfer_source_column("Product Name", "Product", json!([])),
                            transfer_source_column("Quantity", "Quantity", json!([{
                                "type": "parse_number",
                                "extract": false
                            }])),
                            transfer_source_column("SKU Unit Original Price", "Unit Price", json!([{
                                "type": "parse_number",
                                "extract": true
                            }])),
                            transfer_constant_column("Website", "www.tiktokshop.com"),
                            transfer_source_column("Tracking ID", "Tracking", json!([])),
                            transfer_constant_column("Logistics", "J&T"),
                            transfer_constant_column("Platform", "tiktokshop")
                        ]
                    }
                }),
            ),
            ToolInvocationContext::local_with_sandbox_config(
                workspace_root.clone(),
                policy.clone(),
                sandbox.clone(),
            ),
        )
        .await
        .expect("execute full-scale row transfer");
    assert_eq!(
        transferred.metadata["success"], true,
        "{}",
        transferred.output
    );
    assert!(transferred.output.contains("\"rowsWritten\": 1166"));
    assert!(transferred.output.contains("\"cellsWritten\": 12826"));
    assert!(output.is_file());
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(fs::read(&template).unwrap(), template_before);

    for (row, expected_id) in [(2, "335"), (1167, "1500")] {
        let read = SpreadsheetTool
            .execute(
                ToolCall::new(
                    "spreadsheet",
                    json!({
                        "action": "read_range",
                        "path": output.to_string_lossy(),
                        "sheet": "订单明细",
                        "range": {
                            "start": { "row": row, "column": 0 },
                            "end": { "row": row, "column": 10 }
                        }
                    }),
                ),
                ToolInvocationContext::local_with_sandbox_config(
                    workspace_root.clone(),
                    policy.clone(),
                    sandbox.clone(),
                ),
            )
            .await
            .expect("read transferred row");
        assert!(read.output.contains(expected_id));
        assert!(read.output.contains("J&T"));
        assert!(read.output.contains("www.tiktokshop.com"));
        assert!(read.output.contains("1234.5"));
    }

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(case_root).unwrap();
}

fn transfer_source_column(source: &str, target: &str, transforms: Value) -> Value {
    json!({
        "target": { "by": "header", "name": target },
        "value": {
            "kind": "source",
            "source": { "by": "header", "name": source },
            "transforms": transforms
        }
    })
}

fn transfer_constant_column(target: &str, value: &str) -> Value {
    json!({
        "target": { "by": "header", "name": target },
        "value": {
            "kind": "constant",
            "value": { "type": "string", "value": value }
        }
    })
}

/// Manual regression for the two external workbooks from the original failure.
/// The paths stay outside source control and are supplied only when this ignored
/// test is explicitly requested.
#[tokio::test]
#[ignore = "requires OPENTOPIA_PRIOR_CASE_SOURCE and OPENTOPIA_PRIOR_CASE_TEMPLATE"]
async fn prior_two_workbook_case_runs_as_one_native_transfer() {
    let source_path = PathBuf::from(
        std::env::var("OPENTOPIA_PRIOR_CASE_SOURCE").expect("source workbook environment variable"),
    )
    .canonicalize()
    .expect("canonicalize source workbook");
    let template_path = PathBuf::from(
        std::env::var("OPENTOPIA_PRIOR_CASE_TEMPLATE")
            .expect("template workbook environment variable"),
    )
    .canonicalize()
    .expect("canonicalize template workbook");
    let source_before = fs::read(&source_path).expect("snapshot source workbook");
    let template_before = fs::read(&template_path).expect("snapshot template workbook");

    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-prior-sheet-case-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let sandbox = crate::sandbox::LocalSandboxConfig::danger_full_access();
    let store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::open(":memory:").expect("open memory store"));
    let thread = store
        .create_thread(
            Some("prior spreadsheet case".to_string()),
            workspace_root.clone(),
        )
        .expect("create thread");
    let source_id = Uuid::new_v4();
    let template_id = Uuid::new_v4();
    let mut message = Message::text(
        thread.id,
        MessageRole::User,
        "fill the order template from the selected source and use J&T",
    );
    for (id, path, name) in [
        (source_id, source_path.clone(), "Todo pedido.xlsx"),
        (template_id, template_path.clone(), "订单模板.xlsx"),
    ] {
        message.parts.push(MessagePart::SourceRef {
            source: ContextSourceRef {
                id,
                bytes: fs::metadata(&path).unwrap().len(),
                path,
                name: name.to_string(),
                kind: ContextSourceKind::Document,
                content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                    .to_string(),
                truncated: false,
            },
            inline: Some(true),
        });
    }
    store
        .append_message(message)
        .expect("persist selected files");

    let mut context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy.clone(),
        sandbox.clone(),
    );
    context.state = Some(ToolStateStore::new(store));
    context.thread_id = Some(thread.id);
    let result = SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": {
                        "kind": "attachment",
                        "attachmentId": source_id
                    },
                    "operation": "transfer_rows",
                    "arguments": {
                        "sourceSheet": "OrderSKUList",
                        "templatePath": template_path.to_string_lossy(),
                        "outputPath": "prior-case-regression.xlsx",
                        "targetSheet": "订单明细",
                        "sourceHeaderRow": 0,
                        "sourceStartRow": 1,
                        "targetHeaderRow": 1,
                        "targetStartRow": 2,
                        "filters": [
                            {
                                "source": { "by": "header", "name": "Order ID" },
                                "operator": "not_equals",
                                "value": { "type": "string", "value": "Platform unique order ID." }
                            },
                            {
                                "source": { "by": "header", "name": "Order Status" },
                                "operator": "not_contains",
                                "value": { "type": "string", "value": "cancel" }
                            },
                            {
                                "source": { "by": "header", "name": "Order Status" },
                                "operator": "not_equals",
                                "value": { "type": "string", "value": "no pagado" }
                            }
                        ],
                        "columns": [
                            transfer_source_column("Order ID", "订单号", json!([{ "type": "as_string" }])),
                            transfer_source_column("Created Time", "订单日期", json!([{
                                "type": "parse_date_time",
                                "format": "%m/%d/%Y %I:%M:%S %p"
                            }])),
                            transfer_source_column("Order Amount", "币种", json!([{
                                "type": "extract_currency_code"
                            }])),
                            transfer_source_column("Order Amount", "订单金额", json!([{
                                "type": "parse_number",
                                "extract": true
                            }])),
                            transfer_source_column("Product Name", "商品名称", json!([])),
                            transfer_source_column("Quantity", "商品数量", json!([{
                                "type": "parse_number",
                                "extract": false
                            }])),
                            transfer_source_column("SKU Unit Original Price", "商品单价", json!([{
                                "type": "parse_number",
                                "extract": true
                            }])),
                            transfer_constant_column("网站", "www.tiktokshop.com"),
                            transfer_source_column("Tracking ID", "物流单号", json!([])),
                            transfer_constant_column("物流企业", "J&T"),
                            transfer_constant_column("电商平台", "tiktokshop")
                        ]
                    }
                }),
            ),
            context,
        )
        .await
        .expect("execute the prior two-workbook case as one native transfer");
    assert_eq!(result.metadata["success"], true);
    assert!(result.output.contains("\"rowsWritten\": 1166"));

    let output = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "read_range",
                    "path": "prior-case-regression.xlsx",
                    "sheet": "订单明细",
                    "range": {
                        "start": { "row": 2, "column": 0 },
                        "end": { "row": 2, "column": 9 }
                    }
                }),
            ),
            ToolInvocationContext::local_with_sandbox_config(
                workspace_root.clone(),
                policy,
                sandbox,
            ),
        )
        .await
        .expect("read prior-case output");
    assert!(output.output.contains("J&T"));
    assert_eq!(fs::read(&source_path).unwrap(), source_before);
    assert_eq!(fs::read(&template_path).unwrap(), template_before);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn spreadsheet_tool_round_trips_through_execution_environment() {
    let workspace_root = std::env::temp_dir().join(format!("opentopia-sheet-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy.clone());
    let written = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "write",
                    "path": "report.xlsx",
                    "sheets": [{
                        "name": "Summary",
                        "cells": [{
                            "address": { "row": 0, "column": 0 },
                            "value": { "type": "string", "value": "ready" }
                        }]
                    }]
                }),
            ),
            context,
        )
        .await
        .unwrap();
    assert_eq!(written.metadata["success"], true);
    assert!(workspace_root.join("report.xlsx").is_file());

    SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "write_rows",
                    "path": "report.xlsx",
                    "sheet": "Summary",
                    "start": { "row": 1, "column": 0 },
                    "rows": [[{ "type": "string", "value": "compact" }]]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("compact direct spreadsheet mutation");

    let read = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "read_range",
                    "path": "report.xlsx",
                    "sheet": "Summary",
                    "range": {
                        "start": { "row": 0, "column": 0 },
                        "end": { "row": 1, "column": 0 }
                    }
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .unwrap();
    assert!(read.output.contains("ready"));
    assert!(read.output.contains("compact"));

    fs::write(workspace_root.join("source.csv"), "ready\nserver-side\n").unwrap();
    let filled = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "fill_template",
                    "dataPath": "source.csv",
                    "templatePath": "report.xlsx",
                    "outputPath": "filled.xlsx",
                    "targetSheet": "Summary"
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("fill template from CSV");
    assert_eq!(filled.metadata["success"], true);
    assert!(filled.output.contains("validation"));

    let validated = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "validate",
                    "path": "filled.xlsx",
                    "expectedSheets": ["Summary"],
                    "sheets": [{
                        "sheet": "Summary",
                        "expectedRows": 2,
                        "requiredHeaders": ["ready"]
                    }]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("validate filled workbook");
    assert_eq!(validated.metadata["success"], true);
    assert!(validated.output.contains("\"valid\": true"));

    let exported = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "export_delimited",
                    "path": "filled.xlsx",
                    "outputPath": "filled.csv",
                    "sheet": "Summary"
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("export filled workbook");
    assert_eq!(exported.metadata["success"], true);
    assert_eq!(
        fs::read_to_string(workspace_root.join("filled.csv")).unwrap(),
        "ready\nserver-side\n"
    );

    let inspected_csv = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "inspect_delimited",
                    "path": "filled.csv",
                    "sampleRows": 2
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("inspect exported CSV");
    assert_eq!(inspected_csv.metadata["success"], true);
    assert!(inspected_csv.output.contains("server-side"));

    let schema = SpreadsheetTool.schema();
    let write_rows = &action_schema_branch(&schema, "write_rows")["properties"];
    assert!(write_rows.get("path").is_some());
    assert!(write_rows.get("rows").is_some());
    assert!(
        write_rows.get("outputPath").is_some(),
        "write_rows schema: {write_rows}"
    );
    assert!(write_rows.get("atomic").is_some());
    assert!(write_rows.get("operation").is_none());
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn spreadsheet_batch_copies_rows_and_writes_columns_without_model_round_trip_data() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-sheet-batch-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    for (path, sheet, values) in [
        (
            "source.xlsx",
            "Source",
            json!([
                { "address": { "row": 0, "column": 0 }, "value": { "type": "string", "value": "A001" } },
                { "address": { "row": 0, "column": 1 }, "value": { "type": "integer", "value": 10 } },
                { "address": { "row": 1, "column": 0 }, "value": { "type": "string", "value": "A002" } },
                { "address": { "row": 1, "column": 1 }, "value": { "type": "integer", "value": 20 } }
            ]),
        ),
        ("template.xlsx", "Orders", json!([])),
    ] {
        SpreadsheetTool
            .execute(
                ToolCall::new(
                    "spreadsheet",
                    json!({
                        "action": "write",
                        "outputPath": path,
                        "sheets": [{ "name": sheet, "cells": values }]
                    }),
                ),
                ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .expect("create spreadsheet fixture");
    }

    let result = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "batch",
                    "sourcePath": "template.xlsx",
                    "outputPath": "orders.xlsx",
                    "operations": [
                        {
                            "type": "copy_rows",
                            "sourcePath": "source.xlsx",
                            "sourceSheet": "Source",
                            "sourceStart": { "row": 0, "column": 0 },
                            "rowCount": 2,
                            "columnCount": 2,
                            "destinationSheet": "Orders",
                            "destinationStart": { "row": 1, "column": 1 },
                            "contentMode": "values"
                        },
                        {
                            "type": "write_columns",
                            "sheet": "Orders",
                            "start": { "row": 1, "column": 3 },
                            "columns": [[
                                { "type": "string", "value": "ready" },
                                { "type": "string", "value": "ready" }
                            ]]
                        }
                    ]
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("execute spreadsheet batch");
    assert_eq!(result.metadata["success"], true);
    assert!(result.output.contains("preservedTemplateParts"));

    let read = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "read_rows",
                    "path": "orders.xlsx",
                    "sheet": "Orders",
                    "startRow": 1,
                    "startColumn": 1,
                    "rowCount": 2,
                    "columnCount": 3
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("read spreadsheet batch output");
    assert!(read.output.contains("A001"));
    assert!(read.output.contains("A002"));
    assert!(read.output.contains("ready"));

    let found = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "find",
                    "path": "orders.xlsx",
                    "sheet": "Orders",
                    "query": "A002",
                    "matchMode": "exact",
                    "maxResults": 10
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy.clone()),
        )
        .await
        .expect("find spreadsheet cell");
    assert!(found.output.contains("A002"));

    let filtered = SpreadsheetTool
        .execute(
            ToolCall::new(
                "spreadsheet",
                json!({
                    "action": "filter_rows",
                    "path": "orders.xlsx",
                    "sheet": "Orders",
                    "range": {
                        "start": { "row": 1, "column": 1 },
                        "end": { "row": 2, "column": 3 }
                    },
                    "conditions": [{
                        "column": 2,
                        "operator": "greater_than_or_equal",
                        "value": { "type": "integer", "value": 15 }
                    }],
                    "maxResults": 10
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy),
        )
        .await
        .expect("filter spreadsheet rows");
    assert!(filtered.output.contains("A002"));
    assert!(!filtered.output.contains("A001"));
    fs::remove_dir_all(workspace_root).unwrap();
}
