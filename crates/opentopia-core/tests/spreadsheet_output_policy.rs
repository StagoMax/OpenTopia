use opentopia_core::spreadsheet::{
    write_workbook_preferred, SpreadsheetErrorCode, WriteWorkbookRequest,
};
use opentopia_core::tools::SpreadsheetDescribeTool;
use opentopia_core::{
    CapabilityProjection, ExecutionAuthority, LocalSandboxConfig, PermissionMode, Tool, ToolCall,
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
    let described = SpreadsheetDescribeTool
        .execute(
            ToolCall::new(
                "spreadsheet_describe",
                json!({
                    "resource": { "kind": "workspaceFile", "path": "source.xls" },
                    "operations": ["write_rows"]
                }),
            ),
            authority.local_tool_context(),
        )
        .await
        .expect("describe spreadsheet mutation contract");

    assert_eq!(described.metadata["success"], true);
    assert!(described
        .output
        .contains("workspace-relative destination path"));
    assert!(!described.output.contains("read-only sources"));
    assert!(!described.output.contains("write changed workbooks to XLSX"));
}
