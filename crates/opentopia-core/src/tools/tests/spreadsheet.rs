use super::*;

fn context(workspace_root: &Path, policy: Arc<BasicPolicyEngine>) -> ToolInvocationContext {
    ToolInvocationContext::local(workspace_root.to_path_buf(), policy)
}

fn temporary_workspace(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("opentopia-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn assert_property_names_are_snake_case(value: &Value, location: &str) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, schema) in properties {
                    assert!(
                        name.bytes()
                            .all(|byte| !byte.is_ascii_uppercase() && byte != b'-'),
                        "non-snake-case property {name:?} at {location}"
                    );
                    assert_property_names_are_snake_case(schema, &format!("{location}.{name}"));
                }
            }
            for (name, child) in object {
                if name != "properties" {
                    assert_property_names_are_snake_case(child, location);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                assert_property_names_are_snake_case(value, location);
            }
        }
        _ => {}
    }
}

#[test]
fn spreadsheet_surface_is_independent_and_path_based() {
    let registry = ToolRegistry::with_builtins();
    let expected = [
        "spreadsheet_inspect",
        "spreadsheet_read_ranges",
        "spreadsheet_find",
        "spreadsheet_filter_rows",
        "spreadsheet_validate",
        "spreadsheet_write_range",
        "spreadsheet_copy_ranges",
        "spreadsheet_copy_rows",
        "spreadsheet_fill_ranges",
        "spreadsheet_convert_ranges",
        "spreadsheet_export_delimited",
        "spreadsheet_copy_sheet",
        "spreadsheet_delete_rows",
        "spreadsheet_delete_sheet",
    ];
    for name in expected {
        let tool = registry
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert!(tool.provider_contract_loader().is_none());
        let schema = tool.schema();
        assert!(
            schema.get("oneOf").is_none(),
            "{name} must have one fixed input"
        );
        assert!(schema["properties"].get("documentId").is_none());
        assert!(schema["properties"].get("selectionId").is_none());
        assert!(schema["properties"].get("attachmentId").is_none());
        assert_property_names_are_snake_case(&schema, name);
    }
    for removed in [
        "document_open",
        "document_get_operation_schemas",
        "document_execute",
        "fill_template",
        "transfer_rows",
    ] {
        assert!(
            registry.get(removed).is_none(),
            "legacy tool {removed} survived"
        );
    }
}

#[test]
fn spreadsheet_model_contract_uses_concise_excel_coordinates() {
    let write = SpreadsheetWriteRangeTool.schema();
    assert_eq!(write["properties"]["start"]["type"], "string");
    assert!(write["properties"].get("template").is_some());
    assert!(write["properties"].get("basePath").is_none());

    let filter = SpreadsheetFilterRowsTool.schema();
    assert_eq!(
        filter["properties"]["conditions"]["items"]["properties"]["column"]["type"],
        "string"
    );
    assert!(filter["properties"].get("return_mode").is_some());

    let copy_rows = SpreadsheetCopyRowsTool.schema();
    assert!(copy_rows["properties"].get("source_header_row").is_some());
    assert!(copy_rows["properties"].get("source_data_row").is_some());
    assert!(copy_rows["properties"].get("source_range").is_none());
    assert!(copy_rows["properties"]
        .get("destination_header_row")
        .is_some());
    assert!(copy_rows["properties"].get("destination_row").is_none());

    let validate = SpreadsheetValidateTool.schema();
    assert_eq!(
        validate["properties"]["sheets"]["items"]["properties"]["ranges"]["items"]["properties"]
            ["range"]["type"],
        "string"
    );
    let sheet = &validate["properties"]["sheets"]["items"];
    assert!(sheet["properties"].get("header").is_some());
    assert!(sheet["properties"].get("header_row").is_none());

    assert!(SpreadsheetConvertRangesTool
        .input_error(&json!({
            "path": "book.xlsx",
            "conversions": [{
                "sheet": "Data",
                "range": "A1",
                "transforms": [{
                    "type": "parse_date_time",
                    "inputFormat": "%Y-%m-%d",
                    "outputNumberFormat": "yyyy-mm-dd"
                }]
            }]
        }))
        .is_some());
    assert!(SpreadsheetValidateTool
        .input_error(&json!({
            "path": "book.xlsx",
            "sheets": [{ "sheet": "Data", "headerRow": 0 }]
        }))
        .is_some());
}

#[tokio::test]
async fn atomic_tools_write_inspect_and_read_the_same_path() {
    let workspace_root = temporary_workspace("atomic-spreadsheet");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let workbook = workspace_root.join("orders.xlsx");

    let written = SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": workbook,
                    "sheet": "Orders",
                    "start": "A1",
                    "rows": [
                        [
                            { "type": "string", "value": "Order ID" },
                            { "type": "string", "value": "Status" }
                        ],
                        [
                            { "type": "integer", "value": 335 },
                            { "type": "string", "value": "paid" }
                        ]
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("write workbook");
    assert_eq!(written.metadata["success"], true);
    assert!(workbook.is_file());

    let inspected = SpreadsheetInspectTool
        .execute(
            ToolCall::new("spreadsheet_inspect", json!({ "path": workbook })),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("inspect workbook");
    assert_eq!(inspected.metadata["success"], true);
    assert!(inspected.output.contains("Orders"));

    let read = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": workbook,
                        "sheet": "Orders",
                        "range": "A1:B2"
                    }]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("read workbook");
    assert_eq!(read.metadata["success"], true);
    assert!(read.output.contains("Order ID"));
    assert!(read.output.contains("paid"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn one_read_call_batches_ranges_across_workbooks() {
    let workspace_root = temporary_workspace("spreadsheet-multi-workbook-read");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let first = workspace_root.join("first.xlsx");
    let second = workspace_root.join("second.xlsx");
    for (path, value) in [(&first, "first-value"), (&second, "second-value")] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({
                        "path": path,
                        "sheet": "Data",
                        "start": "A1",
                        "rows": [[{ "type": "string", "value": value }]]
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create read fixture");
    }

    let read = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [
                        { "path": first, "sheet": "Data", "range": "A1" },
                        { "path": second, "sheet": "Data", "range": "A1" }
                    ]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("batch read workbooks");
    let result: Value = serde_json::from_str(&read.output).expect("read result JSON");
    assert_eq!(result["reads"].as_array().map(Vec::len), Some(2));
    assert!(read.output.contains("first-value"));
    assert!(read.output.contains("second-value"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn write_range_creates_destination_from_base_without_a_file_copy() {
    let workspace_root = temporary_workspace("spreadsheet-write-from-base");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let template = workspace_root.join("template.xlsx");
    let output = workspace_root.join("output.xlsx");

    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": template,
                    "sheet": "Template",
                    "start": "A1",
                    "rows": [[{ "type": "string", "value": "preserved" }]]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create template");

    let written = SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": output,
                    "template": template,
                    "sheet": "Template",
                    "start": "B1",
                    "rows": [[{ "type": "string", "value": "new" }]]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("write destination from base");
    assert_eq!(written.metadata["success"], true);
    assert!(output.is_file());

    for (path, expected, absent) in [
        (&output, "new", None),
        (&template, "preserved", Some("new")),
    ] {
        let read = SpreadsheetReadRangesTool
            .execute(
                ToolCall::new(
                    "spreadsheet_read_ranges",
                    json!({
                        "reads": [{
                            "path": path,
                            "sheet": "Template",
                            "range": "A1:B1"
                        }]
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("read workbook");
        assert!(read.output.contains(expected));
        if let Some(absent) = absent {
            assert!(!read.output.contains(absent));
        }
    }

    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": output,
                    "template": template,
                    "sheet": "Template",
                    "start": "C1",
                    "rows": [[{ "type": "string", "value": "rerun" }]]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("rerun against the same output path");
    let rerun = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": output,
                        "sheet": "Template",
                        "range": "A1:C1"
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("read rerun output");
    assert!(rerun.output.contains("preserved"));
    assert!(rerun.output.contains("rerun"));
    assert!(!rerun.output.contains("new"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn oversized_batch_read_returns_per_range_continuation() {
    let workspace_root = temporary_workspace("spreadsheet-pagination");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let workbook = workspace_root.join("large-request.xlsx");
    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": workbook,
                    "sheet": "Data",
                    "start": "A1",
                    "rows": [[{ "type": "string", "value": "header" }]]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create workbook");

    let read = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": workbook,
                        "sheet": "Data",
                        "range": "A1:A1499"
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("bounded read");
    let value: Value = serde_json::from_str(&read.output).expect("read result JSON");
    assert_eq!(value["reads"][0]["page"]["returnedRange"], "A1:A1000");
    assert_eq!(value["reads"][0]["page"]["hasMore"], true);
    assert_eq!(value["reads"][0]["page"]["nextStartRow"], "A1001");

    let wide = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": workbook,
                        "sheet": "Data",
                        "range": "A1:BG1501"
                    }]
                }),
            ),
            context(
                &workspace_root,
                Arc::new(BasicPolicyEngine::new(
                    workspace_root.clone(),
                    PermissionMode::FullAccess,
                )),
            ),
        )
        .await
        .expect("bounded wide read");
    let value: Value = serde_json::from_str(&wide.output).expect("wide result JSON");
    assert_eq!(value["reads"][0]["page"]["returnedRange"], "A1:BG169");
    assert_eq!(value["reads"][0]["page"]["nextStartRow"], "A170");

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn convert_ranges_update_one_file_and_skip_blank_and_formula_cells() {
    let workspace_root = temporary_workspace("spreadsheet-convert");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let workbook = workspace_root.join("amounts.xlsx");
    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": workbook,
                    "sheet": "Orders",
                    "start": "A1",
                    "rows": [
                        [{ "type": "string", "value": "Amount" }],
                        [{ "type": "string", "value": "USD 1,234.50" }],
                        [{ "type": "blank" }],
                        [{ "type": "formula", "value": { "expression": "=A2*2" } }]
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create conversion fixture");

    let converted = SpreadsheetConvertRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_convert_ranges",
                json!({
                    "path": workbook,
                    "conversions": [{
                        "sheet": "Orders",
                        "range": "A2:A4",
                        "transforms": [{ "type": "parse_number", "extract": true }]
                    }]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("convert range");
    assert_eq!(converted.metadata["success"], true);
    assert_eq!(converted.metadata["toolName"], "spreadsheet_convert_ranges");

    let read = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": workbook,
                        "sheet": "Orders",
                        "range": "A1:A4"
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("read converted workbook");
    assert!(read.output.contains("1234.5"));
    assert!(read.output.contains("A2*2"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn file_to_file_copy_is_not_limited_by_model_read_pages() {
    let workspace_root = temporary_workspace("spreadsheet-copy-pages");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let target = workspace_root.join("target.xlsx");
    let rows = (0..1166)
        .map(|value| json!([{ "type": "integer", "value": value }]))
        .collect::<Vec<_>>();
    for (path, sheet, rows) in [
        (source.clone(), "Source", json!(rows)),
        (
            target.clone(),
            "Target",
            json!([[{ "type": "string", "value": "placeholder" }]]),
        ),
    ] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({
                        "path": path,
                        "sheet": sheet,
                        "start": "A1",
                        "rows": rows
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create copy fixture");
    }

    let copied = SpreadsheetCopyRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_copy_ranges",
                json!({
                    "path": target,
                    "copies": [{
                        "source_path": source,
                        "source_sheet": "Source",
                        "source_range": "A1:A1166",
                        "destination_sheet": "Target",
                        "destination_start": "A1"
                    }]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("copy more than one model read page");
    assert_eq!(copied.metadata["success"], true);

    let tail = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": target,
                        "sheet": "Target",
                        "range": "A1001:A1166"
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("read copied tail");
    assert!(tail.output.contains("1165"));

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn batched_copy_fill_and_convert_create_one_formatted_output() {
    if crate::office_runtime::OfficeRuntime::shared()
        .python_for_openpyxl()
        .is_err()
    {
        return;
    }
    let workspace_root = temporary_workspace("spreadsheet-batched-transfer");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let template = workspace_root.join("template.xlsx");
    let output = workspace_root.join("订单结果.xlsx");

    let mut source_rows = vec![json!([
        { "type": "string", "value": "Created Time" },
        { "type": "string", "value": "Amount" }
    ])];
    source_rows.extend((0..1166).map(|index| {
        json!([
            { "type": "string", "value": "08/18/2026 10:23:07 PM" },
            { "type": "string", "value": format!("USD {}.50", index + 1) }
        ])
    }));
    for (path, sheet, rows) in [
        (source.clone(), "订单源", json!(source_rows)),
        (
            template.clone(),
            "订单明细",
            json!([[
                { "type": "string", "value": "订单日期" },
                { "type": "string", "value": "订单金额" },
                { "type": "string", "value": "物流" },
                { "type": "string", "value": "平台" }
            ]]),
        ),
    ] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({
                        "path": path,
                        "sheet": sheet,
                        "start": "A1",
                        "rows": rows
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create batched transfer fixture");
    }

    let copied = SpreadsheetCopyRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_copy_ranges",
                json!({
                    "path": output,
                    "template": template,
                    "copies": [
                        {
                            "source_path": source,
                            "source_sheet": "订单源",
                            "source_range": "A2:A1167",
                            "destination_sheet": "订单明细",
                            "destination_start": "A2"
                        },
                        {
                            "source_path": source,
                            "source_sheet": "订单源",
                            "source_range": "B2:B1167",
                            "destination_sheet": "订单明细",
                            "destination_start": "B2"
                        }
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("copy ranges in one transaction");
    assert_eq!(copied.metadata["success"], true);
    assert!(output.is_file());

    let filled = SpreadsheetFillRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_fill_ranges",
                json!({
                    "path": output,
                    "fills": [
                        {
                            "sheet": "订单明细",
                            "range": "C2:C1167",
                            "value": { "type": "string", "value": "J&T" }
                        },
                        {
                            "sheet": "订单明细",
                            "range": "D2:D1167",
                            "value": { "type": "string", "value": "tiktokshop" }
                        }
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("fill ranges in one transaction");
    assert_eq!(filled.metadata["success"], true);

    let converted = SpreadsheetConvertRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_convert_ranges",
                json!({
                    "path": output,
                    "conversions": [
                        {
                            "sheet": "订单明细",
                            "range": "A2:A1167",
                            "transforms": [{
                                "type": "parse_date_time",
                                "input_format": "%m/%d/%Y %I:%M:%S %p",
                                "output_number_format": "yyyy-mm-dd"
                            }]
                        },
                        {
                            "sheet": "订单明细",
                            "range": "B2:B1167",
                            "transforms": [{ "type": "parse_number", "extract": true }]
                        }
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("convert ranges in one transaction");
    assert_eq!(converted.metadata["success"], true);

    let validated = SpreadsheetValidateTool
        .execute(
            ToolCall::new(
                "spreadsheet_validate",
                json!({
                    "path": output,
                    "expected_populated_cells": 4668,
                    "expected_sheets": ["订单明细"],
                    "sheets": [{
                        "sheet": "订单明细",
                        "expected_rows": 1167,
                        "expected_data_rows": 1166,
                        "header": {
                            "row": 1,
                            "required": ["订单日期", "订单金额", "物流", "平台"]
                        },
                        "ranges": [
                            {
                                "range": "A2:A1167",
                                "expected_type": "date_time",
                                "expected_number_format": "yyyy-mm-dd"
                            },
                            {
                                "range": "B2:B1167",
                                "expected_type": "number"
                            }
                        ]
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("validate batched output");
    assert_eq!(validated.metadata["success"], true);
    assert_eq!(validated.metadata["validationPassed"], true);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn copy_rows_filters_and_maps_more_than_one_read_page_without_intermediate_files() {
    let workspace_root = temporary_workspace("spreadsheet-copy-filtered-rows");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let template = workspace_root.join("template.xlsx");
    let output = workspace_root.join("output.xlsx");

    let mut source_rows = vec![
        json!([
            { "type": "string", "value": "Order ID" },
            { "type": "string", "value": "Status" },
            { "type": "string", "value": "Created Time" }
        ]),
        json!([
            { "type": "string", "value": "Order identifier." },
            { "type": "string", "value": "Order payment status." },
            { "type": "string", "value": "Order created time." }
        ]),
    ];
    source_rows.extend((1..=1_500).map(|id| {
        let status = if id <= 330 {
            "cancelled"
        } else if id <= 334 {
            "unpaid"
        } else {
            "paid"
        };
        json!([
            { "type": "integer", "value": id },
            { "type": "string", "value": status },
            { "type": "string", "value": "08/18/2026 10:23:07 PM" }
        ])
    }));
    for (path, sheet, rows) in [
        (source.clone(), "Orders", json!(source_rows)),
        (
            template.clone(),
            "Order detail",
            json!([[{ "type": "string", "value": "ID" }, { "type": "string", "value": "Date" }]]),
        ),
    ] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({ "path": path, "sheet": sheet, "start": "A1", "rows": rows }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create row-copy fixture");
    }

    let arguments = json!({
        "path": output,
        "template": template,
        "source_path": source,
        "source_sheet": "Orders",
        "source_header_row": 1,
        "source_data_row": 3,
        "destination_sheet": "Order detail",
        "destination_header_row": 1,
        "columns": [
            { "source": "A", "destination": "A" },
            {
                "source": "C",
                "destination": "B",
                "transforms": [{
                    "type": "parse_date_time",
                    "input_format": "%m/%d/%Y %I:%M:%S %p",
                    "output_number_format": "yyyy-mm-dd"
                }]
            }
        ],
        "conditions": [
            { "column": "B", "operator": "not_equals", "value": { "type": "string", "value": "cancelled" } },
            { "column": "B", "operator": "not_equals", "value": { "type": "string", "value": "unpaid" } }
        ],
        "match_mode": "all"
    });
    for _ in 0..2 {
        let copied = SpreadsheetCopyRowsTool
            .execute(
                ToolCall::new("spreadsheet_copy_rows", arguments.clone()),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("copy filtered rows directly into the template");
        assert_eq!(copied.metadata["success"], true);
        assert_eq!(copied.metadata["copiedRows"], 1166);
    }

    let inspected =
        crate::spreadsheet::inspect_workbook(&crate::spreadsheet::InspectWorkbookRequest {
            path: output.clone(),
        })
        .expect("inspect row-copy output");
    assert_eq!(inspected.sheets[0].used_range.unwrap().end.row, 1166);
    let first_and_last = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [
                        { "path": output, "sheet": "Order detail", "range": "A2:B2" },
                        { "path": output, "sheet": "Order detail", "range": "A1167:B1167" }
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("read row-copy boundaries");
    assert!(first_and_last.output.contains("335"));
    assert!(first_and_last.output.contains("1500"));
    assert!(first_and_last.output.contains("\"date_time\""));
    assert!(!first_and_last.output.contains("\"type\": \"date_time\""));

    let validated = SpreadsheetValidateTool
        .execute(
            ToolCall::new(
                "spreadsheet_validate",
                json!({
                    "path": output,
                    "sheets": [{
                        "sheet": "Order detail",
                        "expected_data_rows": 1166,
                        "header": { "row": 1, "required": ["ID", "Date"] },
                        "ranges": [{
                            "range": "B2:B1167",
                            "expected_type": "date_time",
                            "expected_number_format": "yyyy-mm-dd"
                        }]
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("validate copied and converted rows");
    assert_eq!(validated.metadata["success"], true);
    assert_eq!(validated.metadata["validationPassed"], true);
    assert_eq!(fs::read_dir(&workspace_root).unwrap().count(), 3);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn copy_rows_rejects_an_invalid_destination_header_before_creating_output() {
    let workspace_root = temporary_workspace("spreadsheet-copy-header-guard");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let template = workspace_root.join("template.xlsx");
    let output = workspace_root.join("output.xlsx");

    for (path, rows) in [
        (
            source.clone(),
            json!([
                [{ "type": "string", "value": "ID" }],
                [{ "type": "integer", "value": 7 }]
            ]),
        ),
        (
            template.clone(),
            json!([[{ "type": "string", "value": "ID" }]]),
        ),
    ] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({ "path": path, "sheet": "Data", "start": "A1", "rows": rows }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create header guard fixture");
    }

    let error = SpreadsheetCopyRowsTool
        .execute(
            ToolCall::new(
                "spreadsheet_copy_rows",
                json!({
                    "path": output,
                    "template": template,
                    "source_path": source,
                    "source_sheet": "Data",
                    "source_header_row": 1,
                    "source_data_row": 2,
                    "destination_sheet": "Data",
                    "destination_header_row": 2,
                    "columns": [{ "source": "A", "destination": "A" }],
                    "conditions": [{
                        "column": "A",
                        "operator": "greater_than",
                        "value": { "type": "integer", "value": 0 }
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect_err("reject an invalid destination header");
    assert!(error.to_string().contains("destination_header_row"));
    assert!(!output.exists());

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn validation_counts_every_populated_row_when_header_is_omitted() {
    let workspace_root = temporary_workspace("spreadsheet-headerless-validation");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let workbook = workspace_root.join("lookup.xlsx");
    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": workbook,
                    "sheet": "Currencies",
                    "start": "A1",
                    "rows": [
                        [{ "type": "string", "value": "USD" }],
                        [{ "type": "string", "value": "MXN" }]
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create headerless workbook");

    let validated = SpreadsheetValidateTool
        .execute(
            ToolCall::new(
                "spreadsheet_validate",
                json!({
                    "path": workbook,
                    "sheets": [{
                        "sheet": "Currencies",
                        "expected_rows": 2,
                        "expected_data_rows": 2
                    }]
                }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("validate headerless sheet");
    assert_eq!(validated.metadata["success"], true);
    assert_eq!(validated.metadata["validationPassed"], true);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn structure_tools_copy_filter_delete_and_remove_a_sheet() {
    if crate::office_runtime::OfficeRuntime::shared()
        .python_for_openpyxl()
        .is_err()
    {
        return;
    }
    let workspace_root = temporary_workspace("spreadsheet-structure");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let target = workspace_root.join("target.xlsx");
    for (path, sheet, rows) in [
        (
            source.clone(),
            "Orders",
            json!([
                [{ "type": "string", "value": "ID" }, { "type": "string", "value": "Status" }],
                [{ "type": "integer", "value": 1 }, { "type": "string", "value": "paid" }],
                [{ "type": "integer", "value": 2 }, { "type": "string", "value": "cancelled" }],
                [{ "type": "integer", "value": 3 }, { "type": "string", "value": "unpaid" }]
            ]),
        ),
        (
            target.clone(),
            "Keep",
            json!([[{ "type": "string", "value": "template" }]]),
        ),
    ] {
        SpreadsheetWriteRangeTool
            .execute(
                ToolCall::new(
                    "spreadsheet_write_range",
                    json!({
                        "path": path,
                        "sheet": sheet,
                        "start": "A1",
                        "rows": rows
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("create structure fixture");
    }

    let copied = SpreadsheetCopySheetTool
        .execute(
            ToolCall::new(
                "spreadsheet_copy_sheet",
                json!({
                    "source_path": source,
                    "source_sheet": "Orders",
                    "path": target,
                    "destination_sheet": "源数据_处理",
                    "visibility": "hidden"
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("copy sheet");
    assert_eq!(copied.metadata["success"], true);

    let deleted = SpreadsheetDeleteRowsTool
        .execute(
            ToolCall::new(
                "spreadsheet_delete_rows",
                json!({
                    "path": target,
                    "sheet": "源数据_处理",
                    "range": "A2:B4",
                    "conditions": [
                        {
                            "column": "B",
                            "operator": "equals",
                            "value": { "type": "string", "value": "cancelled" },
                            "case_sensitive": false
                        },
                        {
                            "column": "B",
                            "operator": "equals",
                            "value": { "type": "string", "value": "unpaid" },
                            "case_sensitive": false
                        }
                    ],
                    "match_mode": "any"
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("delete matching rows");
    assert_eq!(deleted.metadata["success"], true);

    let read = SpreadsheetReadRangesTool
        .execute(
            ToolCall::new(
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": target,
                        "sheet": "源数据_处理",
                        "range": "A1:B2"
                    }]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("read edited rows");
    assert!(!read.output.contains("cancelled"));
    assert!(!read.output.contains("unpaid"));
    assert!(read.output.contains("paid"));

    let removed = SpreadsheetDeleteSheetTool
        .execute(
            ToolCall::new(
                "spreadsheet_delete_sheet",
                json!({ "path": target, "sheet": "源数据_处理" }),
            ),
            context(&workspace_root, policy),
        )
        .await
        .expect("delete staging sheet");
    assert_eq!(removed.metadata["success"], true);

    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn delete_rows_creates_filtered_destination_from_base() {
    if crate::office_runtime::OfficeRuntime::shared()
        .python_for_openpyxl()
        .is_err()
    {
        return;
    }
    let workspace_root = temporary_workspace("spreadsheet-filter-from-base");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let source = workspace_root.join("source.xlsx");
    let output = workspace_root.join("filtered.xlsx");

    SpreadsheetWriteRangeTool
        .execute(
            ToolCall::new(
                "spreadsheet_write_range",
                json!({
                    "path": source,
                    "sheet": "Orders",
                    "start": "A1",
                    "rows": [
                        [{ "type": "string", "value": "Status" }],
                        [{ "type": "string", "value": "paid" }],
                        [{ "type": "string", "value": "cancelled" }],
                        [{ "type": "string", "value": "unpaid" }]
                    ]
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create source");

    let deleted = SpreadsheetDeleteRowsTool
        .execute(
            ToolCall::new(
                "spreadsheet_delete_rows",
                json!({
                    "path": output,
                    "template": source,
                    "sheet": "Orders",
                    "range": "A2:A4",
                    "conditions": [
                        {
                            "column": "A",
                            "operator": "equals",
                            "value": { "type": "string", "value": "cancelled" }
                        },
                        {
                            "column": "A",
                            "operator": "equals",
                            "value": { "type": "string", "value": "unpaid" }
                        }
                    ],
                    "match_mode": "any"
                }),
            ),
            context(&workspace_root, policy.clone()),
        )
        .await
        .expect("create filtered destination");
    assert_eq!(deleted.metadata["success"], true);

    for (path, contains_removed) in [(&source, true), (&output, false)] {
        let read = SpreadsheetReadRangesTool
            .execute(
                ToolCall::new(
                    "spreadsheet_read_ranges",
                    json!({
                        "reads": [{
                            "path": path,
                            "sheet": "Orders",
                            "range": "A1:A4"
                        }]
                    }),
                ),
                context(&workspace_root, policy.clone()),
            )
            .await
            .expect("read filtered workbook");
        assert!(read.output.contains("paid"));
        assert_eq!(read.output.contains("cancelled"), contains_removed);
        assert_eq!(read.output.contains("unpaid"), contains_removed);
    }

    fs::remove_dir_all(workspace_root).unwrap();
}

#[test]
fn word_tool_name_is_format_specific() {
    assert_eq!(Tool::name(&DocumentTool), "word_document");
    let registry = ToolRegistry::with_builtins();
    assert!(registry.get("word_document").is_some());
    assert!(registry.get("document").is_none());
}
