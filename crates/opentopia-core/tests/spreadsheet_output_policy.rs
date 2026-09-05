use opentopia_core::spreadsheet::{
    write_workbook_preferred, SpreadsheetErrorCode, WriteWorkbookRequest,
};
use opentopia_core::{SpreadsheetWriteRangeTool, Tool};
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

#[test]
fn mutation_contract_requires_the_model_selected_path() {
    let schema = SpreadsheetWriteRangeTool.schema();
    assert!(schema["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|field| field == "path")));
    assert!(schema["properties"].get("path").is_some());
    assert!(schema["properties"].get("documentId").is_none());
    assert!(schema["properties"].get("outputPath").is_none());
}
