use super::display::number_format_codes;
use super::{
    ensure_workbook_cell_count, inspect_workbook, load_workbook, validate_address, CellAddress,
    CellRange, InspectWorkbookRequest, SpreadsheetCellValue, SpreadsheetError, StoredCell,
    EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS, MAX_INPUT_FILE_BYTES, MAX_OUTPUT_FILE_BYTES,
    MAX_WORKBOOK_CELLS,
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
pub struct SpreadsheetSheetValidation {
    pub sheet: String,
    /// Expected number of rows in the sheet's populated bounding range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rows: Option<u32>,
    /// Expected number of populated data rows. If a header is configured, only
    /// rows strictly after it count; otherwise every populated row counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_data_rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_row: Option<u32>,
    #[serde(default)]
    pub required_headers: Vec<String>,
    #[serde(default)]
    pub ranges: Vec<SpreadsheetRangeValidation>,
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetExpectedCellType {
    Empty,
    String,
    Number,
    Boolean,
    DateTime,
    Formula,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetRangeValidation {
    pub range: CellRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_type: Option<SpreadsheetExpectedCellType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_number_format: Option<String>,
    #[serde(default)]
    pub allow_blank: bool,
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
    pub validation_passed: bool,
    pub reopened: bool,
    pub sheet_count: usize,
    pub populated_cells: u64,
    pub sheet_metrics: Vec<SpreadsheetSheetValidationResult>,
    pub checks: Vec<SpreadsheetValidationCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSheetValidationResult {
    pub sheet: String,
    pub present: bool,
    pub used_rows: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_row: Option<u32>,
    pub data_rows: u32,
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
    let mut sheet_metrics = Vec::new();
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
            sheet_metrics.push(SpreadsheetSheetValidationResult {
                sheet: expected.sheet.clone(),
                present: false,
                used_rows: 0,
                header_row: expected.header_row,
                data_rows: 0,
            });
            continue;
        };
        let used_rows = used_range(&sheet.cells)
            .and_then(CellRange::row_count)
            .and_then(|rows| u32::try_from(rows).ok())
            .unwrap_or(0);
        let data_rows = u32::try_from(
            sheet
                .cells
                .keys()
                .filter_map(|address| {
                    expected
                        .header_row
                        .is_none_or(|header_row| address.row > header_row)
                        .then_some(address.row)
                })
                .collect::<BTreeSet<_>>()
                .len(),
        )
        .unwrap_or(u32::MAX);
        sheet_metrics.push(SpreadsheetSheetValidationResult {
            sheet: sheet.name.clone(),
            present: true,
            used_rows,
            header_row: expected.header_row,
            data_rows,
        });
        if let Some(expected_rows) = expected.expected_rows {
            checks.push(SpreadsheetValidationCheck {
                check: format!("sheet:{}:used_rows", expected.sheet),
                passed: used_rows == expected_rows,
                expected: expected_rows.to_string(),
                actual: used_rows.to_string(),
            });
        }
        if let Some(expected_data_rows) = expected.expected_data_rows {
            checks.push(SpreadsheetValidationCheck {
                check: format!("sheet:{}:data_rows", expected.sheet),
                passed: data_rows == expected_data_rows,
                expected: expected_data_rows.to_string(),
                actual: data_rows.to_string(),
            });
        }
        if let Some(header_row) = expected.header_row {
            let headers = headers_from_sheet(sheet, header_row);
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
        for range_validation in &expected.ranges {
            validate_address(range_validation.range.start)?;
            validate_address(range_validation.range.end)?;
            let Some(_) = range_validation.range.cell_count() else {
                return Err(SpreadsheetError::InvalidRange {
                    reason: "validation range end precedes start",
                });
            };
            let formats = if range_validation.expected_number_format.is_some() {
                number_format_codes(&request.path, &sheet.name, range_validation.range)?
            } else {
                Default::default()
            };
            let mut checked = 0u64;
            let mut type_matches = 0u64;
            let mut format_matches = 0u64;
            let mut actual_types = BTreeSet::new();
            let mut actual_formats = BTreeSet::new();
            for row in range_validation.range.start.row..=range_validation.range.end.row {
                for column in
                    range_validation.range.start.column..=range_validation.range.end.column
                {
                    let address = CellAddress { row, column };
                    let cell = sheet.cells.get(&address);
                    let is_blank = cell.is_none_or(|cell| {
                        matches!(cell.value, SpreadsheetCellValue::Empty) && cell.formula.is_none()
                    });
                    if is_blank && range_validation.allow_blank {
                        continue;
                    }
                    checked += 1;
                    if let Some(expected_type) = range_validation.expected_type {
                        let actual_type = validation_cell_type(cell);
                        actual_types.insert(actual_type);
                        if actual_type == expected_type {
                            type_matches += 1;
                        }
                    }
                    if let Some(expected_format) =
                        range_validation.expected_number_format.as_deref()
                    {
                        let actual_format = formats
                            .get(&(row, column))
                            .map(String::as_str)
                            .unwrap_or("General");
                        actual_formats.insert(actual_format.to_string());
                        if actual_format
                            .trim()
                            .eq_ignore_ascii_case(expected_format.trim())
                        {
                            format_matches += 1;
                        }
                    }
                }
            }
            let range_name = format!(
                "R{}C{}:R{}C{}",
                range_validation.range.start.row,
                range_validation.range.start.column,
                range_validation.range.end.row,
                range_validation.range.end.column
            );
            if let Some(expected_type) = range_validation.expected_type {
                checks.push(SpreadsheetValidationCheck {
                    check: format!("sheet:{}:range:{range_name}:type", expected.sheet),
                    passed: type_matches == checked,
                    expected: format!("{expected_type:?}"),
                    actual: format!(
                        "{type_matches}/{checked} cells; types={}",
                        actual_types
                            .iter()
                            .map(|value| format!("{value:?}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                });
            }
            if let Some(expected_format) = &range_validation.expected_number_format {
                checks.push(SpreadsheetValidationCheck {
                    check: format!("sheet:{}:range:{range_name}:number_format", expected.sheet),
                    passed: format_matches == checked,
                    expected: expected_format.clone(),
                    actual: format!(
                        "{format_matches}/{checked} cells; formats={}",
                        actual_formats.into_iter().collect::<Vec<_>>().join(",")
                    ),
                });
            }
        }
    }
    let validation_passed = checks.iter().all(|check| check.passed);
    let result = ValidateWorkbookResult {
        path: request.path.clone(),
        validation_passed,
        reopened: true,
        sheet_count: inspected.sheets.len(),
        populated_cells: inspected.populated_cells,
        sheet_metrics,
        checks,
    };
    Ok(result)
}

fn validation_cell_type(cell: Option<&StoredCell>) -> SpreadsheetExpectedCellType {
    let Some(cell) = cell else {
        return SpreadsheetExpectedCellType::Empty;
    };
    if cell.formula.is_some() {
        return SpreadsheetExpectedCellType::Formula;
    }
    match cell.value {
        SpreadsheetCellValue::Integer(_) | SpreadsheetCellValue::Number(_) => {
            SpreadsheetExpectedCellType::Number
        }
        SpreadsheetCellValue::Boolean(_) => SpreadsheetExpectedCellType::Boolean,
        SpreadsheetCellValue::DateTime(_) | SpreadsheetCellValue::DateTimeIso(_) => {
            SpreadsheetExpectedCellType::DateTime
        }
        SpreadsheetCellValue::Empty => SpreadsheetExpectedCellType::Empty,
        SpreadsheetCellValue::String(_)
        | SpreadsheetCellValue::DurationIso(_)
        | SpreadsheetCellValue::Error(_) => SpreadsheetExpectedCellType::String,
    }
}

pub(super) fn export_delimited(
    request: &ExportDelimitedRequest,
) -> Result<ExportDelimitedResult, SpreadsheetError> {
    super::validate_workbook_path(&request.path)?;
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

pub(super) fn headers_from_sheet(sheet: &super::LoadedSheet, row: u32) -> Vec<DelimitedHeader> {
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

fn normalize_header(value: &str) -> String {
    value.trim().to_lowercase()
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

pub(super) fn stored_cell_value(cell: &StoredCell) -> String {
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
    use crate::spreadsheet::{
        write_workbook, CellUpdate, SheetWriteRequest, SpreadsheetCellInput, WriteWorkbookRequest,
    };

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
                        style_from: None,
                    },
                    CellUpdate {
                        address: CellAddress { row: 0, column: 1 },
                        value: SpreadsheetCellInput::String("name".to_string()),
                        style_from: None,
                    },
                    CellUpdate {
                        address: CellAddress { row: 0, column: 2 },
                        value: SpreadsheetCellInput::String("note".to_string()),
                        style_from: None,
                    },
                ],
            }],
        })
        .expect("write template");
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
                expected_data_rows: Some(0),
                header_row: Some(0),
                required_headers: vec!["note".to_string()],
                ranges: Vec::new(),
            }],
        })
        .expect("validate workbook");

        assert!(!result.validation_passed);
        assert!(result.reopened);
        assert_eq!(result.sheet_metrics[0].used_rows, 1);
        assert_eq!(result.sheet_metrics[0].data_rows, 0);
        assert!(result.checks.iter().any(|check| !check.passed));
    }
}
