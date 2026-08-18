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

    let resource = json!({ "kind": "workspaceFile", "path": "protocol.xlsx" });
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

    SpreadsheetExecuteTool
        .execute(
            ToolCall::new(
                "spreadsheet_execute",
                json!({
                    "resource": resource,
                    "operation": "write_rows",
                    "arguments": {
                        "outputPath": "protocol.xlsx",
                        "sheet": "Summary",
                        "start": { "row": 1, "column": 0 },
                        "rows": [[{ "type": "string", "value": "updated" }]]
                    }
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
                    "arguments": {
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
        .expect("read protocol workbook");
    assert!(read.output.contains("initial"));
    assert!(read.output.contains("updated"));

    let unavailable = SpreadsheetInspectTool
        .execute(
            ToolCall::new(
                "spreadsheet_inspect",
                json!({
                    "resource": {
                        "kind": "liveSession",
                        "sessionId": "session-1",
                        "documentId": "workbook-1"
                    }
                }),
            ),
            ToolInvocationContext::local(workspace_root.clone(), policy),
        )
        .await
        .expect_err("live sessions must not fall back to filesystem handling");
    assert!(unavailable
        .to_string()
        .contains("liveSession resources require an Office live-session provider"));

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
