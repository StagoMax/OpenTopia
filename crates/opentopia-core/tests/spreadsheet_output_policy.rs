use opentopia_core::spreadsheet::{
    write_workbook_preferred, SpreadsheetErrorCode, WriteWorkbookRequest,
};
use opentopia_core::{
    CapabilityProjection, DocumentGetOperationSchemasTool, DocumentOpenTool, ExecutionAuthority,
    LocalSandboxConfig, PermissionMode, Tool, ToolCall,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn model_selected_output_reaches_backend_without_a_host_format_policy() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let output = std::env::temp_dir().join(format!(
        "opentopia-xls-output-policy-{}-{nonce}.xls",
        std::process::id()
    ));
    std::env::set_var("OPENTOPIA_SPREADSHEET_BACKEND", "backend-selection-probe");
    let error = write_workbook_preferred(&WriteWorkbookRequest {
        source: None,
        output: output.clone(),
        sheets: vec![],
    })
    .expect_err("the selected backend should report its own capability error");
    std::env::remove_var("OPENTOPIA_SPREADSHEET_BACKEND");

    assert_eq!(error.code(), SpreadsheetErrorCode::BackendUnavailable);
    assert!(error.to_string().contains("backend-selection-probe"));
    assert!(!output.exists());
}

#[tokio::test]
async fn mutation_contract_does_not_override_the_model_selected_output() {
    let authority = ExecutionAuthority::new(
        std::env::temp_dir(),
        PermissionMode::FullAccess,
        LocalSandboxConfig::from_env(),
        CapabilityProjection::unrestricted(),
    )
    .expect("valid local test authority");
    let opened = DocumentOpenTool
        .execute(
            ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": "source.xls" },
                    "mode": "create"
                }),
            ),
            authority.local_tool_context(),
        )
        .await
        .expect("open model-selected spreadsheet path");
    let document_id = opened.metadata["documentId"].clone();
    let described = DocumentGetOperationSchemasTool
        .execute(
            ToolCall::new(
                "document_get_operation_schemas",
                json!({ "documentId": document_id, "operations": ["write_rows"] }),
            ),
            authority.local_tool_context(),
        )
        .await
        .expect("load spreadsheet mutation contract");

    assert_eq!(described.metadata["success"], true);
    let payload: serde_json::Value = serde_json::from_str(&described.output).unwrap();
    let schema = &payload["operations"][0]["argumentsSchema"];
    assert!(schema["properties"].get("path").is_none());
    assert!(schema["properties"].get("outputPath").is_none());
}
