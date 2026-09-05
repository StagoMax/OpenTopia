use crate::office_runtime::OfficeRuntime;
use calamine::{
    open_workbook_auto, CellType, Data, Range, Reader, SheetType as CalamineSheetType,
    SheetVisible as CalamineSheetVisible, Sheets,
};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event as XmlEvent};
use quick_xml::{Reader as XmlReader, Writer as XmlWriter};
use rust_xlsxwriter::{Formula, Workbook, Worksheet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

mod display;
mod format;
mod guidance;
mod ooxml;
mod read;
mod structure;
mod template_patch;
mod transfer;
mod value_transform;
mod workbook_write;

pub use format::SpreadsheetFileFormat;
pub use read::{filter_rows, find_cells, inspect_workbook, list_sheets, read_range, read_ranges};
pub(crate) use read::{read_range_for_display, read_ranges_for_mutation};
pub use structure::*;
pub use transfer::*;
pub use value_transform::*;

use template_patch::patch_workbook_template;
use workbook_write::{
    apply_sheet_updates, load_spreadsheet, load_workbook, render_workbook,
    write_delimited_workbook, write_failed, LoadedSheet, LoadedWorkbook, StoredCell,
};

pub const EXCEL_MAX_ROWS: u32 = 1_048_576;
pub const EXCEL_MAX_COLUMNS: u32 = 16_384;
pub const MAX_INPUT_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_OUTPUT_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_SHEETS: usize = 256;
pub const MAX_READ_ROWS: u64 = 1_000;
pub const MAX_READ_COLUMNS: u64 = 256;
pub const MAX_READ_CELLS: u64 = 10_000;
pub const MAX_READ_RANGES: usize = 64;
pub const MAX_WRITE_UPDATES: usize = 10_000;
pub const MAX_WORKBOOK_CELLS: usize = 250_000;
pub const MAX_CELL_CHARACTERS: usize = 32_767;
pub const MAX_CELL_TEXT_BYTES: usize = 128 * 1024;
pub const MAX_FORMULA_BYTES: usize = 8_192;
pub const MAX_FIND_RESULTS: usize = 1_000;
pub const MAX_FILTER_CONDITIONS: usize = 32;
pub const MAX_FILTER_RESULTS: usize = 2_000;

const MAX_EXCEL_INTEGER: i64 = 999_999_999_999_999;
const OPENPYXL_WORKER_TIMEOUT: Duration = Duration::from_secs(60);
const OPENPYXL_WORKER: &str = include_str!("spreadsheet_openpyxl_worker.py");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetRequest {
    pub action: SpreadsheetAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "request", rename_all = "snake_case")]
pub enum SpreadsheetAction {
    InspectDelimited(InspectDelimitedRequest),
    InspectWorkbook(InspectWorkbookRequest),
    ListSheets(ListSheetsRequest),
    ReadRange(ReadRangeRequest),
    ReadRanges(ReadRangesRequest),
    FindCells(FindCellsRequest),
    FilterRows(FilterRowsRequest),
    ValidateWorkbook(ValidateWorkbookRequest),
    ExportDelimited(ExportDelimitedRequest),
    WriteWorkbook(WriteWorkbookRequest),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetActionKind {
    InspectDelimited,
    InspectWorkbook,
    ListSheets,
    ReadRange,
    ReadRanges,
    FindCells,
    FilterRows,
    ValidateWorkbook,
    ExportDelimited,
    WriteWorkbook,
}

impl SpreadsheetAction {
    pub fn kind(&self) -> SpreadsheetActionKind {
        match self {
            Self::InspectDelimited(_) => SpreadsheetActionKind::InspectDelimited,
            Self::InspectWorkbook(_) => SpreadsheetActionKind::InspectWorkbook,
            Self::ListSheets(_) => SpreadsheetActionKind::ListSheets,
            Self::ReadRange(_) => SpreadsheetActionKind::ReadRange,
            Self::ReadRanges(_) => SpreadsheetActionKind::ReadRanges,
            Self::FindCells(_) => SpreadsheetActionKind::FindCells,
            Self::FilterRows(_) => SpreadsheetActionKind::FilterRows,
            Self::ValidateWorkbook(_) => SpreadsheetActionKind::ValidateWorkbook,
            Self::ExportDelimited(_) => SpreadsheetActionKind::ExportDelimited,
            Self::WriteWorkbook(_) => SpreadsheetActionKind::WriteWorkbook,
        }
    }
}

impl SpreadsheetActionKind {
    pub fn is_mutation(self) -> bool {
        matches!(self, Self::ExportDelimited | Self::WriteWorkbook)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InspectWorkbookRequest {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListSheetsRequest {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadRangeRequest {
    pub path: PathBuf,
    pub sheet: String,
    pub range: CellRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetRangeRequest {
    pub sheet: String,
    pub range: CellRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadRangesRequest {
    pub path: PathBuf,
    #[serde(default)]
    pub ranges: Vec<SheetRangeRequest>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetTextMatchMode {
    #[default]
    Contains,
    Exact,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FindCellsRequest {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<CellRange>,
    pub query: String,
    #[serde(default)]
    pub match_mode: SpreadsheetTextMatchMode,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub include_formulas: bool,
    #[serde(default = "default_find_results")]
    pub max_results: usize,
}

fn default_find_results() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SpreadsheetFilterValue {
    String(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetFilterOperator {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    IsBlank,
    IsNotBlank,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpreadsheetFilterCondition {
    /// Absolute zero-based worksheet column.
    #[schemars(range(max = 16383))]
    pub column: u32,
    pub operator: SpreadsheetFilterOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<SpreadsheetFilterValue>,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetFilterMatchMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetFilterReturnMode {
    /// Return only the exact number of matching rows.
    #[default]
    Summary,
    /// Return matching worksheet row indices, bounded by `max_results`.
    Indices,
    /// Return matching worksheet row indices and cell values, bounded by `max_results`.
    Rows,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilterRowsRequest {
    pub path: PathBuf,
    pub sheet: String,
    pub range: CellRange,
    #[serde(default)]
    pub conditions: Vec<SpreadsheetFilterCondition>,
    #[serde(default)]
    pub match_mode: SpreadsheetFilterMatchMode,
    #[serde(default)]
    pub return_mode: SpreadsheetFilterReturnMode,
    #[serde(default = "default_filter_results")]
    pub max_results: usize,
}

fn default_filter_results() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkbookRequest {
    /// An optional XLSX source. When all target sheets already exist and visibility is
    /// unchanged, the package is patched so untouched styles, charts, tables, images,
    /// macros, and workbook objects are preserved. Structural changes rebuild it.
    pub source: Option<PathBuf>,
    pub output: PathBuf,
    #[serde(default)]
    pub sheets: Vec<SheetWriteRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SheetWriteRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<SheetVisibility>,
    #[serde(default)]
    #[schemars(length(max = 10000))]
    pub cells: Vec<CellUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellUpdate {
    pub address: CellAddress,
    pub value: SpreadsheetCellInput,
    /// Optional template cell whose style should be reused when the target cell has no style.
    /// This is an internal mutation hint and is intentionally absent from public tool schemas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(skip)]
    pub(crate) style_from: Option<CellAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SpreadsheetCellInput {
    Blank,
    String(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Formula(FormulaInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct FormulaInput {
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_result: Option<String>,
}

/// Zero-based row and column coordinates.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellAddress {
    #[schemars(range(max = 1048575))]
    pub row: u32,
    #[schemars(range(max = 16383))]
    pub column: u32,
}

/// An inclusive range using zero-based coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CellRange {
    pub start: CellAddress,
    pub end: CellAddress,
}

impl CellRange {
    pub fn row_count(self) -> Option<u64> {
        (self.end.row >= self.start.row).then(|| u64::from(self.end.row - self.start.row) + 1)
    }

    pub fn column_count(self) -> Option<u64> {
        (self.end.column >= self.start.column)
            .then(|| u64::from(self.end.column - self.start.column) + 1)
    }

    pub fn cell_count(self) -> Option<u64> {
        self.row_count()?.checked_mul(self.column_count()?)
    }
}

pub fn parse_a1_address(value: &str) -> Result<CellAddress, SpreadsheetError> {
    let value = value.trim();
    let value = value.strip_prefix('$').unwrap_or(value);
    let mut column = 0_u32;
    let mut letters = 0_usize;
    let mut split = 0_usize;
    for (index, character) in value.char_indices() {
        if character.is_ascii_alphabetic() {
            column = column
                .checked_mul(26)
                .and_then(|value| {
                    value.checked_add(u32::from(character.to_ascii_uppercase() as u8 - b'A') + 1)
                })
                .ok_or(SpreadsheetError::InvalidRange {
                    reason: "A1 column is outside spreadsheet bounds",
                })?;
            letters += 1;
            split = index + character.len_utf8();
        } else {
            break;
        }
    }
    if letters == 0 {
        return Err(SpreadsheetError::InvalidRange {
            reason: "A1 address must start with a column name",
        });
    }
    let row_text = value[split..].strip_prefix('$').unwrap_or(&value[split..]);
    if row_text.is_empty() || !row_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SpreadsheetError::InvalidRange {
            reason: "A1 address must end with a row number",
        });
    }
    let row = row_text
        .parse::<u32>()
        .ok()
        .and_then(|row| row.checked_sub(1))
        .ok_or(SpreadsheetError::InvalidRange {
            reason: "A1 row must be a positive integer",
        })?;
    let address = CellAddress {
        row,
        column: column.checked_sub(1).expect("validated A1 column"),
    };
    validate_address(address)?;
    Ok(address)
}

pub fn parse_a1_range(value: &str) -> Result<CellRange, SpreadsheetError> {
    let value = value.trim();
    let (start, end) = value.split_once(':').unwrap_or((value, value));
    if end.contains(':') {
        return Err(SpreadsheetError::InvalidRange {
            reason: "A1 range must contain at most one colon",
        });
    }
    let range = CellRange {
        start: parse_a1_address(start)?,
        end: parse_a1_address(end)?,
    };
    validate_range_dimensions(range)?;
    Ok(range)
}

pub fn format_a1_address(address: CellAddress) -> String {
    let mut column = address.column + 1;
    let mut letters = Vec::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        letters.push(char::from(
            b'A' + u8::try_from(remainder).expect("A1 remainder"),
        ));
        column = (column - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        letters.into_iter().collect::<String>(),
        address.row + 1
    )
}

pub fn format_a1_range(range: CellRange) -> String {
    let start = format_a1_address(range.start);
    let end = format_a1_address(range.end);
    if start == end {
        start
    } else {
        format!("{start}:{end}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "result", rename_all = "snake_case")]
pub enum SpreadsheetResult {
    DelimitedInspected(InspectDelimitedResult),
    WorkbookInspected(InspectWorkbookResult),
    SheetsListed(ListSheetsResult),
    RangeRead(ReadRangeResult),
    RangesRead(ReadRangesResult),
    CellsFound(FindCellsResult),
    RowsFiltered(FilterRowsResult),
    WorkbookValidated(ValidateWorkbookResult),
    DelimitedExported(ExportDelimitedResult),
    WorkbookWritten(WriteWorkbookResult),
}

impl SpreadsheetResult {
    pub fn kind(&self) -> SpreadsheetActionKind {
        match self {
            Self::DelimitedInspected(_) => SpreadsheetActionKind::InspectDelimited,
            Self::WorkbookInspected(_) => SpreadsheetActionKind::InspectWorkbook,
            Self::SheetsListed(_) => SpreadsheetActionKind::ListSheets,
            Self::RangeRead(_) => SpreadsheetActionKind::ReadRange,
            Self::RangesRead(_) => SpreadsheetActionKind::ReadRanges,
            Self::CellsFound(_) => SpreadsheetActionKind::FindCells,
            Self::RowsFiltered(_) => SpreadsheetActionKind::FilterRows,
            Self::WorkbookValidated(_) => SpreadsheetActionKind::ValidateWorkbook,
            Self::DelimitedExported(_) => SpreadsheetActionKind::ExportDelimited,
            Self::WorkbookWritten(_) => SpreadsheetActionKind::WriteWorkbook,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ListSheetsResult {
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub sheets: Vec<SheetInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InspectWorkbookResult {
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub sheets: Vec<SheetInspection>,
    pub populated_cells: u64,
    pub guidance: WorkbookGuidance,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookGuidance {
    pub data_validations: Vec<SpreadsheetDataValidation>,
    pub comments: Vec<SpreadsheetCellComment>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetDataValidation {
    pub sheet: String,
    pub ranges: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_blank: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetCellComment {
    pub sheet: String,
    pub cell: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SheetInspection {
    pub sheet: SheetInfo,
    pub used_range: Option<CellRange>,
    pub populated_cells: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SheetInfo {
    pub name: String,
    pub kind: SheetKind,
    pub visibility: SheetVisibility,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SheetKind {
    Worksheet,
    DialogSheet,
    MacroSheet,
    ChartSheet,
    Vba,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SheetVisibility {
    #[default]
    Visible,
    Hidden,
    VeryHidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadRangeResult {
    pub path: PathBuf,
    pub sheet: String,
    pub range: CellRange,
    pub rows: Vec<Vec<SpreadsheetCell>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadRangesResult {
    pub path: PathBuf,
    pub ranges: Vec<ReadRangeResult>,
    pub total_cells: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetCellMatch {
    pub sheet: String,
    pub address: CellAddress,
    pub cell: SpreadsheetCell,
    pub matched_formula: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FindCellsResult {
    pub path: PathBuf,
    pub matches: Vec<SpreadsheetCellMatch>,
    pub scanned_cells: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FilterRowsResult {
    pub path: PathBuf,
    pub sheet: String,
    pub range: CellRange,
    pub matched_row_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_row_indices: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Vec<SpreadsheetCell>>,
    pub scanned_rows: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetCell {
    pub value: SpreadsheetCellValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SpreadsheetCellValue {
    Empty,
    String(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    DateTime(ExcelDateTimeValue),
    DateTimeIso(String),
    DurationIso(String),
    Error(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExcelDateTimeValue {
    pub serial: f64,
    pub is_duration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WriteWorkbookResult {
    pub output: PathBuf,
    pub bytes_written: u64,
    pub sheet_count: usize,
    pub output_cells: usize,
    pub applied_updates: usize,
    pub rebuilt_from_source: bool,
    pub preserved_template_parts: bool,
    pub backend: SpreadsheetWriteBackend,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetWriteBackend {
    Native,
    Openpyxl,
    Delimited,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetErrorCode {
    UnsupportedFormat,
    InvalidDelimited,
    InvalidMapping,
    ValidationFailed,
    FileTooLarge,
    OutputTooLarge,
    Io,
    InvalidWorkbook,
    TooManySheets,
    UnsupportedSheetType,
    SheetNotFound,
    InvalidRange,
    RangeTooLarge,
    InvalidQuery,
    InvalidFilter,
    CellOutOfBounds,
    TooManyCells,
    DuplicateSheet,
    DuplicateCellUpdate,
    InvalidSheetName,
    InvalidCellValue,
    CellContentTooLarge,
    NoSheets,
    NoVisibleSheet,
    Serialization,
    BackendUnavailable,
    WorkerTimeout,
    WriteFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetErrorInfo {
    pub code: SpreadsheetErrorCode,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum SpreadsheetError {
    #[error("unsupported spreadsheet format for {path}: {extension:?}")]
    UnsupportedFormat {
        path: PathBuf,
        extension: Option<String>,
    },
    #[error("invalid delimited file {path}: {message}")]
    InvalidDelimited { path: PathBuf, message: String },
    #[error("invalid tabular column mapping: {message}")]
    InvalidMapping { message: String },
    #[error("spreadsheet validation failed: {message}")]
    ValidationFailed { message: String },
    #[error("spreadsheet file {path} is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    FileTooLarge {
        path: PathBuf,
        actual_bytes: u64,
        limit_bytes: u64,
    },
    #[error("generated spreadsheet is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    OutputTooLarge { actual_bytes: u64, limit_bytes: u64 },
    #[error("failed to {operation} {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid XLSX workbook {path}: {message}")]
    InvalidWorkbook { path: PathBuf, message: String },
    #[error("invalid XLSX workbook: cell coordinate overflow in sheet {sheet:?}")]
    InvalidWorkbookCoordinate { sheet: String },
    #[error("workbook has {actual} sheets; limit is {limit}")]
    TooManySheets { actual: usize, limit: usize },
    #[error("sheet {sheet:?} has unsupported type {kind:?}")]
    UnsupportedSheetType { sheet: String, kind: SheetKind },
    #[error("sheet {sheet:?} was not found")]
    SheetNotFound { sheet: String },
    #[error("invalid cell range: {reason}")]
    InvalidRange { reason: &'static str },
    #[error(
        "requested range is {rows} rows x {columns} columns ({cells} cells); limits are {max_rows} rows, {max_columns} columns, and {max_cells} cells"
    )]
    RangeTooLarge {
        rows: u64,
        columns: u64,
        cells: u64,
        max_rows: u64,
        max_columns: u64,
        max_cells: u64,
    },
    #[error("invalid spreadsheet query: {reason}")]
    InvalidQuery { reason: String },
    #[error("invalid spreadsheet filter: {reason}")]
    InvalidFilter { reason: String },
    #[error(
        "cell ({row}, {column}) is outside XLSX bounds (rows 0..{max_rows}, columns 0..{max_columns})"
    )]
    CellOutOfBounds {
        row: u32,
        column: u32,
        max_rows: u32,
        max_columns: u32,
    },
    #[error("{context} contains {actual} cells; limit is {limit}")]
    TooManyCells {
        context: &'static str,
        actual: usize,
        limit: usize,
    },
    #[error("sheet {sheet:?} appears more than once in the write request")]
    DuplicateSheet { sheet: String },
    #[error("sheet {sheet:?} updates cell ({row}, {column}) more than once")]
    DuplicateCellUpdate {
        sheet: String,
        row: u32,
        column: u32,
    },
    #[error("invalid sheet name {sheet:?}: {reason}")]
    InvalidSheetName { sheet: String, reason: &'static str },
    #[error("invalid value for {sheet}!R{row}C{column}: {reason}")]
    InvalidCellValue {
        sheet: String,
        row: u32,
        column: u32,
        reason: String,
    },
    #[error(
        "content at {sheet}!R{row}C{column} is {actual_bytes} bytes; limit is {limit_bytes} bytes"
    )]
    CellContentTooLarge {
        sheet: String,
        row: u32,
        column: u32,
        actual_bytes: usize,
        limit_bytes: usize,
    },
    #[error("a workbook must contain at least one worksheet")]
    NoSheets,
    #[error("a workbook must contain at least one visible worksheet")]
    NoVisibleSheet,
    #[error("failed to serialize spreadsheet result: {message}")]
    Serialization { message: String },
    #[error("spreadsheet backend is unavailable: {message}")]
    BackendUnavailable { message: String },
    #[error("spreadsheet worker exceeded the {seconds} second timeout")]
    WorkerTimeout { seconds: u64 },
    #[error("failed to generate XLSX workbook {path}: {message}")]
    WriteFailed { path: PathBuf, message: String },
}

impl SpreadsheetError {
    pub fn code(&self) -> SpreadsheetErrorCode {
        match self {
            Self::UnsupportedFormat { .. } => SpreadsheetErrorCode::UnsupportedFormat,
            Self::InvalidDelimited { .. } => SpreadsheetErrorCode::InvalidDelimited,
            Self::InvalidMapping { .. } => SpreadsheetErrorCode::InvalidMapping,
            Self::ValidationFailed { .. } => SpreadsheetErrorCode::ValidationFailed,
            Self::FileTooLarge { .. } => SpreadsheetErrorCode::FileTooLarge,
            Self::OutputTooLarge { .. } => SpreadsheetErrorCode::OutputTooLarge,
            Self::Io { .. } => SpreadsheetErrorCode::Io,
            Self::InvalidWorkbook { .. } => SpreadsheetErrorCode::InvalidWorkbook,
            Self::InvalidWorkbookCoordinate { .. } => SpreadsheetErrorCode::InvalidWorkbook,
            Self::TooManySheets { .. } => SpreadsheetErrorCode::TooManySheets,
            Self::UnsupportedSheetType { .. } => SpreadsheetErrorCode::UnsupportedSheetType,
            Self::SheetNotFound { .. } => SpreadsheetErrorCode::SheetNotFound,
            Self::InvalidRange { .. } => SpreadsheetErrorCode::InvalidRange,
            Self::RangeTooLarge { .. } => SpreadsheetErrorCode::RangeTooLarge,
            Self::InvalidQuery { .. } => SpreadsheetErrorCode::InvalidQuery,
            Self::InvalidFilter { .. } => SpreadsheetErrorCode::InvalidFilter,
            Self::CellOutOfBounds { .. } => SpreadsheetErrorCode::CellOutOfBounds,
            Self::TooManyCells { .. } => SpreadsheetErrorCode::TooManyCells,
            Self::DuplicateSheet { .. } => SpreadsheetErrorCode::DuplicateSheet,
            Self::DuplicateCellUpdate { .. } => SpreadsheetErrorCode::DuplicateCellUpdate,
            Self::InvalidSheetName { .. } => SpreadsheetErrorCode::InvalidSheetName,
            Self::InvalidCellValue { .. } => SpreadsheetErrorCode::InvalidCellValue,
            Self::CellContentTooLarge { .. } => SpreadsheetErrorCode::CellContentTooLarge,
            Self::NoSheets => SpreadsheetErrorCode::NoSheets,
            Self::NoVisibleSheet => SpreadsheetErrorCode::NoVisibleSheet,
            Self::Serialization { .. } => SpreadsheetErrorCode::Serialization,
            Self::BackendUnavailable { .. } => SpreadsheetErrorCode::BackendUnavailable,
            Self::WorkerTimeout { .. } => SpreadsheetErrorCode::WorkerTimeout,
            Self::WriteFailed { .. } => SpreadsheetErrorCode::WriteFailed,
        }
    }

    pub fn info(&self) -> SpreadsheetErrorInfo {
        SpreadsheetErrorInfo {
            code: self.code(),
            message: self.to_string(),
        }
    }
}

pub fn execute_spreadsheet(
    request: SpreadsheetRequest,
) -> Result<SpreadsheetResult, SpreadsheetError> {
    match request.action {
        SpreadsheetAction::InspectDelimited(request) => {
            transfer::inspect_delimited(&request).map(SpreadsheetResult::DelimitedInspected)
        }
        SpreadsheetAction::InspectWorkbook(request) => {
            inspect_workbook(&request).map(SpreadsheetResult::WorkbookInspected)
        }
        SpreadsheetAction::ListSheets(request) => {
            list_sheets(&request).map(SpreadsheetResult::SheetsListed)
        }
        SpreadsheetAction::ReadRange(request) => {
            read_range(&request).map(SpreadsheetResult::RangeRead)
        }
        SpreadsheetAction::ReadRanges(request) => {
            read_ranges(&request).map(SpreadsheetResult::RangesRead)
        }
        SpreadsheetAction::FindCells(request) => {
            find_cells(&request).map(SpreadsheetResult::CellsFound)
        }
        SpreadsheetAction::FilterRows(request) => {
            filter_rows(&request).map(SpreadsheetResult::RowsFiltered)
        }
        SpreadsheetAction::ValidateWorkbook(request) => {
            transfer::validate_workbook(&request).map(SpreadsheetResult::WorkbookValidated)
        }
        SpreadsheetAction::ExportDelimited(request) => {
            transfer::export_delimited(&request).map(SpreadsheetResult::DelimitedExported)
        }
        SpreadsheetAction::WriteWorkbook(request) => {
            write_workbook_preferred(&request).map(SpreadsheetResult::WorkbookWritten)
        }
    }
}

pub fn write_workbook_preferred(
    request: &WriteWorkbookRequest,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    if let Some(output_format) =
        SpreadsheetFileFormat::from_path(&request.output).filter(|format| format.is_delimited())
    {
        return write_delimited_workbook(request, output_format);
    }
    let preference = std::env::var("OPENTOPIA_SPREADSHEET_BACKEND")
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase();
    if preference == "native" || (preference == "auto" && native_template_patch_applies(request)?) {
        return write_workbook(request);
    }

    let python = OfficeRuntime::shared().python_for_openpyxl().ok();
    match (preference.as_str(), python) {
        ("auto", Some(python)) => write_workbook_openpyxl(request, &python.executable)
            .or_else(|_| write_workbook(request)),
        ("auto", None) => write_workbook(request),
        ("openpyxl", Some(python)) => write_workbook_openpyxl(request, &python.executable),
        ("openpyxl", None) => Err(SpreadsheetError::BackendUnavailable {
            message: "OPENTOPIA_SPREADSHEET_BACKEND=openpyxl requires the packaged Office runtime; set OPENTOPIA_OFFICE_RUNTIME_ROOT for a development runtime".to_string(),
        }),
        (other, _) => Err(SpreadsheetError::BackendUnavailable {
            message: format!(
                "unsupported OPENTOPIA_SPREADSHEET_BACKEND value {other:?}; expected auto, native, or openpyxl"
            ),
        }),
    }
}

fn native_template_patch_applies(request: &WriteWorkbookRequest) -> Result<bool, SpreadsheetError> {
    let Some(source) = request.source.as_ref() else {
        return Ok(false);
    };
    let source_format = SpreadsheetFileFormat::from_path(source);
    let output_format = SpreadsheetFileFormat::from_path(&request.output);
    if source_format != output_format || !source_format.is_some_and(SpreadsheetFileFormat::is_ooxml)
    {
        return Ok(false);
    }
    if request
        .sheets
        .iter()
        .any(|sheet| sheet.visibility.is_some())
    {
        return Ok(false);
    }
    let listed = list_sheets(&ListSheetsRequest {
        path: source.clone(),
    })?;
    Ok(request.sheets.iter().all(|requested| {
        listed
            .sheets
            .iter()
            .any(|existing| existing.name.eq_ignore_ascii_case(&requested.name))
    }))
}

pub fn write_workbook_openpyxl(
    request: &WriteWorkbookRequest,
    python: &Path,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    validate_xlsx_path(&request.output)?;
    let applied_updates = validate_write_request(request)?;
    if request.sheets.is_empty() && request.source.is_none() {
        return Err(SpreadsheetError::NoSheets);
    }
    if let Some(source) = request.source.as_ref() {
        let _ = list_sheets(&ListSheetsRequest {
            path: source.clone(),
        })?;
    }

    run_openpyxl_worker(request, &request.output, python)?;

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
    let result = WriteWorkbookResult {
        output: request.output.clone(),
        bytes_written,
        sheet_count: inspected.sheets.len(),
        output_cells: inspected.populated_cells as usize,
        applied_updates,
        rebuilt_from_source: request.source.is_some(),
        preserved_template_parts: false,
        backend: SpreadsheetWriteBackend::Openpyxl,
    };
    Ok(result)
}

pub(crate) fn run_openpyxl_worker<T: Serialize>(
    request: &T,
    output: &Path,
    python: &Path,
) -> Result<serde_json::Value, SpreadsheetError> {
    let payload = serde_json::to_vec(request).map_err(|error| SpreadsheetError::Serialization {
        message: error.to_string(),
    })?;
    let mut child = Command::new(python)
        .args(["-I", "-c", OPENPYXL_WORKER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| SpreadsheetError::BackendUnavailable {
            message: format!("failed to start {}: {error}", python.display()),
        })?;
    child
        .stdin
        .take()
        .expect("piped worker stdin")
        .write_all(&payload)
        .map_err(|error| write_failed(output, error))?;

    let status = match child
        .wait_timeout(OPENPYXL_WORKER_TIMEOUT)
        .map_err(|error| write_failed(output, error))?
    {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SpreadsheetError::WorkerTimeout {
                seconds: OPENPYXL_WORKER_TIMEOUT.as_secs(),
            });
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut stdout_pipe) = child.stdout.take() {
        stdout_pipe
            .read_to_string(&mut stdout)
            .map_err(|error| write_failed(output, error))?;
    }
    if let Some(mut stderr_pipe) = child.stderr.take() {
        stderr_pipe
            .read_to_string(&mut stderr)
            .map_err(|error| write_failed(output, error))?;
    }
    if !status.success() {
        let message = stderr.trim();
        return Err(write_failed(
            output,
            if message.is_empty() {
                format!("openpyxl worker exited with {status}")
            } else {
                message.to_string()
            },
        ));
    }
    let worker_result: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|error| SpreadsheetError::Serialization {
            message: format!("invalid openpyxl worker response: {error}"),
        })?;
    if worker_result.get("ok") != Some(&serde_json::Value::Bool(true)) {
        return Err(write_failed(
            output,
            "openpyxl worker did not report success",
        ));
    }
    Ok(worker_result)
}

pub fn write_workbook(
    request: &WriteWorkbookRequest,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    let output_format = SpreadsheetFileFormat::from_path(&request.output).ok_or_else(|| {
        SpreadsheetError::UnsupportedFormat {
            path: request.output.clone(),
            extension: request
                .output
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_string),
        }
    })?;
    if !output_format.is_ooxml() {
        return Err(SpreadsheetError::UnsupportedFormat {
            path: request.output.clone(),
            extension: request
                .output
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_string),
        });
    }
    let applied_updates = validate_write_request(request)?;
    let mut loaded = match &request.source {
        Some(source) => load_spreadsheet(source)?,
        None => LoadedWorkbook::default(),
    };
    let preserve_template_parts = request.source.as_ref().is_some_and(|source| {
        SpreadsheetFileFormat::from_path(source) == Some(output_format) && output_format.is_ooxml()
    }) && request.sheets.iter().all(|sheet| {
        sheet.visibility.is_none()
            && loaded
                .sheets
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(&sheet.name))
    });
    if matches!(
        output_format,
        SpreadsheetFileFormat::Xlsm | SpreadsheetFileFormat::Xltx | SpreadsheetFileFormat::Xltm
    ) && !preserve_template_parts
    {
        return Err(SpreadsheetError::ValidationFailed {
            message: format!(
                ".{} output requires package-preserving updates to an existing same-format source",
                output_format.extension()
            ),
        });
    }

    apply_sheet_updates(&mut loaded, &request.sheets)?;
    if loaded.sheets.is_empty() {
        return Err(SpreadsheetError::NoSheets);
    }
    ensure_sheet_count(loaded.sheets.len())?;
    if !loaded
        .sheets
        .iter()
        .any(|sheet| sheet.visibility == SheetVisibility::Visible)
    {
        return Err(SpreadsheetError::NoVisibleSheet);
    }

    let output_cells = loaded.sheets.iter().map(|sheet| sheet.cells.len()).sum();
    ensure_workbook_cell_count(output_cells)?;
    let bytes = if preserve_template_parts {
        patch_workbook_template(
            request.source.as_deref().expect("template source exists"),
            &request.sheets,
            &request.output,
        )?
    } else {
        render_workbook(&loaded, &request.output)?
    };
    let bytes_written = bytes.len() as u64;
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }

    let result = WriteWorkbookResult {
        output: request.output.clone(),
        bytes_written,
        sheet_count: loaded.sheets.len(),
        output_cells,
        applied_updates,
        rebuilt_from_source: request.source.is_some() && !preserve_template_parts,
        preserved_template_parts: preserve_template_parts,
        backend: SpreadsheetWriteBackend::Native,
    };
    fs::write(&request.output, &bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    Ok(result)
}

fn open_workbook_reader(path: &Path) -> Result<(Sheets<BufReader<File>>, u64), SpreadsheetError> {
    validate_workbook_path(path)?;
    let metadata = fs::metadata(path).map_err(|source| SpreadsheetError::Io {
        operation: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    let file_size_bytes = metadata.len();
    if file_size_bytes > MAX_INPUT_FILE_BYTES {
        return Err(SpreadsheetError::FileTooLarge {
            path: path.to_path_buf(),
            actual_bytes: file_size_bytes,
            limit_bytes: MAX_INPUT_FILE_BYTES,
        });
    }
    let workbook = open_workbook_auto(path).map_err(|error| SpreadsheetError::InvalidWorkbook {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Ok((workbook, file_size_bytes))
}

fn validate_workbook_path(path: &Path) -> Result<SpreadsheetFileFormat, SpreadsheetError> {
    let format = SpreadsheetFileFormat::from_path(path).ok_or_else(|| {
        SpreadsheetError::UnsupportedFormat {
            path: path.to_path_buf(),
            extension: path.extension().and_then(OsStr::to_str).map(str::to_string),
        }
    })?;
    if !format.is_workbook() {
        return Err(SpreadsheetError::UnsupportedFormat {
            path: path.to_path_buf(),
            extension: path.extension().and_then(OsStr::to_str).map(str::to_string),
        });
    }
    Ok(format)
}

fn validate_xlsx_path(path: &Path) -> Result<(), SpreadsheetError> {
    let extension = path.extension().and_then(OsStr::to_str);
    if !extension.is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx")) {
        return Err(SpreadsheetError::UnsupportedFormat {
            path: path.to_path_buf(),
            extension: extension.map(str::to_string),
        });
    }
    Ok(())
}

fn ensure_sheet_count(actual: usize) -> Result<(), SpreadsheetError> {
    if actual > MAX_SHEETS {
        return Err(SpreadsheetError::TooManySheets {
            actual,
            limit: MAX_SHEETS,
        });
    }
    Ok(())
}

fn ensure_workbook_cell_count(actual: usize) -> Result<(), SpreadsheetError> {
    if actual > MAX_WORKBOOK_CELLS {
        return Err(SpreadsheetError::TooManyCells {
            context: "workbook",
            actual,
            limit: MAX_WORKBOOK_CELLS,
        });
    }
    Ok(())
}

fn validate_address(address: CellAddress) -> Result<(), SpreadsheetError> {
    if address.row >= EXCEL_MAX_ROWS || address.column >= EXCEL_MAX_COLUMNS {
        return Err(SpreadsheetError::CellOutOfBounds {
            row: address.row,
            column: address.column,
            max_rows: EXCEL_MAX_ROWS - 1,
            max_columns: EXCEL_MAX_COLUMNS - 1,
        });
    }
    Ok(())
}

fn validate_read_range(range: CellRange) -> Result<(), SpreadsheetError> {
    let (rows, columns, cells) = validate_range_dimensions(range)?;
    if rows > MAX_READ_ROWS || columns > MAX_READ_COLUMNS || cells > MAX_READ_CELLS {
        return Err(SpreadsheetError::RangeTooLarge {
            rows,
            columns,
            cells,
            max_rows: MAX_READ_ROWS,
            max_columns: MAX_READ_COLUMNS,
            max_cells: MAX_READ_CELLS,
        });
    }
    Ok(())
}

fn validate_scan_range(range: CellRange) -> Result<(), SpreadsheetError> {
    validate_range_dimensions(range).map(|_| ())
}

fn validate_range_dimensions(range: CellRange) -> Result<(u64, u64, u64), SpreadsheetError> {
    validate_address(range.start)?;
    validate_address(range.end)?;
    let Some(rows) = range.row_count() else {
        return Err(SpreadsheetError::InvalidRange {
            reason: "start row must not exceed end row",
        });
    };
    let Some(columns) = range.column_count() else {
        return Err(SpreadsheetError::InvalidRange {
            reason: "start column must not exceed end column",
        });
    };
    let cells = rows
        .checked_mul(columns)
        .ok_or(SpreadsheetError::InvalidRange {
            reason: "range cell count overflowed",
        })?;
    Ok((rows, columns, cells))
}

fn validate_write_request(request: &WriteWorkbookRequest) -> Result<usize, SpreadsheetError> {
    if let Some(source) = &request.source {
        if SpreadsheetFileFormat::from_path(source).is_none() {
            return Err(SpreadsheetError::UnsupportedFormat {
                path: source.clone(),
                extension: source
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(str::to_string),
            });
        }
    }
    let update_count = request
        .sheets
        .iter()
        .try_fold(0usize, |count, sheet| count.checked_add(sheet.cells.len()))
        .unwrap_or(usize::MAX);
    if update_count > MAX_WRITE_UPDATES {
        return Err(SpreadsheetError::TooManyCells {
            context: "write request",
            actual: update_count,
            limit: MAX_WRITE_UPDATES,
        });
    }

    let mut sheet_names = HashSet::with_capacity(request.sheets.len());
    for sheet in &request.sheets {
        validate_sheet_name(&sheet.name)?;
        if !sheet_names.insert(sheet.name.to_lowercase()) {
            return Err(SpreadsheetError::DuplicateSheet {
                sheet: sheet.name.clone(),
            });
        }

        let mut addresses = HashSet::with_capacity(sheet.cells.len());
        for update in &sheet.cells {
            validate_address(update.address)?;
            if let Some(style_from) = update.style_from {
                validate_address(style_from)?;
            }
            if !addresses.insert(update.address) {
                return Err(SpreadsheetError::DuplicateCellUpdate {
                    sheet: sheet.name.clone(),
                    row: update.address.row,
                    column: update.address.column,
                });
            }
            validate_cell_input(&sheet.name, update)?;
        }
    }
    Ok(update_count)
}

fn validate_sheet_name(sheet: &str) -> Result<(), SpreadsheetError> {
    let reason = if sheet.is_empty() {
        Some("name must not be empty")
    } else if sheet.chars().count() > 31 {
        Some("name must not exceed 31 characters")
    } else if sheet
        .chars()
        .any(|character| "[]:*?/\\".contains(character))
    {
        Some("name contains an Excel-reserved character")
    } else if sheet.starts_with('\'') || sheet.ends_with('\'') {
        Some("name must not start or end with an apostrophe")
    } else if sheet.eq_ignore_ascii_case("history") {
        Some("name is reserved by Excel")
    } else if contains_invalid_xml_character(sheet) {
        Some("name contains an unsupported control character")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(SpreadsheetError::InvalidSheetName {
            sheet: sheet.to_string(),
            reason,
        });
    }
    Ok(())
}

fn validate_cell_input(sheet: &str, update: &CellUpdate) -> Result<(), SpreadsheetError> {
    let row = update.address.row;
    let column = update.address.column;
    match &update.value {
        SpreadsheetCellInput::Blank | SpreadsheetCellInput::Boolean(_) => Ok(()),
        SpreadsheetCellInput::Integer(value) => {
            if value.unsigned_abs() > MAX_EXCEL_INTEGER as u64 {
                Err(invalid_cell_value(
                    sheet,
                    row,
                    column,
                    "integer exceeds Excel's 15-digit numeric precision",
                ))
            } else {
                Ok(())
            }
        }
        SpreadsheetCellInput::Number(value) => {
            if value.is_finite() {
                Ok(())
            } else {
                Err(invalid_cell_value(
                    sheet,
                    row,
                    column,
                    "number must be finite",
                ))
            }
        }
        SpreadsheetCellInput::String(value) => validate_write_text(value, sheet, row, column),
        SpreadsheetCellInput::Formula(formula) => {
            let expression = formula.expression.trim();
            if expression.is_empty() {
                return Err(invalid_cell_value(
                    sheet,
                    row,
                    column,
                    "formula must not be empty",
                ));
            }
            if formula.expression.len() > MAX_FORMULA_BYTES {
                return Err(invalid_cell_value(
                    sheet,
                    row,
                    column,
                    format!("formula exceeds {MAX_FORMULA_BYTES} bytes"),
                ));
            }
            if contains_invalid_xml_character(&formula.expression) {
                return Err(invalid_cell_value(
                    sheet,
                    row,
                    column,
                    "formula contains an unsupported control character",
                ));
            }
            if let Some(result) = &formula.cached_result {
                validate_write_text(result, sheet, row, column)?;
            }
            Ok(())
        }
    }
}

fn validate_write_text(
    value: &str,
    sheet: &str,
    row: u32,
    column: u32,
) -> Result<(), SpreadsheetError> {
    if value.len() > MAX_CELL_TEXT_BYTES {
        return Err(SpreadsheetError::CellContentTooLarge {
            sheet: sheet.to_string(),
            row,
            column,
            actual_bytes: value.len(),
            limit_bytes: MAX_CELL_TEXT_BYTES,
        });
    }
    if value.chars().count() > MAX_CELL_CHARACTERS {
        return Err(invalid_cell_value(
            sheet,
            row,
            column,
            format!("text exceeds {MAX_CELL_CHARACTERS} characters"),
        ));
    }
    if contains_invalid_xml_character(value) {
        return Err(invalid_cell_value(
            sheet,
            row,
            column,
            "text contains an unsupported control character",
        ));
    }
    Ok(())
}

fn validate_return_text(
    value: &str,
    sheet: &str,
    row: u32,
    column: u32,
) -> Result<(), SpreadsheetError> {
    if value.len() > MAX_CELL_TEXT_BYTES {
        return Err(SpreadsheetError::CellContentTooLarge {
            sheet: sheet.to_string(),
            row,
            column,
            actual_bytes: value.len(),
            limit_bytes: MAX_CELL_TEXT_BYTES,
        });
    }
    Ok(())
}

fn invalid_cell_value(
    sheet: &str,
    row: u32,
    column: u32,
    reason: impl Into<String>,
) -> SpreadsheetError {
    SpreadsheetError::InvalidCellValue {
        sheet: sheet.to_string(),
        row,
        column,
        reason: reason.into(),
    }
}

fn contains_invalid_xml_character(value: &str) -> bool {
    value.chars().any(|character| {
        let code = character as u32;
        code < 0x20 && !matches!(character, '\t' | '\n' | '\r')
    })
}

fn worksheet_values(
    workbook: &mut Sheets<BufReader<File>>,
    path: &Path,
    sheet: &str,
) -> Result<Range<Data>, SpreadsheetError> {
    workbook
        .worksheet_range(sheet)
        .map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: path.to_path_buf(),
            message: format!("failed to read values from sheet {sheet:?}: {error}"),
        })
}

fn worksheet_formulas(
    workbook: &mut Sheets<BufReader<File>>,
    path: &Path,
    sheet: &str,
) -> Result<Range<String>, SpreadsheetError> {
    workbook
        .worksheet_formula(sheet)
        .map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: path.to_path_buf(),
            message: format!("failed to read formulas from sheet {sheet:?}: {error}"),
        })
}

#[derive(Debug, Default)]
struct SheetStats {
    used_range: Option<CellRange>,
    populated_cells: usize,
}

fn collect_sheet_stats(
    values: &Range<Data>,
    formulas: &Range<String>,
    sheet: &str,
) -> Result<SheetStats, SpreadsheetError> {
    let mut positions = HashSet::new();
    add_used_positions(values, &mut positions, sheet)?;
    add_used_positions(formulas, &mut positions, sheet)?;
    let populated_cells = positions.len();
    ensure_workbook_cell_count(populated_cells)?;

    let used_range = if positions.is_empty() {
        None
    } else {
        let mut min_row = u32::MAX;
        let mut min_column = u32::MAX;
        let mut max_row = 0;
        let mut max_column = 0;
        for &(row, column) in &positions {
            min_row = min_row.min(row);
            min_column = min_column.min(column);
            max_row = max_row.max(row);
            max_column = max_column.max(column);
        }
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
    };

    Ok(SheetStats {
        used_range,
        populated_cells,
    })
}

fn add_used_positions<T: CellType>(
    range: &Range<T>,
    positions: &mut HashSet<(u32, u32)>,
    sheet: &str,
) -> Result<(), SpreadsheetError> {
    let Some((base_row, base_column)) = range.start() else {
        return Ok(());
    };
    for (relative_row, relative_column, _) in range.used_cells() {
        let relative_row =
            u32::try_from(relative_row).map_err(|_| invalid_workbook_coordinate(sheet))?;
        let relative_column =
            u32::try_from(relative_column).map_err(|_| invalid_workbook_coordinate(sheet))?;
        let row = base_row
            .checked_add(relative_row)
            .ok_or_else(|| invalid_workbook_coordinate(sheet))?;
        let column = base_column
            .checked_add(relative_column)
            .ok_or_else(|| invalid_workbook_coordinate(sheet))?;
        validate_address(CellAddress { row, column })?;
        positions.insert((row, column));
        if positions.len() > MAX_WORKBOOK_CELLS {
            ensure_workbook_cell_count(positions.len())?;
        }
    }
    Ok(())
}

fn invalid_workbook_coordinate(sheet: &str) -> SpreadsheetError {
    SpreadsheetError::InvalidWorkbookCoordinate {
        sheet: sheet.to_string(),
    }
}

fn cell_value_from_data(
    value: &Data,
    sheet: &str,
    row: u32,
    column: u32,
) -> Result<SpreadsheetCellValue, SpreadsheetError> {
    let result = match value {
        Data::Empty => SpreadsheetCellValue::Empty,
        Data::String(value) => {
            validate_return_text(value, sheet, row, column)?;
            SpreadsheetCellValue::String(value.clone())
        }
        Data::Int(value) => SpreadsheetCellValue::Integer(*value),
        Data::Float(value) if value.is_finite() => SpreadsheetCellValue::Number(*value),
        Data::Float(_) => {
            return Err(invalid_cell_value(
                sheet,
                row,
                column,
                "workbook contains a non-finite number",
            ));
        }
        Data::Bool(value) => SpreadsheetCellValue::Boolean(*value),
        Data::DateTime(value) => SpreadsheetCellValue::DateTime(ExcelDateTimeValue {
            serial: value.as_f64(),
            is_duration: value.is_duration(),
        }),
        Data::DateTimeIso(value) => {
            validate_return_text(value, sheet, row, column)?;
            SpreadsheetCellValue::DateTimeIso(value.clone())
        }
        Data::DurationIso(value) => {
            validate_return_text(value, sheet, row, column)?;
            SpreadsheetCellValue::DurationIso(value.clone())
        }
        Data::Error(value) => SpreadsheetCellValue::Error(value.to_string()),
    };
    Ok(result)
}

fn sheet_info(sheet: &calamine::Sheet) -> SheetInfo {
    SheetInfo {
        name: sheet.name.clone(),
        kind: match sheet.typ {
            CalamineSheetType::WorkSheet => SheetKind::Worksheet,
            CalamineSheetType::DialogSheet => SheetKind::DialogSheet,
            CalamineSheetType::MacroSheet => SheetKind::MacroSheet,
            CalamineSheetType::ChartSheet => SheetKind::ChartSheet,
            CalamineSheetType::Vba => SheetKind::Vba,
        },
        visibility: match sheet.visible {
            CalamineSheetVisible::Visible => SheetVisibility::Visible,
            CalamineSheetVisible::Hidden => SheetVisibility::Hidden,
            CalamineSheetVisible::VeryHidden => SheetVisibility::VeryHidden,
        },
    }
}

#[cfg(test)]
mod tests;
