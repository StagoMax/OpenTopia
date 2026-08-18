use super::{
    apply_sheet_updates, ensure_return_size, ensure_workbook_cell_count, inspect_workbook,
    load_workbook, patch_workbook_template, validate_address, validate_write_text, CellAddress,
    CellRange, InspectWorkbookRequest, SheetWriteRequest, SpreadsheetCellInput,
    SpreadsheetCellValue, SpreadsheetError, StoredCell, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS,
    MAX_INPUT_FILE_BYTES, MAX_OUTPUT_FILE_BYTES, MAX_WORKBOOK_CELLS,
};
use crate::delimited;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

const MAX_DELIMITED_SAMPLE_ROWS: usize = 20;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelimitedFormat {
    #[default]
    Csv,
    Tsv,
}

impl DelimitedFormat {
    pub fn delimiter(self) -> u8 {
        match self {
            Self::Csv => b',',
            Self::Tsv => b'\t',
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Csv => "text/csv",
            Self::Tsv => "text/tab-separated-values",
        }
    }

    fn resolve(path: &Path, requested: Option<Self>) -> Result<Self, SpreadsheetError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        let inferred = match extension.as_deref() {
            Some("csv") => Some(Self::Csv),
            Some("tsv") | Some("tab") => Some(Self::Tsv),
            _ => None,
        };
        match (requested, inferred) {
            (Some(requested), Some(inferred)) if requested != inferred => {
                Err(SpreadsheetError::InvalidDelimited {
                    path: path.to_path_buf(),
                    message: format!(
                        "requested {:?} format does not match .{} extension",
                        requested,
                        extension.as_deref().unwrap_or_default()
                    ),
                })
            }
            (Some(requested), _) => Ok(requested),
            (None, Some(inferred)) => Ok(inferred),
            (None, None) => Err(SpreadsheetError::InvalidDelimited {
                path: path.to_path_buf(),
                message: "expected a .csv, .tsv, or .tab path, or an explicit format".to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "by",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DelimitedColumnSelector {
    /// Zero-based physical column index.
    Index { index: u32 },
    /// Header name plus a one-based occurrence for duplicate headers.
    Header {
        name: String,
        #[serde(default = "default_occurrence")]
        #[schemars(range(min = 1))]
        occurrence: u32,
    },
}

fn default_occurrence() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DelimitedColumnMapping {
    pub source: DelimitedColumnSelector,
    pub target: DelimitedColumnSelector,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelimitedFormulaMode {
    #[default]
    Values,
    Formulas,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InspectDelimitedRequest {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DelimitedFormat>,
    #[serde(default)]
    pub header_row: u32,
    #[serde(default = "default_sample_rows")]
    #[schemars(range(min = 1, max = 20))]
    pub sample_rows: usize,
    #[serde(default)]
    pub rstrip_tabs: bool,
}

fn default_sample_rows() -> usize {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelimitedHeader {
    pub name: String,
    pub column: u32,
    pub occurrence: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateDelimitedHeader {
    pub name: String,
    pub occurrences: u32,
    pub columns: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InspectDelimitedResult {
    pub path: PathBuf,
    pub format: DelimitedFormat,
    pub record_count: u32,
    pub data_row_count: u32,
    pub column_count: u32,
    pub headers: Vec<DelimitedHeader>,
    pub duplicate_headers: Vec<DuplicateDelimitedHeader>,
    pub ragged_row_count: u32,
    pub sample_rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FillTemplateRequest {
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_format: Option<DelimitedFormat>,
    pub template: PathBuf,
    pub output: PathBuf,
    pub target_sheet: String,
    #[serde(default)]
    pub source_header_row: u32,
    #[serde(default)]
    pub target_header_row: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_start_row: Option<u32>,
    /// Omit to match equal header names and duplicate occurrences automatically.
    #[serde(default)]
    pub mappings: Vec<DelimitedColumnMapping>,
    /// Explicit cleanup for source systems that append tab characters to fields.
    #[serde(default)]
    pub rstrip_tabs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDelimitedColumnMapping {
    pub source_column: u32,
    pub source_header: Option<String>,
    pub target_column: u32,
    pub target_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FillTemplateValidation {
    pub reopened: bool,
    pub verified_cells: usize,
    pub target_range: Option<CellRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FillTemplateResult {
    pub source: PathBuf,
    pub template: PathBuf,
    pub output: PathBuf,
    pub format: DelimitedFormat,
    pub records_read: u32,
    pub rows_written: u32,
    pub columns_written: usize,
    pub cells_written: usize,
    pub bytes_written: u64,
    pub mappings: Vec<ResolvedDelimitedColumnMapping>,
    pub duplicate_headers: Vec<DuplicateDelimitedHeader>,
    pub validation: FillTemplateValidation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetSheetValidation {
    pub sheet: String,
    /// Expected number of rows in the sheet's populated bounding range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rows: Option<u32>,
    #[serde(default)]
    pub header_row: u32,
    #[serde(default)]
    pub required_headers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidateWorkbookRequest {
    pub path: PathBuf,
    #[serde(default)]
    pub expected_sheets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_populated_cells: Option<u64>,
    #[serde(default)]
    pub sheets: Vec<SpreadsheetSheetValidation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetValidationCheck {
    pub check: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidateWorkbookResult {
    pub path: PathBuf,
    pub valid: bool,
    pub reopened: bool,
    pub sheet_count: usize,
    pub populated_cells: u64,
    pub checks: Vec<SpreadsheetValidationCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportDelimitedRequest {
    pub path: PathBuf,
    pub output: PathBuf,
    pub sheet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CellRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DelimitedFormat>,
    #[serde(default)]
    pub formula_mode: DelimitedFormulaMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportDelimitedResult {
    pub source: PathBuf,
    pub output: PathBuf,
    pub sheet: String,
    pub format: DelimitedFormat,
    pub range: CellRange,
    pub row_count: u32,
    pub column_count: u32,
    pub bytes_written: u64,
    pub validation_reopened: bool,
}

struct DelimitedScan {
    format: DelimitedFormat,
    record_count: u32,
    column_count: u32,
    headers: Vec<DelimitedHeader>,
    duplicate_headers: Vec<DuplicateDelimitedHeader>,
    ragged_row_count: u32,
    samples: Vec<Vec<String>>,
}

pub(super) fn inspect_delimited(
    request: &InspectDelimitedRequest,
) -> Result<InspectDelimitedResult, SpreadsheetError> {
    if request.sample_rows == 0 || request.sample_rows > MAX_DELIMITED_SAMPLE_ROWS {
        return Err(SpreadsheetError::InvalidDelimited {
            path: request.path.clone(),
            message: format!("sampleRows must be between 1 and {MAX_DELIMITED_SAMPLE_ROWS}"),
        });
    }
    let scan = scan_delimited(
        &request.path,
        request.format,
        request.header_row,
        request.sample_rows,
        request.rstrip_tabs,
    )?;
    let data_row_count = scan
        .record_count
        .saturating_sub(request.header_row.saturating_add(1));
    let result = InspectDelimitedResult {
        path: request.path.clone(),
        format: scan.format,
        record_count: scan.record_count,
        data_row_count,
        column_count: scan.column_count,
        headers: scan.headers,
        duplicate_headers: scan.duplicate_headers,
        ragged_row_count: scan.ragged_row_count,
        sample_rows: scan.samples,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub(super) fn fill_template(
    request: &FillTemplateRequest,
) -> Result<FillTemplateResult, SpreadsheetError> {
    super::validate_xlsx_path(&request.template)?;
    super::validate_xlsx_path(&request.output)?;
    let scan = scan_delimited(
        &request.source,
        request.source_format,
        request.source_header_row,
        1,
        request.rstrip_tabs,
    )?;
    let mut workbook = load_workbook(&request.template)?;
    let target_index = workbook
        .sheets
        .iter()
        .position(|sheet| sheet.name.eq_ignore_ascii_case(request.target_sheet.trim()))
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.target_sheet.clone(),
        })?;
    let target_headers =
        headers_from_sheet(&workbook.sheets[target_index], request.target_header_row);
    let mappings = resolve_mappings(
        &scan.headers,
        scan.column_count,
        &target_headers,
        &request.mappings,
    )?;
    let target_start_row = request
        .target_start_row
        .unwrap_or_else(|| request.target_header_row.saturating_add(1));
    if target_start_row <= request.target_header_row {
        return Err(SpreadsheetError::InvalidMapping {
            message: "targetStartRow must be after targetHeaderRow".to_string(),
        });
    }

    let format = scan.format;
    let mut reader = open_delimited_reader(&request.source, format)?;
    let mut updates = Vec::new();
    let mut rows_written = 0_u32;
    for (record_index, record) in reader.byte_records().enumerate() {
        let record_index =
            u32::try_from(record_index).map_err(|_| SpreadsheetError::TooManyCells {
                context: "delimited records",
                actual: usize::MAX,
                limit: MAX_WORKBOOK_CELLS,
            })?;
        if record_index <= request.source_header_row {
            continue;
        }
        let record = record.map_err(|error| invalid_delimited(&request.source, error))?;
        let row =
            target_start_row
                .checked_add(rows_written)
                .ok_or(SpreadsheetError::InvalidMapping {
                    message: "target row overflow".to_string(),
                })?;
        for mapping in &mappings {
            let address = CellAddress {
                row,
                column: mapping.target_column,
            };
            validate_address(address)?;
            let value = record
                .get(mapping.source_column as usize)
                .map(|field| delimited::decode_field(field, false, request.rstrip_tabs))
                .unwrap_or_default();
            let value = if value.is_empty() {
                SpreadsheetCellInput::Blank
            } else {
                validate_write_text(&value, &request.target_sheet, row, mapping.target_column)?;
                SpreadsheetCellInput::String(value)
            };
            updates.push(super::CellUpdate { address, value });
            if updates.len() > MAX_WORKBOOK_CELLS {
                return Err(SpreadsheetError::TooManyCells {
                    context: "delimited import",
                    actual: updates.len(),
                    limit: MAX_WORKBOOK_CELLS,
                });
            }
        }
        rows_written = rows_written
            .checked_add(1)
            .ok_or(SpreadsheetError::InvalidMapping {
                message: "too many source rows".to_string(),
            })?;
    }

    let sheet_request = SheetWriteRequest {
        name: workbook.sheets[target_index].name.clone(),
        visibility: None,
        cells: updates,
    };
    apply_sheet_updates(&mut workbook, std::slice::from_ref(&sheet_request))?;
    let output_cells = workbook
        .sheets
        .iter()
        .map(|sheet| sheet.cells.len())
        .sum::<usize>();
    ensure_workbook_cell_count(output_cells)?;

    let bytes = patch_workbook_template(
        &request.template,
        std::slice::from_ref(&sheet_request),
        &request.output,
    )?;
    let bytes_written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }
    fs::write(&request.output, &bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    verify_template_updates(&request.output, &sheet_request)?;

    let target_range = if rows_written == 0 || mappings.is_empty() {
        None
    } else {
        let min_column = mappings
            .iter()
            .map(|mapping| mapping.target_column)
            .min()
            .expect("mapping exists");
        let max_column = mappings
            .iter()
            .map(|mapping| mapping.target_column)
            .max()
            .expect("mapping exists");
        Some(CellRange {
            start: CellAddress {
                row: target_start_row,
                column: min_column,
            },
            end: CellAddress {
                row: target_start_row + rows_written - 1,
                column: max_column,
            },
        })
    };
    let result = FillTemplateResult {
        source: request.source.clone(),
        template: request.template.clone(),
        output: request.output.clone(),
        format,
        records_read: rows_written,
        rows_written,
        columns_written: mappings.len(),
        cells_written: sheet_request.cells.len(),
        bytes_written,
        mappings,
        duplicate_headers: scan.duplicate_headers,
        validation: FillTemplateValidation {
            reopened: true,
            verified_cells: sheet_request.cells.len(),
            target_range,
        },
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub(super) fn validate_workbook(
    request: &ValidateWorkbookRequest,
) -> Result<ValidateWorkbookResult, SpreadsheetError> {
    let inspected = inspect_workbook(&InspectWorkbookRequest {
        path: request.path.clone(),
    })?;
    let loaded = load_workbook(&request.path)?;
    let mut checks = Vec::new();
    let sheet_names = inspected
        .sheets
        .iter()
        .map(|sheet| sheet.sheet.name.clone())
        .collect::<Vec<_>>();
    for expected in &request.expected_sheets {
        let actual = sheet_names
            .iter()
            .any(|sheet| sheet.eq_ignore_ascii_case(expected));
        checks.push(SpreadsheetValidationCheck {
            check: format!("sheet:{expected}"),
            passed: actual,
            expected: "present".to_string(),
            actual: if actual { "present" } else { "missing" }.to_string(),
        });
    }
    if let Some(expected) = request.expected_populated_cells {
        checks.push(SpreadsheetValidationCheck {
            check: "populated_cells".to_string(),
            passed: inspected.populated_cells == expected,
            expected: expected.to_string(),
            actual: inspected.populated_cells.to_string(),
        });
    }
    for expected in &request.sheets {
        let Some(sheet) = loaded
            .sheets
            .iter()
            .find(|sheet| sheet.name.eq_ignore_ascii_case(&expected.sheet))
        else {
            checks.push(SpreadsheetValidationCheck {
                check: format!("sheet:{}", expected.sheet),
                passed: false,
                expected: "present".to_string(),
                actual: "missing".to_string(),
            });
            continue;
        };
        if let Some(expected_rows) = expected.expected_rows {
            let actual_rows = used_range(&sheet.cells)
                .and_then(CellRange::row_count)
                .and_then(|rows| u32::try_from(rows).ok())
                .unwrap_or(0);
            checks.push(SpreadsheetValidationCheck {
                check: format!("sheet:{}:used_rows", expected.sheet),
                passed: actual_rows == expected_rows,
                expected: expected_rows.to_string(),
                actual: actual_rows.to_string(),
            });
        }
        let headers = headers_from_sheet(sheet, expected.header_row);
        for required in &expected.required_headers {
            let present = headers
                .iter()
                .any(|header| normalize_header(&header.name) == normalize_header(required));
            checks.push(SpreadsheetValidationCheck {
                check: format!("sheet:{}:header:{required}", expected.sheet),
                passed: present,
                expected: "present".to_string(),
                actual: if present { "present" } else { "missing" }.to_string(),
            });
        }
    }
    let valid = checks.iter().all(|check| check.passed);
    let result = ValidateWorkbookResult {
        path: request.path.clone(),
        valid,
        reopened: true,
        sheet_count: inspected.sheets.len(),
        populated_cells: inspected.populated_cells,
        checks,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub(super) fn export_delimited(
    request: &ExportDelimitedRequest,
) -> Result<ExportDelimitedResult, SpreadsheetError> {
    super::validate_xlsx_path(&request.path)?;
    let format = DelimitedFormat::resolve(&request.output, request.format)?;
    let workbook = load_workbook(&request.path)?;
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(request.sheet.trim()))
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.sheet.clone(),
        })?;
    let range = request
        .range
        .or_else(|| used_range(&sheet.cells))
        .unwrap_or(CellRange {
            start: CellAddress { row: 0, column: 0 },
            end: CellAddress { row: 0, column: 0 },
        });
    validate_address(range.start)?;
    validate_address(range.end)?;
    let row_count = range.row_count().ok_or(SpreadsheetError::InvalidRange {
        reason: "range end precedes start",
    })?;
    let column_count = range.column_count().ok_or(SpreadsheetError::InvalidRange {
        reason: "range end precedes start",
    })?;
    let cell_count = row_count
        .checked_mul(column_count)
        .and_then(|cells| usize::try_from(cells).ok())
        .unwrap_or(usize::MAX);
    ensure_workbook_cell_count(cell_count)?;

    let mut writer = delimited::writer(Vec::new(), format.delimiter());
    for row in range.start.row..=range.end.row {
        let record = (range.start.column..=range.end.column)
            .map(|column| {
                sheet
                    .cells
                    .get(&CellAddress { row, column })
                    .map(|cell| export_cell(cell, request.formula_mode))
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        writer
            .write_record(&record)
            .map_err(|error| SpreadsheetError::WriteFailed {
                path: request.output.clone(),
                message: error.to_string(),
            })?;
    }
    writer.flush().map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    let bytes = writer
        .into_inner()
        .map_err(|error| SpreadsheetError::WriteFailed {
            path: request.output.clone(),
            message: error.error().to_string(),
        })?;
    let bytes_written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }
    fs::write(&request.output, &bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    verify_delimited_output(&request.output, format, row_count)?;
    let result = ExportDelimitedResult {
        source: request.path.clone(),
        output: request.output.clone(),
        sheet: sheet.name.clone(),
        format,
        range,
        row_count: u32::try_from(row_count).unwrap_or(u32::MAX),
        column_count: u32::try_from(column_count).unwrap_or(u32::MAX),
        bytes_written,
        validation_reopened: true,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

fn scan_delimited(
    path: &Path,
    requested_format: Option<DelimitedFormat>,
    header_row: u32,
    sample_rows: usize,
    rstrip_tabs: bool,
) -> Result<DelimitedScan, SpreadsheetError> {
    let format = DelimitedFormat::resolve(path, requested_format)?;
    let mut reader = open_delimited_reader(path, format)?;
    let mut record_count = 0_u32;
    let mut column_count = 0_u32;
    let mut header_values = None;
    let mut samples = Vec::new();
    let mut ragged_row_count = 0_u32;
    for (index, record) in reader.byte_records().enumerate() {
        let index = u32::try_from(index).map_err(|_| SpreadsheetError::TooManyCells {
            context: "delimited records",
            actual: usize::MAX,
            limit: MAX_WORKBOOK_CELLS,
        })?;
        let record = record.map_err(|error| invalid_delimited(path, error))?;
        record_count = record_count.saturating_add(1);
        if record_count > EXCEL_MAX_ROWS {
            return Err(SpreadsheetError::TooManyCells {
                context: "delimited rows",
                actual: record_count as usize,
                limit: EXCEL_MAX_ROWS as usize,
            });
        }
        let width = u32::try_from(record.len()).unwrap_or(u32::MAX);
        column_count = column_count.max(width);
        if column_count > EXCEL_MAX_COLUMNS {
            return Err(SpreadsheetError::TooManyCells {
                context: "delimited columns",
                actual: column_count as usize,
                limit: EXCEL_MAX_COLUMNS as usize,
            });
        }
        let values = record
            .iter()
            .enumerate()
            .map(|(column, field)| {
                delimited::decode_field(field, index == 0 && column == 0, rstrip_tabs)
            })
            .collect::<Vec<_>>();
        if index == header_row {
            header_values = Some(values);
        } else if index > header_row {
            if let Some(headers) = header_values.as_ref() {
                if values.len() != headers.len() {
                    ragged_row_count = ragged_row_count.saturating_add(1);
                }
            }
            if samples.len() < sample_rows {
                samples.push(values);
            }
        }
    }
    let header_values = header_values.ok_or_else(|| SpreadsheetError::InvalidDelimited {
        path: path.to_path_buf(),
        message: format!("header row {header_row} does not exist"),
    })?;
    let headers = describe_headers(&header_values);
    let duplicate_headers = duplicate_headers(&headers);
    Ok(DelimitedScan {
        format,
        record_count,
        column_count,
        headers,
        duplicate_headers,
        ragged_row_count,
        samples,
    })
}

fn open_delimited_reader(
    path: &Path,
    format: DelimitedFormat,
) -> Result<csv::Reader<File>, SpreadsheetError> {
    let metadata = fs::metadata(path).map_err(|source| SpreadsheetError::Io {
        operation: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_INPUT_FILE_BYTES {
        return Err(SpreadsheetError::FileTooLarge {
            path: path.to_path_buf(),
            actual_bytes: metadata.len(),
            limit_bytes: MAX_INPUT_FILE_BYTES,
        });
    }
    let file = File::open(path).map_err(|source| SpreadsheetError::Io {
        operation: "open",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(delimited::byte_reader(file, format.delimiter()))
}

fn invalid_delimited(path: &Path, error: csv::Error) -> SpreadsheetError {
    SpreadsheetError::InvalidDelimited {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn describe_headers(values: &[String]) -> Vec<DelimitedHeader> {
    let mut occurrences = BTreeMap::<String, u32>::new();
    values
        .iter()
        .enumerate()
        .map(|(column, name)| {
            let occurrence = occurrences.entry(normalize_header(name)).or_default();
            *occurrence += 1;
            DelimitedHeader {
                name: name.clone(),
                column: u32::try_from(column).unwrap_or(u32::MAX),
                occurrence: *occurrence,
            }
        })
        .collect()
}

fn duplicate_headers(headers: &[DelimitedHeader]) -> Vec<DuplicateDelimitedHeader> {
    let mut groups = BTreeMap::<String, Vec<&DelimitedHeader>>::new();
    for header in headers {
        if !header.name.trim().is_empty() {
            groups
                .entry(normalize_header(&header.name))
                .or_default()
                .push(header);
        }
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1)
        .map(|group| DuplicateDelimitedHeader {
            name: group[0].name.clone(),
            occurrences: u32::try_from(group.len()).unwrap_or(u32::MAX),
            columns: group.iter().map(|header| header.column).collect(),
        })
        .collect()
}

fn headers_from_sheet(sheet: &super::LoadedSheet, row: u32) -> Vec<DelimitedHeader> {
    let values = sheet
        .cells
        .iter()
        .filter(|(address, _)| address.row == row)
        .map(|(address, cell)| (address.column, stored_cell_value(cell)))
        .filter(|(_, value)| !value.is_empty())
        .collect::<Vec<_>>();
    let mut occurrences = BTreeMap::<String, u32>::new();
    values
        .into_iter()
        .map(|(column, name)| {
            let occurrence = occurrences.entry(normalize_header(&name)).or_default();
            *occurrence += 1;
            DelimitedHeader {
                name,
                column,
                occurrence: *occurrence,
            }
        })
        .collect()
}

fn resolve_mappings(
    source_headers: &[DelimitedHeader],
    source_columns: u32,
    target_headers: &[DelimitedHeader],
    requested: &[DelimitedColumnMapping],
) -> Result<Vec<ResolvedDelimitedColumnMapping>, SpreadsheetError> {
    let mut resolved = if requested.is_empty() {
        source_headers
            .iter()
            .filter(|source| !source.name.trim().is_empty())
            .filter_map(|source| {
                target_headers
                    .iter()
                    .find(|target| {
                        normalize_header(&target.name) == normalize_header(&source.name)
                            && target.occurrence == source.occurrence
                    })
                    .map(|target| resolved_mapping(source, target))
            })
            .collect::<Vec<_>>()
    } else {
        requested
            .iter()
            .map(|mapping| {
                let source =
                    resolve_selector(&mapping.source, source_headers, source_columns, "source")?;
                let target =
                    resolve_selector(&mapping.target, target_headers, EXCEL_MAX_COLUMNS, "target")?;
                Ok(resolved_mapping(&source, &target))
            })
            .collect::<Result<Vec<_>, SpreadsheetError>>()?
    };
    if resolved.is_empty() {
        return Err(SpreadsheetError::InvalidMapping {
            message: format!(
                "no source headers matched target headers; provide mappings using header occurrence or zero-based index (source: [{}], target: [{}])",
                header_summary(source_headers),
                header_summary(target_headers)
            ),
        });
    }
    let mut target_columns = BTreeSet::new();
    for mapping in &resolved {
        if !target_columns.insert(mapping.target_column) {
            return Err(SpreadsheetError::InvalidMapping {
                message: format!(
                    "target column {} is mapped more than once",
                    mapping.target_column
                ),
            });
        }
    }
    resolved.sort_by_key(|mapping| mapping.target_column);
    Ok(resolved)
}

fn resolve_selector(
    selector: &DelimitedColumnSelector,
    headers: &[DelimitedHeader],
    column_limit: u32,
    side: &str,
) -> Result<DelimitedHeader, SpreadsheetError> {
    match selector {
        DelimitedColumnSelector::Index { index } => {
            if *index >= column_limit {
                return Err(SpreadsheetError::InvalidMapping {
                    message: format!(
                        "{side} column index {index} is outside 0..{}",
                        column_limit.saturating_sub(1)
                    ),
                });
            }
            Ok(headers
                .iter()
                .find(|header| header.column == *index)
                .cloned()
                .unwrap_or(DelimitedHeader {
                    name: String::new(),
                    column: *index,
                    occurrence: 1,
                }))
        }
        DelimitedColumnSelector::Header { name, occurrence } => {
            if name.trim().is_empty() || *occurrence == 0 {
                return Err(SpreadsheetError::InvalidMapping {
                    message: format!("{side} header name must be non-empty and occurrence >= 1"),
                });
            }
            headers
                .iter()
                .find(|header| {
                    normalize_header(&header.name) == normalize_header(name)
                        && header.occurrence == *occurrence
                })
                .cloned()
                .ok_or_else(|| SpreadsheetError::InvalidMapping {
                    message: format!(
                        "{side} header {name:?} occurrence {occurrence} was not found; available: [{}]",
                        header_summary(headers)
                    ),
                })
        }
    }
}

fn resolved_mapping(
    source: &DelimitedHeader,
    target: &DelimitedHeader,
) -> ResolvedDelimitedColumnMapping {
    ResolvedDelimitedColumnMapping {
        source_column: source.column,
        source_header: (!source.name.is_empty()).then(|| source.name.clone()),
        target_column: target.column,
        target_header: (!target.name.is_empty()).then(|| target.name.clone()),
    }
}

fn normalize_header(value: &str) -> String {
    value.trim().to_lowercase()
}

fn header_summary(headers: &[DelimitedHeader]) -> String {
    headers
        .iter()
        .take(32)
        .map(|header| {
            if header.occurrence > 1 {
                format!("{}#{}@{}", header.name, header.occurrence, header.column)
            } else {
                format!("{}@{}", header.name, header.column)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn verify_template_updates(
    output: &Path,
    request: &SheetWriteRequest,
) -> Result<(), SpreadsheetError> {
    let workbook = load_workbook(output)?;
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(&request.name))
        .ok_or_else(|| SpreadsheetError::ValidationFailed {
            message: format!("output sheet {:?} was not found", request.name),
        })?;
    for update in &request.cells {
        let actual = sheet.cells.get(&update.address);
        let matches = match (&update.value, actual) {
            (SpreadsheetCellInput::Blank, None) => true,
            (SpreadsheetCellInput::Blank, Some(cell)) => {
                matches!(cell.value, SpreadsheetCellValue::Empty) && cell.formula.is_none()
            }
            (SpreadsheetCellInput::String(expected), Some(cell)) => {
                cell.formula.is_none()
                    && matches!(&cell.value, SpreadsheetCellValue::String(actual) if actual == expected)
            }
            _ => false,
        };
        if !matches {
            return Err(SpreadsheetError::ValidationFailed {
                message: format!(
                    "output did not preserve imported value at {}!R{}C{}",
                    request.name, update.address.row, update.address.column
                ),
            });
        }
    }
    Ok(())
}

fn verify_delimited_output(
    output: &Path,
    format: DelimitedFormat,
    expected_rows: u64,
) -> Result<(), SpreadsheetError> {
    let mut reader = open_delimited_reader(output, format)?;
    let actual_rows = reader.byte_records().try_fold(0_u64, |count, record| {
        record
            .map(|_| count + 1)
            .map_err(|error| invalid_delimited(output, error))
    })?;
    if actual_rows != expected_rows {
        return Err(SpreadsheetError::ValidationFailed {
            message: format!(
                "exported delimited file reopened with {actual_rows} rows; expected {expected_rows}"
            ),
        });
    }
    Ok(())
}

fn used_range(cells: &BTreeMap<CellAddress, StoredCell>) -> Option<CellRange> {
    let min_row = cells.keys().map(|address| address.row).min()?;
    let max_row = cells.keys().map(|address| address.row).max()?;
    let min_column = cells.keys().map(|address| address.column).min()?;
    let max_column = cells.keys().map(|address| address.column).max()?;
    Some(CellRange {
        start: CellAddress {
            row: min_row,
            column: min_column,
        },
        end: CellAddress {
            row: max_row,
            column: max_column,
        },
    })
}

fn export_cell(cell: &StoredCell, formula_mode: DelimitedFormulaMode) -> String {
    if formula_mode == DelimitedFormulaMode::Formulas {
        if let Some(formula) = &cell.formula {
            return formula.clone();
        }
    }
    stored_cell_value(cell)
}

fn stored_cell_value(cell: &StoredCell) -> String {
    match &cell.value {
        SpreadsheetCellValue::Empty => cell.formula_result.clone().unwrap_or_default(),
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => value.clone(),
        SpreadsheetCellValue::Integer(value) => value.to_string(),
        SpreadsheetCellValue::Number(value) => value.to_string(),
        SpreadsheetCellValue::Boolean(value) => if *value { "TRUE" } else { "FALSE" }.to_string(),
        SpreadsheetCellValue::DateTime(value) => value.serial.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spreadsheet::{write_workbook, CellUpdate, WriteWorkbookRequest};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "opentopia-spreadsheet-transfer-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self, file_name: &str) -> PathBuf {
            self.0.join(file_name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_template(path: &Path) {
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: path.to_path_buf(),
            sheets: vec![SheetWriteRequest {
                name: "Data".to_string(),
                visibility: None,
                cells: vec![
                    CellUpdate {
                        address: CellAddress { row: 0, column: 0 },
                        value: SpreadsheetCellInput::String("name".to_string()),
                    },
                    CellUpdate {
                        address: CellAddress { row: 0, column: 1 },
                        value: SpreadsheetCellInput::String("name".to_string()),
                    },
                    CellUpdate {
                        address: CellAddress { row: 0, column: 2 },
                        value: SpreadsheetCellInput::String("note".to_string()),
                    },
                ],
            }],
        })
        .expect("write template");
    }

    #[test]
    fn fill_template_handles_duplicate_headers_and_quoted_newlines() {
        let temp = TestDirectory::new();
        let source = temp.path("source.csv");
        let template = temp.path("template.xlsx");
        let output = temp.path("output.xlsx");
        fs::write(
            &source,
            "name,name,note\nalpha,beta,\"comma, and\nnewline\"\n",
        )
        .expect("write source");
        write_template(&template);

        let result = fill_template(&FillTemplateRequest {
            source,
            source_format: None,
            template,
            output: output.clone(),
            target_sheet: "Data".to_string(),
            source_header_row: 0,
            target_header_row: 0,
            target_start_row: None,
            mappings: Vec::new(),
            rstrip_tabs: false,
        })
        .expect("fill template");

        assert_eq!(result.rows_written, 1);
        assert_eq!(result.columns_written, 3);
        assert_eq!(result.duplicate_headers[0].columns, vec![0, 1]);
        assert!(result.validation.reopened);
        let loaded = load_workbook(&output).expect("reopen output");
        let sheet = &loaded.sheets[0];
        assert_eq!(
            stored_cell_value(
                sheet
                    .cells
                    .get(&CellAddress { row: 1, column: 2 })
                    .expect("note")
            ),
            "comma, and\nnewline"
        );
    }

    #[test]
    fn export_delimited_round_trips_complex_values() {
        let temp = TestDirectory::new();
        let source = temp.path("source.xlsx");
        let output = temp.path("output.csv");
        write_template(&source);
        let result = export_delimited(&ExportDelimitedRequest {
            path: source,
            output: output.clone(),
            sheet: "Data".to_string(),
            range: None,
            format: None,
            formula_mode: DelimitedFormulaMode::Values,
        })
        .expect("export csv");

        assert_eq!(result.row_count, 1);
        assert!(result.validation_reopened);
        assert_eq!(
            fs::read_to_string(output).expect("read csv"),
            "name,name,note\n"
        );
    }

    #[test]
    fn validation_reports_assertion_failures_without_using_another_format_tool() {
        let temp = TestDirectory::new();
        let workbook = temp.path("template.xlsx");
        write_template(&workbook);
        let result = validate_workbook(&ValidateWorkbookRequest {
            path: workbook,
            expected_sheets: vec!["Data".to_string(), "Missing".to_string()],
            expected_populated_cells: Some(3),
            sheets: vec![SpreadsheetSheetValidation {
                sheet: "Data".to_string(),
                expected_rows: Some(1),
                header_row: 0,
                required_headers: vec!["note".to_string()],
            }],
        })
        .expect("validate workbook");

        assert!(!result.valid);
        assert!(result.reopened);
        assert!(result.checks.iter().any(|check| !check.passed));
    }
}
