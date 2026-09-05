use super::{
    inspect_workbook, list_sheets, run_openpyxl_worker, validate_range_dimensions, CellRange,
    InspectWorkbookRequest, ListSheetsRequest, SheetVisibility, SpreadsheetError,
    SpreadsheetFileFormat, EXCEL_MAX_ROWS, MAX_OUTPUT_FILE_BYTES, MAX_WRITE_UPDATES,
};
use crate::office_runtime::OfficeRuntime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

const MAX_STRUCTURE_OPERATIONS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditWorkbookStructureRequest {
    pub source: PathBuf,
    pub output: PathBuf,
    pub operations: Vec<SpreadsheetStructureOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SpreadsheetStructureOperation {
    CopySheet {
        source: PathBuf,
        source_sheet: String,
        destination_sheet: String,
        #[serde(default)]
        visibility: Option<SheetVisibility>,
    },
    DeleteRows {
        sheet: String,
        rows: Vec<u32>,
    },
    DeleteSheet {
        sheet: String,
    },
    SetNumberFormat {
        sheet: String,
        range: CellRange,
        number_format: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditWorkbookStructureResult {
    pub output: PathBuf,
    pub bytes_written: u64,
    pub sheet_count: usize,
    pub populated_cells: u64,
    pub applied_operations: usize,
}

pub fn edit_workbook_structure(
    request: &EditWorkbookStructureRequest,
) -> Result<EditWorkbookStructureResult, SpreadsheetError> {
    validate_ooxml_path(&request.source)?;
    validate_ooxml_path(&request.output)?;
    if request.operations.is_empty() || request.operations.len() > MAX_STRUCTURE_OPERATIONS {
        return Err(SpreadsheetError::ValidationFailed {
            message: format!(
                "structure edit requires between 1 and {MAX_STRUCTURE_OPERATIONS} operations"
            ),
        });
    }

    let original = list_sheets(&ListSheetsRequest {
        path: request.source.clone(),
    })?;
    let mut known_sheets = original
        .sheets
        .iter()
        .map(|sheet| sheet.name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for operation in &request.operations {
        match operation {
            SpreadsheetStructureOperation::CopySheet {
                source,
                source_sheet,
                destination_sheet,
                ..
            } => {
                validate_sheet_name(destination_sheet)?;
                let source_sheets = list_sheets(&ListSheetsRequest {
                    path: source.clone(),
                })?;
                if !source_sheets
                    .sheets
                    .iter()
                    .any(|sheet| sheet.name.eq_ignore_ascii_case(source_sheet))
                {
                    return Err(SpreadsheetError::SheetNotFound {
                        sheet: source_sheet.clone(),
                    });
                }
                if !known_sheets.insert(destination_sheet.to_ascii_lowercase()) {
                    return Err(SpreadsheetError::DuplicateSheet {
                        sheet: destination_sheet.clone(),
                    });
                }
            }
            SpreadsheetStructureOperation::DeleteRows { sheet, rows } => {
                if !known_sheets.contains(&sheet.to_ascii_lowercase()) {
                    return Err(SpreadsheetError::SheetNotFound {
                        sheet: sheet.clone(),
                    });
                }
                if rows.is_empty() {
                    return Err(SpreadsheetError::InvalidRange {
                        reason: "delete_rows requires at least one row",
                    });
                }
                if rows.iter().any(|row| *row >= EXCEL_MAX_ROWS) {
                    return Err(SpreadsheetError::InvalidRange {
                        reason: "delete_rows contains a row outside XLSX bounds",
                    });
                }
            }
            SpreadsheetStructureOperation::DeleteSheet { sheet } => {
                if !known_sheets.remove(&sheet.to_ascii_lowercase()) {
                    return Err(SpreadsheetError::SheetNotFound {
                        sheet: sheet.clone(),
                    });
                }
                if known_sheets.is_empty() {
                    return Err(SpreadsheetError::NoSheets);
                }
            }
            SpreadsheetStructureOperation::SetNumberFormat {
                sheet,
                range,
                number_format,
            } => {
                if !known_sheets.contains(&sheet.to_ascii_lowercase()) {
                    return Err(SpreadsheetError::SheetNotFound {
                        sheet: sheet.clone(),
                    });
                }
                let (_, _, cells) = validate_range_dimensions(*range)?;
                if cells > MAX_WRITE_UPDATES as u64 {
                    return Err(SpreadsheetError::TooManyCells {
                        context: "number format range",
                        actual: usize::try_from(cells).unwrap_or(usize::MAX),
                        limit: MAX_WRITE_UPDATES,
                    });
                }
                let trimmed = number_format.trim();
                if trimmed.is_empty() || trimmed.len() > 255 || trimmed.contains('\0') {
                    return Err(SpreadsheetError::ValidationFailed {
                        message: "number format must contain 1 to 255 characters".to_string(),
                    });
                }
            }
        }
    }

    let python = OfficeRuntime::shared()
        .python_for_openpyxl()
        .map_err(|error| SpreadsheetError::BackendUnavailable {
            message: error.to_string(),
        })?;
    let worker = run_openpyxl_worker(request, &request.output, &python.executable)?;
    let applied_operations = worker
        .get("appliedStructureOperations")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(request.operations.len() as u64) as usize;
    let bytes_written = fs::metadata(&request.output)
        .map_err(|source| SpreadsheetError::Io {
            operation: "inspect",
            path: request.output.clone(),
            source,
        })?
        .len();
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        let _ = fs::remove_file(&request.output);
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }
    let inspected = inspect_workbook(&InspectWorkbookRequest {
        path: request.output.clone(),
    })?;
    Ok(EditWorkbookStructureResult {
        output: request.output.clone(),
        bytes_written,
        sheet_count: inspected.sheets.len(),
        populated_cells: inspected.populated_cells,
        applied_operations,
    })
}

fn validate_ooxml_path(path: &std::path::Path) -> Result<(), SpreadsheetError> {
    if SpreadsheetFileFormat::from_path(path).is_some_and(SpreadsheetFileFormat::is_ooxml) {
        Ok(())
    } else {
        Err(SpreadsheetError::UnsupportedFormat {
            path: path.to_path_buf(),
            extension: path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_string),
        })
    }
}

fn validate_sheet_name(sheet: &str) -> Result<(), SpreadsheetError> {
    let trimmed = sheet.trim();
    if trimmed.is_empty() {
        return Err(SpreadsheetError::InvalidSheetName {
            sheet: sheet.to_string(),
            reason: "name must not be empty",
        });
    }
    if trimmed.chars().count() > 31 {
        return Err(SpreadsheetError::InvalidSheetName {
            sheet: sheet.to_string(),
            reason: "name exceeds 31 characters",
        });
    }
    if trimmed
        .chars()
        .any(|character| "[]:*?/\\".contains(character))
    {
        return Err(SpreadsheetError::InvalidSheetName {
            sheet: sheet.to_string(),
            reason: "name contains an invalid character",
        });
    }
    Ok(())
}
