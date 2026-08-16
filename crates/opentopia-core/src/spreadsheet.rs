use calamine::{
    CellType, Data, Range, Reader, SheetType as CalamineSheetType,
    SheetVisible as CalamineSheetVisible, Xlsx,
};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event as XmlEvent};
use quick_xml::{Reader as XmlReader, Writer as XmlWriter};
use rust_xlsxwriter::{Formula, Workbook, Worksheet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufReader, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

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
pub const MAX_RETURN_BYTES: usize = 1024 * 1024;
pub const MAX_CELL_CHARACTERS: usize = 32_767;
pub const MAX_CELL_TEXT_BYTES: usize = 128 * 1024;
pub const MAX_FORMULA_BYTES: usize = 8_192;
pub const MAX_FIND_RESULTS: usize = 1_000;
pub const MAX_FILTER_CONDITIONS: usize = 32;
pub const MAX_FILTER_RESULTS: usize = 1_000;

const MAX_EXCEL_INTEGER: i64 = 999_999_999_999_999;
const OPENPYXL_WORKER_TIMEOUT: Duration = Duration::from_secs(60);
const OPENPYXL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const OPENPYXL_WORKER: &str = include_str!("spreadsheet_openpyxl_worker.py");
static DISCOVERED_OPENPYXL_PYTHON: OnceLock<Option<PathBuf>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetRequest {
    pub action: SpreadsheetAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "request", rename_all = "snake_case")]
pub enum SpreadsheetAction {
    InspectWorkbook(InspectWorkbookRequest),
    ListSheets(ListSheetsRequest),
    ReadRange(ReadRangeRequest),
    ReadRanges(ReadRangesRequest),
    FindCells(FindCellsRequest),
    FilterRows(FilterRowsRequest),
    WriteWorkbook(WriteWorkbookRequest),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetActionKind {
    InspectWorkbook,
    ListSheets,
    ReadRange,
    ReadRanges,
    FindCells,
    FilterRows,
    WriteWorkbook,
}

impl SpreadsheetAction {
    pub fn kind(&self) -> SpreadsheetActionKind {
        match self {
            Self::InspectWorkbook(_) => SpreadsheetActionKind::InspectWorkbook,
            Self::ListSheets(_) => SpreadsheetActionKind::ListSheets,
            Self::ReadRange(_) => SpreadsheetActionKind::ReadRange,
            Self::ReadRanges(_) => SpreadsheetActionKind::ReadRanges,
            Self::FindCells(_) => SpreadsheetActionKind::FindCells,
            Self::FilterRows(_) => SpreadsheetActionKind::FilterRows,
            Self::WriteWorkbook(_) => SpreadsheetActionKind::WriteWorkbook,
        }
    }
}

impl SpreadsheetActionKind {
    pub fn is_mutation(self) -> bool {
        matches!(self, Self::WriteWorkbook)
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "result", rename_all = "snake_case")]
pub enum SpreadsheetResult {
    WorkbookInspected(InspectWorkbookResult),
    SheetsListed(ListSheetsResult),
    RangeRead(ReadRangeResult),
    RangesRead(ReadRangesResult),
    CellsFound(FindCellsResult),
    RowsFiltered(FilterRowsResult),
    WorkbookWritten(WriteWorkbookResult),
}

impl SpreadsheetResult {
    pub fn kind(&self) -> SpreadsheetActionKind {
        match self {
            Self::WorkbookInspected(_) => SpreadsheetActionKind::InspectWorkbook,
            Self::SheetsListed(_) => SpreadsheetActionKind::ListSheets,
            Self::RangeRead(_) => SpreadsheetActionKind::ReadRange,
            Self::RangesRead(_) => SpreadsheetActionKind::ReadRanges,
            Self::CellsFound(_) => SpreadsheetActionKind::FindCells,
            Self::RowsFiltered(_) => SpreadsheetActionKind::FilterRows,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    pub matched_row_indices: Vec<u32>,
    pub rows: Vec<Vec<SpreadsheetCell>>,
    pub scanned_rows: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetCell {
    pub value: SpreadsheetCellValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpreadsheetErrorCode {
    UnsupportedFormat,
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
    ReturnTooLarge,
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
    #[error("unsupported spreadsheet format for {path}: expected .xlsx, found {extension:?}")]
    UnsupportedFormat {
        path: PathBuf,
        extension: Option<String>,
    },
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
    #[error("serialized result is {actual_bytes} bytes; return limit is {limit_bytes} bytes")]
    ReturnTooLarge {
        actual_bytes: usize,
        limit_bytes: usize,
    },
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
            Self::ReturnTooLarge { .. } => SpreadsheetErrorCode::ReturnTooLarge,
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
        SpreadsheetAction::WriteWorkbook(request) => {
            write_workbook_preferred(&request).map(SpreadsheetResult::WorkbookWritten)
        }
    }
}

pub fn list_sheets(request: &ListSheetsRequest) -> Result<ListSheetsResult, SpreadsheetError> {
    let (workbook, file_size_bytes) = open_xlsx(&request.path)?;
    let sheets = workbook
        .sheets_metadata()
        .iter()
        .map(sheet_info)
        .collect::<Vec<_>>();
    ensure_sheet_count(sheets.len())?;

    let result = ListSheetsResult {
        path: request.path.clone(),
        file_size_bytes,
        sheets,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn inspect_workbook(
    request: &InspectWorkbookRequest,
) -> Result<InspectWorkbookResult, SpreadsheetError> {
    let (mut workbook, file_size_bytes) = open_xlsx(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;

    let mut sheets = Vec::with_capacity(metadata.len());
    let mut workbook_cells = 0usize;
    for sheet in metadata {
        let info = sheet_info(&sheet);
        let stats = if info.kind == SheetKind::Worksheet {
            let values = worksheet_values(&mut workbook, &request.path, &info.name)?;
            let formulas = worksheet_formulas(&mut workbook, &request.path, &info.name)?;
            collect_sheet_stats(&values, &formulas, &info.name)?
        } else {
            SheetStats::default()
        };

        workbook_cells = workbook_cells.checked_add(stats.populated_cells).ok_or(
            SpreadsheetError::TooManyCells {
                context: "workbook",
                actual: usize::MAX,
                limit: MAX_WORKBOOK_CELLS,
            },
        )?;
        ensure_workbook_cell_count(workbook_cells)?;
        sheets.push(SheetInspection {
            sheet: info,
            used_range: stats.used_range,
            populated_cells: stats.populated_cells as u64,
        });
    }

    let result = InspectWorkbookResult {
        path: request.path.clone(),
        file_size_bytes,
        sheets,
        populated_cells: workbook_cells as u64,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn read_range(request: &ReadRangeRequest) -> Result<ReadRangeResult, SpreadsheetError> {
    validate_read_range(request.range)?;
    let (mut workbook, _) = open_xlsx(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    let sheet = metadata
        .iter()
        .find(|sheet| sheet.name == request.sheet)
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.sheet.clone(),
        })?;
    let info = sheet_info(sheet);
    if info.kind != SheetKind::Worksheet {
        return Err(SpreadsheetError::UnsupportedSheetType {
            sheet: info.name,
            kind: info.kind,
        });
    }

    let values = worksheet_values(&mut workbook, &request.path, &request.sheet)?;
    let formulas = worksheet_formulas(&mut workbook, &request.path, &request.sheet)?;
    let stats = collect_sheet_stats(&values, &formulas, &request.sheet)?;
    ensure_workbook_cell_count(stats.populated_cells)?;

    let row_count = request.range.row_count().expect("validated range") as usize;
    let column_count = request.range.column_count().expect("validated range") as usize;
    let mut rows = Vec::with_capacity(row_count);
    for row in request.range.start.row..=request.range.end.row {
        let mut cells = Vec::with_capacity(column_count);
        for column in request.range.start.column..=request.range.end.column {
            let value = values.get_value((row, column)).unwrap_or(&Data::Empty);
            let formula = formulas
                .get_value((row, column))
                .filter(|formula| !formula.is_empty())
                .cloned();
            if let Some(formula) = &formula {
                validate_return_text(formula, &request.sheet, row, column)?;
            }
            cells.push(SpreadsheetCell {
                value: cell_value_from_data(value, &request.sheet, row, column)?,
                formula,
            });
        }
        rows.push(cells);
    }

    let result = ReadRangeResult {
        path: request.path.clone(),
        sheet: request.sheet.clone(),
        range: request.range,
        rows,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn read_ranges(request: &ReadRangesRequest) -> Result<ReadRangesResult, SpreadsheetError> {
    if request.ranges.is_empty() {
        return Err(SpreadsheetError::InvalidRange {
            reason: "read_ranges requires at least one range",
        });
    }
    if request.ranges.len() > MAX_READ_RANGES {
        return Err(SpreadsheetError::TooManyCells {
            context: "read range list",
            actual: request.ranges.len(),
            limit: MAX_READ_RANGES,
        });
    }
    let total_cells = request.ranges.iter().try_fold(0u64, |total, item| {
        validate_read_range(item.range)?;
        total
            .checked_add(item.range.cell_count().expect("validated range"))
            .ok_or(SpreadsheetError::InvalidRange {
                reason: "combined range cell count overflowed",
            })
    })?;
    if total_cells > MAX_READ_CELLS {
        return Err(SpreadsheetError::RangeTooLarge {
            rows: request
                .ranges
                .iter()
                .filter_map(|item| item.range.row_count())
                .sum(),
            columns: request
                .ranges
                .iter()
                .filter_map(|item| item.range.column_count())
                .sum(),
            cells: total_cells,
            max_rows: MAX_READ_ROWS,
            max_columns: MAX_READ_COLUMNS,
            max_cells: MAX_READ_CELLS,
        });
    }

    let ranges = request
        .ranges
        .iter()
        .map(|item| {
            read_range(&ReadRangeRequest {
                path: request.path.clone(),
                sheet: item.sheet.clone(),
                range: item.range,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = ReadRangesResult {
        path: request.path.clone(),
        ranges,
        total_cells,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn find_cells(request: &FindCellsRequest) -> Result<FindCellsResult, SpreadsheetError> {
    if request.query.is_empty() {
        return Err(SpreadsheetError::InvalidQuery {
            reason: "query must not be empty".to_string(),
        });
    }
    if request.max_results == 0 || request.max_results > MAX_FIND_RESULTS {
        return Err(SpreadsheetError::InvalidQuery {
            reason: format!("maxResults must be between 1 and {MAX_FIND_RESULTS}"),
        });
    }
    if request.range.is_some() && request.sheet.is_none() {
        return Err(SpreadsheetError::InvalidQuery {
            reason: "range requires a sheet".to_string(),
        });
    }
    if let Some(range) = request.range {
        validate_read_range(range)?;
    }

    let (mut workbook, _) = open_xlsx(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    if let Some(sheet) = request.sheet.as_deref() {
        if !metadata.iter().any(|candidate| candidate.name == sheet) {
            return Err(SpreadsheetError::SheetNotFound {
                sheet: sheet.to_string(),
            });
        }
    }

    let mut matches = Vec::new();
    let mut scanned_cells = 0usize;
    let mut truncated = false;
    for sheet in metadata {
        let info = sheet_info(&sheet);
        if info.kind != SheetKind::Worksheet
            || request
                .sheet
                .as_deref()
                .is_some_and(|requested| requested != info.name)
        {
            continue;
        }
        let values = worksheet_values(&mut workbook, &request.path, &info.name)?;
        let formulas = worksheet_formulas(&mut workbook, &request.path, &info.name)?;
        let mut positions = HashSet::new();
        add_used_positions(&values, &mut positions, &info.name)?;
        add_used_positions(&formulas, &mut positions, &info.name)?;
        let mut positions = positions.into_iter().collect::<Vec<_>>();
        positions.sort_unstable();

        for (row, column) in positions {
            if request.range.is_some_and(|range| {
                row < range.start.row
                    || row > range.end.row
                    || column < range.start.column
                    || column > range.end.column
            }) {
                continue;
            }
            scanned_cells = scanned_cells.saturating_add(1);
            ensure_workbook_cell_count(scanned_cells)?;
            let value = values.get_value((row, column)).unwrap_or(&Data::Empty);
            let formula = formulas
                .get_value((row, column))
                .filter(|formula| !formula.is_empty())
                .cloned();
            let cell = SpreadsheetCell {
                value: cell_value_from_data(value, &info.name, row, column)?,
                formula,
            };
            let value_text = spreadsheet_cell_display_text(&cell.value);
            let value_matches = text_matches(
                &value_text,
                &request.query,
                request.match_mode,
                request.case_sensitive,
            );
            let formula_matches = request.include_formulas
                && cell.formula.as_deref().is_some_and(|formula| {
                    text_matches(
                        formula,
                        &request.query,
                        request.match_mode,
                        request.case_sensitive,
                    )
                });
            if value_matches || formula_matches {
                if matches.len() == request.max_results {
                    truncated = true;
                    break;
                }
                matches.push(SpreadsheetCellMatch {
                    sheet: info.name.clone(),
                    address: CellAddress { row, column },
                    cell,
                    matched_formula: !value_matches && formula_matches,
                });
            }
        }
        if truncated {
            break;
        }
    }

    let result = FindCellsResult {
        path: request.path.clone(),
        matches,
        scanned_cells: scanned_cells as u64,
        truncated,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn filter_rows(request: &FilterRowsRequest) -> Result<FilterRowsResult, SpreadsheetError> {
    validate_read_range(request.range)?;
    if request.conditions.is_empty() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "conditions must not be empty".to_string(),
        });
    }
    if request.conditions.len() > MAX_FILTER_CONDITIONS {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("conditions are limited to {MAX_FILTER_CONDITIONS}"),
        });
    }
    if request.max_results == 0 || request.max_results > MAX_FILTER_RESULTS {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("maxResults must be between 1 and {MAX_FILTER_RESULTS}"),
        });
    }
    for condition in &request.conditions {
        validate_filter_condition(condition, request.range)?;
    }

    let read = read_range(&ReadRangeRequest {
        path: request.path.clone(),
        sheet: request.sheet.clone(),
        range: request.range,
    })?;
    let mut rows = Vec::new();
    let mut matched_row_indices = Vec::new();
    let mut truncated = false;
    let mut scanned_rows = 0_u64;
    for (row_offset, row) in read.rows.into_iter().enumerate() {
        scanned_rows = scanned_rows.saturating_add(1);
        let include = match request.match_mode {
            SpreadsheetFilterMatchMode::All => request.conditions.iter().all(|condition| {
                let index = (condition.column - request.range.start.column) as usize;
                filter_condition_matches(&row[index], condition)
            }),
            SpreadsheetFilterMatchMode::Any => request.conditions.iter().any(|condition| {
                let index = (condition.column - request.range.start.column) as usize;
                filter_condition_matches(&row[index], condition)
            }),
        };
        if include {
            if rows.len() == request.max_results {
                truncated = true;
                break;
            }
            let row_index = request
                .range
                .start
                .row
                .checked_add(row_offset as u32)
                .ok_or(SpreadsheetError::InvalidRange {
                    reason: "filtered row index overflowed",
                })?;
            matched_row_indices.push(row_index);
            rows.push(row);
        }
    }

    let result = FilterRowsResult {
        path: request.path.clone(),
        sheet: request.sheet.clone(),
        range: request.range,
        matched_row_indices,
        rows,
        scanned_rows,
        truncated,
    };
    ensure_return_size(&result)?;
    Ok(result)
}

fn spreadsheet_cell_display_text(value: &SpreadsheetCellValue) -> String {
    match value {
        SpreadsheetCellValue::Empty => String::new(),
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => value.clone(),
        SpreadsheetCellValue::Integer(value) => value.to_string(),
        SpreadsheetCellValue::Number(value) => value.to_string(),
        SpreadsheetCellValue::Boolean(value) => value.to_string(),
        SpreadsheetCellValue::DateTime(value) => value.serial.to_string(),
    }
}

fn text_matches(
    candidate: &str,
    query: &str,
    mode: SpreadsheetTextMatchMode,
    case_sensitive: bool,
) -> bool {
    let (candidate, query) = if case_sensitive {
        (candidate.to_string(), query.to_string())
    } else {
        (candidate.to_lowercase(), query.to_lowercase())
    };
    match mode {
        SpreadsheetTextMatchMode::Contains => candidate.contains(&query),
        SpreadsheetTextMatchMode::Exact => candidate == query,
        SpreadsheetTextMatchMode::StartsWith => candidate.starts_with(&query),
        SpreadsheetTextMatchMode::EndsWith => candidate.ends_with(&query),
    }
}

fn validate_filter_condition(
    condition: &SpreadsheetFilterCondition,
    range: CellRange,
) -> Result<(), SpreadsheetError> {
    validate_address(CellAddress {
        row: range.start.row,
        column: condition.column,
    })?;
    if condition.column < range.start.column || condition.column > range.end.column {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!(
                "condition column {} is outside the requested range {}..={}",
                condition.column, range.start.column, range.end.column
            ),
        });
    }
    let blank_operator = matches!(
        condition.operator,
        SpreadsheetFilterOperator::IsBlank | SpreadsheetFilterOperator::IsNotBlank
    );
    if blank_operator && condition.value.is_some() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "is_blank and is_not_blank conditions must omit value".to_string(),
        });
    }
    if !blank_operator && condition.value.is_none() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("{:?} condition requires value", condition.operator),
        });
    }
    if matches!(
        condition.operator,
        SpreadsheetFilterOperator::GreaterThan
            | SpreadsheetFilterOperator::GreaterThanOrEqual
            | SpreadsheetFilterOperator::LessThan
            | SpreadsheetFilterOperator::LessThanOrEqual
    ) && !matches!(
        condition.value,
        Some(SpreadsheetFilterValue::Integer(_) | SpreadsheetFilterValue::Number(_))
    ) {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "numeric comparison conditions require an integer or number value".to_string(),
        });
    }
    if let Some(SpreadsheetFilterValue::Number(value)) = condition.value {
        if !value.is_finite() {
            return Err(SpreadsheetError::InvalidFilter {
                reason: "filter number must be finite".to_string(),
            });
        }
    }
    Ok(())
}

fn filter_condition_matches(
    cell: &SpreadsheetCell,
    condition: &SpreadsheetFilterCondition,
) -> bool {
    match condition.operator {
        SpreadsheetFilterOperator::IsBlank => {
            matches!(cell.value, SpreadsheetCellValue::Empty)
                || spreadsheet_cell_display_text(&cell.value).is_empty()
        }
        SpreadsheetFilterOperator::IsNotBlank => {
            !matches!(cell.value, SpreadsheetCellValue::Empty)
                && !spreadsheet_cell_display_text(&cell.value).is_empty()
        }
        SpreadsheetFilterOperator::Equals => condition
            .value
            .as_ref()
            .is_some_and(|value| filter_values_equal(&cell.value, value, condition.case_sensitive)),
        SpreadsheetFilterOperator::NotEquals => condition.value.as_ref().is_some_and(|value| {
            !filter_values_equal(&cell.value, value, condition.case_sensitive)
        }),
        SpreadsheetFilterOperator::Contains
        | SpreadsheetFilterOperator::StartsWith
        | SpreadsheetFilterOperator::EndsWith => {
            let Some(value) = condition.value.as_ref() else {
                return false;
            };
            let mode = match condition.operator {
                SpreadsheetFilterOperator::Contains => SpreadsheetTextMatchMode::Contains,
                SpreadsheetFilterOperator::StartsWith => SpreadsheetTextMatchMode::StartsWith,
                SpreadsheetFilterOperator::EndsWith => SpreadsheetTextMatchMode::EndsWith,
                _ => unreachable!(),
            };
            text_matches(
                &spreadsheet_cell_display_text(&cell.value),
                &filter_value_display_text(value),
                mode,
                condition.case_sensitive,
            )
        }
        SpreadsheetFilterOperator::GreaterThan
        | SpreadsheetFilterOperator::GreaterThanOrEqual
        | SpreadsheetFilterOperator::LessThan
        | SpreadsheetFilterOperator::LessThanOrEqual => {
            let Some(left) = spreadsheet_cell_number(&cell.value) else {
                return false;
            };
            let Some(right) = condition.value.as_ref().and_then(filter_value_number) else {
                return false;
            };
            match condition.operator {
                SpreadsheetFilterOperator::GreaterThan => left > right,
                SpreadsheetFilterOperator::GreaterThanOrEqual => left >= right,
                SpreadsheetFilterOperator::LessThan => left < right,
                SpreadsheetFilterOperator::LessThanOrEqual => left <= right,
                _ => unreachable!(),
            }
        }
    }
}

fn filter_values_equal(
    cell: &SpreadsheetCellValue,
    expected: &SpreadsheetFilterValue,
    case_sensitive: bool,
) -> bool {
    match expected {
        SpreadsheetFilterValue::String(expected) => text_matches(
            &spreadsheet_cell_display_text(cell),
            expected,
            SpreadsheetTextMatchMode::Exact,
            case_sensitive,
        ),
        SpreadsheetFilterValue::Integer(expected) => {
            spreadsheet_cell_number(cell) == Some(*expected as f64)
        }
        SpreadsheetFilterValue::Number(expected) => {
            spreadsheet_cell_number(cell) == Some(*expected)
        }
        SpreadsheetFilterValue::Boolean(expected) => {
            matches!(cell, SpreadsheetCellValue::Boolean(actual) if actual == expected)
        }
    }
}

fn spreadsheet_cell_number(value: &SpreadsheetCellValue) -> Option<f64> {
    match value {
        SpreadsheetCellValue::Integer(value) => Some(*value as f64),
        SpreadsheetCellValue::Number(value) => Some(*value),
        SpreadsheetCellValue::DateTime(value) => Some(value.serial),
        _ => None,
    }
}

fn filter_value_number(value: &SpreadsheetFilterValue) -> Option<f64> {
    match value {
        SpreadsheetFilterValue::Integer(value) => Some(*value as f64),
        SpreadsheetFilterValue::Number(value) => Some(*value),
        _ => None,
    }
}

fn filter_value_display_text(value: &SpreadsheetFilterValue) -> String {
    match value {
        SpreadsheetFilterValue::String(value) => value.clone(),
        SpreadsheetFilterValue::Integer(value) => value.to_string(),
        SpreadsheetFilterValue::Number(value) => value.to_string(),
        SpreadsheetFilterValue::Boolean(value) => value.to_string(),
    }
}

pub fn write_workbook_preferred(
    request: &WriteWorkbookRequest,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    let preference = env::var("OPENTOPIA_SPREADSHEET_BACKEND")
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase();
    if preference == "native" || (preference == "auto" && native_template_patch_applies(request)?) {
        return write_workbook(request);
    }

    let python = discover_openpyxl_python();
    match (preference.as_str(), python) {
        ("auto", Some(python)) => write_workbook_openpyxl(request, &python)
            .or_else(|_| write_workbook(request)),
        ("auto", None) => write_workbook(request),
        ("openpyxl", Some(python)) => write_workbook_openpyxl(request, &python),
        ("openpyxl", None) => Err(SpreadsheetError::BackendUnavailable {
            message: "OPENTOPIA_SPREADSHEET_BACKEND=openpyxl, but no Python executable with openpyxl was found; set OPENTOPIA_SPREADSHEET_PYTHON".to_string(),
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

fn discover_openpyxl_python() -> Option<PathBuf> {
    DISCOVERED_OPENPYXL_PYTHON
        .get_or_init(discover_openpyxl_python_uncached)
        .clone()
}

fn discover_openpyxl_python_uncached() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(configured) = env::var_os("OPENTOPIA_SPREADSHEET_PYTHON") {
        let configured = PathBuf::from(configured);
        if !configured.as_os_str().is_empty() {
            candidates.push(configured);
        }
    }
    candidates.extend([PathBuf::from("python3"), PathBuf::from("python")]);
    candidates.into_iter().find(python_has_openpyxl)
}

fn python_has_openpyxl(candidate: &PathBuf) -> bool {
    let Ok(mut child) = Command::new(candidate)
        .args(["-I", "-c", "import openpyxl"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    match child.wait_timeout(OPENPYXL_PROBE_TIMEOUT) {
        Ok(Some(status)) => status.success(),
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            false
        }
        Err(_) => false,
    }
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
        .map_err(|error| write_failed(&request.output, error))?;

    let status = match child
        .wait_timeout(OPENPYXL_WORKER_TIMEOUT)
        .map_err(|error| write_failed(&request.output, error))?
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
    if let Some(mut output) = child.stdout.take() {
        output
            .read_to_string(&mut stdout)
            .map_err(|error| write_failed(&request.output, error))?;
    }
    if let Some(mut output) = child.stderr.take() {
        output
            .read_to_string(&mut stderr)
            .map_err(|error| write_failed(&request.output, error))?;
    }
    if !status.success() {
        let message = stderr.trim();
        return Err(write_failed(
            &request.output,
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
            &request.output,
            "openpyxl worker did not report success",
        ));
    }

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
    ensure_return_size(&result)?;
    Ok(result)
}

pub fn write_workbook(
    request: &WriteWorkbookRequest,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    validate_xlsx_path(&request.output)?;
    let applied_updates = validate_write_request(request)?;
    let mut loaded = match &request.source {
        Some(source) => load_workbook(source)?,
        None => LoadedWorkbook::default(),
    };
    let preserve_template_parts = request.source.is_some()
        && request.sheets.iter().all(|sheet| {
            sheet.visibility.is_none()
                && loaded
                    .sheets
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&sheet.name))
        });

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
    ensure_return_size(&result)?;

    fs::write(&request.output, &bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    Ok(result)
}

fn open_xlsx(path: &Path) -> Result<(Xlsx<BufReader<File>>, u64), SpreadsheetError> {
    validate_xlsx_path(path)?;
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
    let file = File::open(path).map_err(|source| SpreadsheetError::Io {
        operation: "open",
        path: path.to_path_buf(),
        source,
    })?;
    let workbook =
        Xlsx::new(BufReader::new(file)).map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    Ok((workbook, file_size_bytes))
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

fn validate_write_request(request: &WriteWorkbookRequest) -> Result<usize, SpreadsheetError> {
    if let Some(source) = &request.source {
        validate_xlsx_path(source)?;
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
    workbook: &mut Xlsx<BufReader<File>>,
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
    workbook: &mut Xlsx<BufReader<File>>,
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

fn ensure_return_size<T: Serialize>(result: &T) -> Result<(), SpreadsheetError> {
    let actual_bytes = serde_json::to_vec(result)
        .map_err(|error| SpreadsheetError::Serialization {
            message: error.to_string(),
        })?
        .len();
    if actual_bytes > MAX_RETURN_BYTES {
        return Err(SpreadsheetError::ReturnTooLarge {
            actual_bytes,
            limit_bytes: MAX_RETURN_BYTES,
        });
    }
    Ok(())
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

#[derive(Debug, Default)]
struct LoadedWorkbook {
    sheets: Vec<LoadedSheet>,
}

#[derive(Debug)]
struct LoadedSheet {
    name: String,
    visibility: SheetVisibility,
    cells: BTreeMap<CellAddress, StoredCell>,
}

#[derive(Debug)]
struct StoredCell {
    value: SpreadsheetCellValue,
    formula: Option<String>,
    formula_result: Option<String>,
}

fn load_workbook(path: &Path) -> Result<LoadedWorkbook, SpreadsheetError> {
    let (mut workbook, _) = open_xlsx(path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    let mut loaded = LoadedWorkbook {
        sheets: Vec::with_capacity(metadata.len()),
    };
    let mut workbook_cells = 0usize;

    for metadata in metadata {
        let info = sheet_info(&metadata);
        if info.kind != SheetKind::Worksheet {
            return Err(SpreadsheetError::UnsupportedSheetType {
                sheet: info.name,
                kind: info.kind,
            });
        }
        let values = worksheet_values(&mut workbook, path, &info.name)?;
        let formulas = worksheet_formulas(&mut workbook, path, &info.name)?;
        let mut cells = BTreeMap::new();
        load_values(&values, &info.name, &mut cells)?;
        load_formulas(&formulas, &info.name, &mut cells)?;
        workbook_cells =
            workbook_cells
                .checked_add(cells.len())
                .ok_or(SpreadsheetError::TooManyCells {
                    context: "workbook",
                    actual: usize::MAX,
                    limit: MAX_WORKBOOK_CELLS,
                })?;
        ensure_workbook_cell_count(workbook_cells)?;
        loaded.sheets.push(LoadedSheet {
            name: info.name,
            visibility: info.visibility,
            cells,
        });
    }
    Ok(loaded)
}

fn load_values(
    values: &Range<Data>,
    sheet: &str,
    cells: &mut BTreeMap<CellAddress, StoredCell>,
) -> Result<(), SpreadsheetError> {
    let Some((base_row, base_column)) = values.start() else {
        return Ok(());
    };
    for (relative_row, relative_column, value) in values.used_cells() {
        let address =
            absolute_address(base_row, base_column, relative_row, relative_column, sheet)?;
        let value = cell_value_from_data(value, sheet, address.row, address.column)?;
        cells.insert(
            address,
            StoredCell {
                formula_result: formula_result_from_value(&value),
                value,
                formula: None,
            },
        );
        ensure_workbook_cell_count(cells.len())?;
    }
    Ok(())
}

fn load_formulas(
    formulas: &Range<String>,
    sheet: &str,
    cells: &mut BTreeMap<CellAddress, StoredCell>,
) -> Result<(), SpreadsheetError> {
    let Some((base_row, base_column)) = formulas.start() else {
        return Ok(());
    };
    for (relative_row, relative_column, formula) in formulas.used_cells() {
        let address =
            absolute_address(base_row, base_column, relative_row, relative_column, sheet)?;
        validate_source_formula(formula, sheet, address)?;
        let cell = cells.entry(address).or_insert_with(|| StoredCell {
            value: SpreadsheetCellValue::Empty,
            formula: None,
            formula_result: None,
        });
        cell.formula = Some(formula.clone());
        ensure_workbook_cell_count(cells.len())?;
    }
    Ok(())
}

fn absolute_address(
    base_row: u32,
    base_column: u32,
    relative_row: usize,
    relative_column: usize,
    sheet: &str,
) -> Result<CellAddress, SpreadsheetError> {
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
    let address = CellAddress { row, column };
    validate_address(address)?;
    Ok(address)
}

fn validate_source_formula(
    formula: &str,
    sheet: &str,
    address: CellAddress,
) -> Result<(), SpreadsheetError> {
    if formula.len() > MAX_FORMULA_BYTES {
        return Err(invalid_cell_value(
            sheet,
            address.row,
            address.column,
            format!("source formula exceeds {MAX_FORMULA_BYTES} bytes"),
        ));
    }
    if contains_invalid_xml_character(formula) {
        return Err(invalid_cell_value(
            sheet,
            address.row,
            address.column,
            "source formula contains an unsupported control character",
        ));
    }
    Ok(())
}

fn apply_sheet_updates(
    workbook: &mut LoadedWorkbook,
    requests: &[SheetWriteRequest],
) -> Result<(), SpreadsheetError> {
    for request in requests {
        let index = workbook
            .sheets
            .iter()
            .position(|sheet| sheet.name.to_lowercase() == request.name.to_lowercase());
        let index = match index {
            Some(index) => index,
            None => {
                workbook.sheets.push(LoadedSheet {
                    name: request.name.clone(),
                    visibility: request.visibility.unwrap_or_default(),
                    cells: BTreeMap::new(),
                });
                workbook.sheets.len() - 1
            }
        };
        let sheet = &mut workbook.sheets[index];
        if let Some(visibility) = request.visibility {
            sheet.visibility = visibility;
        }
        for update in &request.cells {
            match &update.value {
                SpreadsheetCellInput::Blank => {
                    sheet.cells.remove(&update.address);
                }
                SpreadsheetCellInput::String(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::String(value.clone()),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Integer(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Integer(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Number(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Number(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Boolean(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Boolean(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Formula(formula) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Empty,
                            formula: Some(formula.expression.clone()),
                            formula_result: formula.cached_result.clone(),
                        },
                    );
                }
            }
        }
    }
    Ok(())
}

fn patch_workbook_template(
    source: &Path,
    sheets: &[SheetWriteRequest],
    output: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let source_bytes = fs::read(source).map_err(|source_error| SpreadsheetError::Io {
        operation: "read",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut archive = ZipArchive::new(Cursor::new(source_bytes)).map_err(|error| {
        SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("invalid XLSX package: {error}"),
        }
    })?;
    let workbook_xml = read_zip_part(&mut archive, "xl/workbook.xml", source)?;
    let relationships_xml = read_zip_part(&mut archive, "xl/_rels/workbook.xml.rels", source)?;
    let sheet_relationships = workbook_sheet_relationships(&workbook_xml, source)?;
    let relationship_targets = workbook_relationship_targets(&relationships_xml, source)?;
    let mut updates_by_part = BTreeMap::<String, Vec<CellUpdate>>::new();
    for sheet in sheets {
        let relationship_id = sheet_relationships
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&sheet.name))
            .map(|(_, id)| id)
            .ok_or_else(|| SpreadsheetError::SheetNotFound {
                sheet: sheet.name.clone(),
            })?;
        let target = relationship_targets.get(relationship_id).ok_or_else(|| {
            SpreadsheetError::InvalidWorkbook {
                path: source.to_path_buf(),
                message: format!(
                    "worksheet relationship {relationship_id:?} for sheet {:?} was not found",
                    sheet.name
                ),
            }
        })?;
        updates_by_part.insert(normalize_workbook_part_target(target), sheet.cells.clone());
    }

    let mut output_cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output_cursor);
        for index in 0..archive.len() {
            let (name, is_directory, compression, unix_mode, mut bytes) = {
                let mut file =
                    archive
                        .by_index(index)
                        .map_err(|error| SpreadsheetError::InvalidWorkbook {
                            path: source.to_path_buf(),
                            message: format!("failed to read XLSX package entry {index}: {error}"),
                        })?;
                let name = file.name().to_string();
                let is_directory = file.is_dir();
                let compression = file.compression();
                let unix_mode = file.unix_mode();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|source_error| SpreadsheetError::Io {
                        operation: "read",
                        path: source.to_path_buf(),
                        source: source_error,
                    })?;
                (name, is_directory, compression, unix_mode, bytes)
            };
            if let Some(updates) = updates_by_part.get(&name) {
                bytes = patch_worksheet_xml(&bytes, updates, source)?;
            }
            let mut options = SimpleFileOptions::default().compression_method(compression);
            if let Some(mode) = unix_mode {
                options = options.unix_permissions(mode);
            }
            if is_directory {
                writer
                    .add_directory(&name, options)
                    .map_err(|error| write_failed(output, error))?;
            } else {
                writer
                    .start_file(&name, options)
                    .map_err(|error| write_failed(output, error))?;
                writer
                    .write_all(&bytes)
                    .map_err(|error| write_failed(output, error))?;
            }
        }
        writer
            .finish()
            .map_err(|error| write_failed(output, error))?;
    }
    Ok(output_cursor.into_inner())
}

fn read_zip_part(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut part = archive
        .by_name(name)
        .map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("missing XLSX part {name}: {error}"),
        })?;
    let mut bytes = Vec::new();
    part.read_to_end(&mut bytes)
        .map_err(|source_error| SpreadsheetError::Io {
            operation: "read",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    Ok(bytes)
}

fn workbook_sheet_relationships(
    xml: &[u8],
    source: &Path,
) -> Result<Vec<(String, String)>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut relationships = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "workbook.xml", error))?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"sheet" =>
            {
                let name = xml_attribute(&reader, &element, b"name")?;
                let relationship_id = xml_attribute(&reader, &element, b"id")?;
                if let (Some(name), Some(relationship_id)) = (name, relationship_id) {
                    relationships.push((name, relationship_id));
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(relationships)
}

fn workbook_relationship_targets(
    xml: &[u8],
    source: &Path,
) -> Result<BTreeMap<String, String>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut targets = BTreeMap::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "workbook.xml.rels", error))?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"Relationship" =>
            {
                let id = xml_attribute(&reader, &element, b"Id")?;
                let target = xml_attribute(&reader, &element, b"Target")?;
                if let (Some(id), Some(target)) = (id, target) {
                    targets.insert(id, target);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(targets)
}

fn xml_attribute(
    reader: &XmlReader<&[u8]>,
    element: &BytesStart<'_>,
    name: &[u8],
) -> Result<Option<String>, SpreadsheetError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: PathBuf::from("<xlsx-xml>"),
            message: error.to_string(),
        })?;
        if attribute.key.local_name().as_ref() != name {
            continue;
        }
        return attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map(|value| Some(value.into_owned()))
            .map_err(|error| SpreadsheetError::InvalidWorkbook {
                path: PathBuf::from("<xlsx-xml>"),
                message: error.to_string(),
            });
    }
    Ok(None)
}

fn normalize_workbook_part_target(target: &str) -> String {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{}", target.trim_start_matches("../"))
    }
}

fn patch_worksheet_xml(
    xml: &[u8],
    updates: &[CellUpdate],
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut pending = BTreeMap::<u32, BTreeMap<u32, SpreadsheetCellInput>>::new();
    for update in updates {
        pending
            .entry(update.address.row)
            .or_default()
            .insert(update.address.column, update.value.clone());
    }
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = XmlWriter::new(Vec::with_capacity(xml.len()));
    let mut found_sheet_data = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                patch_sheet_data(&mut reader, &mut writer, &mut pending, source)?;
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(BytesStart::new("sheetData")))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                write_remaining_rows(&mut writer, &mut pending, source)?;
                writer
                    .write_event(XmlEvent::End(BytesEnd::new("sheetData")))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
            }
            XmlEvent::Eof => break,
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "worksheet", error))?,
        }
    }
    if !found_sheet_data {
        return Err(SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: "worksheet is missing sheetData".to_string(),
        });
    }
    Ok(writer.into_inner())
}

fn patch_sheet_data(
    reader: &mut XmlReader<&[u8]>,
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "sheetData", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"row" => {
                let row_number = xml_attribute(reader, &element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| SpreadsheetError::InvalidWorkbook {
                        path: source.to_path_buf(),
                        message: "worksheet row is missing a valid one-based r attribute"
                            .to_string(),
                    })?;
                write_rows_before(writer, pending, row_number, source)?;
                let row = collect_xml_element(reader, element, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    let patched = patch_row_xml(&row, row_number, &updates, source)?;
                    writer.get_mut().extend_from_slice(&patched);
                } else {
                    writer.get_mut().extend_from_slice(&row);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"row" => {
                let row_number = xml_attribute(reader, &element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| SpreadsheetError::InvalidWorkbook {
                        path: source.to_path_buf(),
                        message: "worksheet row is missing a valid one-based r attribute"
                            .to_string(),
                    })?;
                write_rows_before(writer, pending, row_number, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    write_generated_row(writer, row_number, &updates, source)?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "row", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"sheetData" => {
                write_remaining_rows(writer, pending, source)?;
                writer
                    .write_event(XmlEvent::End(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "sheetData", error))?;
                return Ok(());
            }
            XmlEvent::Eof => {
                return Err(SpreadsheetError::InvalidWorkbook {
                    path: source.to_path_buf(),
                    message: "worksheet sheetData ended unexpectedly".to_string(),
                });
            }
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "sheetData", error))?,
        }
    }
}

fn collect_xml_element(
    reader: &mut XmlReader<&[u8]>,
    start: BytesStart<'_>,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut writer = XmlWriter::new(Vec::new());
    writer
        .write_event(XmlEvent::Start(start.into_owned()))
        .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
    let mut depth = 1usize;
    while depth > 0 {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
        match &event {
            XmlEvent::Start(_) => depth += 1,
            XmlEvent::End(_) => depth -= 1,
            XmlEvent::Eof => {
                return Err(SpreadsheetError::InvalidWorkbook {
                    path: source.to_path_buf(),
                    message: "worksheet element ended unexpectedly".to_string(),
                });
            }
            _ => {}
        }
        writer
            .write_event(event.into_owned())
            .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
    }
    Ok(writer.into_inner())
}

fn patch_row_xml(
    row_xml: &[u8],
    row: u32,
    updates: &BTreeMap<u32, SpreadsheetCellInput>,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut pending = updates.clone();
    let mut reader = XmlReader::from_reader(row_xml);
    reader.config_mut().trim_text(false);
    let mut writer = XmlWriter::new(Vec::with_capacity(row_xml.len()));
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "row", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"c" => {
                let column = cell_column_from_element(&reader, &element, source)?;
                write_cells_before(&mut writer, row, &mut pending, column, source)?;
                let style = xml_attribute(&reader, &element, b"s")?;
                let original = collect_xml_element(&mut reader, element, source)?;
                if let Some(value) = pending.remove(&column) {
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref(),
                        source,
                    )?;
                } else {
                    writer.get_mut().extend_from_slice(&original);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"c" => {
                let column = cell_column_from_element(&reader, &element, source)?;
                write_cells_before(&mut writer, row, &mut pending, column, source)?;
                if let Some(value) = pending.remove(&column) {
                    let style = xml_attribute(&reader, &element, b"s")?;
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref(),
                        source,
                    )?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "cell", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"row" => {
                write_remaining_cells(&mut writer, row, &mut pending, source)?;
                writer
                    .write_event(XmlEvent::End(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "row", error))?;
            }
            XmlEvent::Eof => break,
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "row", error))?,
        }
    }
    Ok(writer.into_inner())
}

fn cell_column_from_element(
    reader: &XmlReader<&[u8]>,
    element: &BytesStart<'_>,
    source: &Path,
) -> Result<u32, SpreadsheetError> {
    let reference =
        xml_attribute(reader, element, b"r")?.ok_or_else(|| SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: "worksheet cell is missing r attribute".to_string(),
        })?;
    let mut column = 0u32;
    let mut letters = 0usize;
    for byte in reference.bytes() {
        if !byte.is_ascii_alphabetic() {
            break;
        }
        letters += 1;
        column = column
            .checked_mul(26)
            .and_then(|value| value.checked_add(u32::from(byte.to_ascii_uppercase() - b'A') + 1))
            .ok_or_else(|| invalid_workbook_coordinate("worksheet"))?;
    }
    if letters == 0 || column == 0 {
        return Err(SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("invalid worksheet cell reference {reference:?}"),
        });
    }
    Ok(column - 1)
}

fn write_rows_before(
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    before: u32,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = pending
        .range(..before)
        .map(|(row, _)| *row)
        .collect::<Vec<_>>();
    for row in rows {
        if let Some(updates) = pending.remove(&row) {
            write_generated_row(writer, row, &updates, source)?;
        }
    }
    Ok(())
}

fn write_remaining_rows(
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = std::mem::take(pending);
    for (row, updates) in rows {
        write_generated_row(writer, row, &updates, source)?;
    }
    Ok(())
}

fn write_generated_row(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    updates: &BTreeMap<u32, SpreadsheetCellInput>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let mut element = BytesStart::new("row");
    let row_number = row.saturating_add(1).to_string();
    element.push_attribute(("r", row_number.as_str()));
    writer
        .write_event(XmlEvent::Start(element))
        .map_err(|error| invalid_template_xml(source, "row", error))?;
    for (column, value) in updates {
        write_generated_cell(writer, row, *column, value, None, source)?;
    }
    writer
        .write_event(XmlEvent::End(BytesEnd::new("row")))
        .map_err(|error| invalid_template_xml(source, "row", error))?;
    Ok(())
}

fn write_cells_before(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    pending: &mut BTreeMap<u32, SpreadsheetCellInput>,
    before: u32,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let columns = pending
        .range(..before)
        .map(|(column, _)| *column)
        .collect::<Vec<_>>();
    for column in columns {
        if let Some(value) = pending.remove(&column) {
            write_generated_cell(writer, row, column, &value, None, source)?;
        }
    }
    Ok(())
}

fn write_remaining_cells(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    pending: &mut BTreeMap<u32, SpreadsheetCellInput>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let cells = std::mem::take(pending);
    for (column, value) in cells {
        write_generated_cell(writer, row, column, &value, None, source)?;
    }
    Ok(())
}

fn write_generated_cell(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    column: u32,
    value: &SpreadsheetCellInput,
    style: Option<&str>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let reference = cell_reference(row, column);
    let mut cell = BytesStart::new("c");
    cell.push_attribute(("r", reference.as_str()));
    if let Some(style) = style {
        cell.push_attribute(("s", style));
    }
    match value {
        SpreadsheetCellInput::Blank => {
            if style.is_some() {
                writer
                    .write_event(XmlEvent::Empty(cell))
                    .map_err(|error| invalid_template_xml(source, "cell", error))?;
            }
        }
        SpreadsheetCellInput::String(value) => {
            cell.push_attribute(("t", "inlineStr"));
            writer
                .write_event(XmlEvent::Start(cell))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Start(BytesStart::new("is")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            let mut text = BytesStart::new("t");
            text.push_attribute(("xml:space", "preserve"));
            writer
                .write_event(XmlEvent::Start(text))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Text(BytesText::new(value)))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::End(BytesEnd::new("t")))
                .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("is"))))
                .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("c"))))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
        }
        SpreadsheetCellInput::Integer(value) => {
            write_scalar_cell(writer, cell, &value.to_string(), source)?;
        }
        SpreadsheetCellInput::Number(value) => {
            write_scalar_cell(writer, cell, &value.to_string(), source)?;
        }
        SpreadsheetCellInput::Boolean(value) => {
            cell.push_attribute(("t", "b"));
            write_scalar_cell(writer, cell, if *value { "1" } else { "0" }, source)?;
        }
        SpreadsheetCellInput::Formula(formula) => {
            writer
                .write_event(XmlEvent::Start(cell))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Start(BytesStart::new("f")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Text(BytesText::new(
                    formula.expression.trim_start_matches('='),
                )))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::End(BytesEnd::new("f")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            if let Some(result) = &formula.cached_result {
                writer
                    .write_event(XmlEvent::Start(BytesStart::new("v")))
                    .and_then(|_| writer.write_event(XmlEvent::Text(BytesText::new(result))))
                    .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("v"))))
                    .map_err(|error| invalid_template_xml(source, "cell", error))?;
            }
            writer
                .write_event(XmlEvent::End(BytesEnd::new("c")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
        }
    }
    Ok(())
}

fn write_scalar_cell(
    writer: &mut XmlWriter<Vec<u8>>,
    cell: BytesStart<'_>,
    value: &str,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    writer
        .write_event(XmlEvent::Start(cell.into_owned()))
        .and_then(|_| writer.write_event(XmlEvent::Start(BytesStart::new("v"))))
        .and_then(|_| writer.write_event(XmlEvent::Text(BytesText::new(value))))
        .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("v"))))
        .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("c"))))
        .map_err(|error| invalid_template_xml(source, "cell", error))?;
    Ok(())
}

fn cell_reference(row: u32, column: u32) -> String {
    let mut value = column.saturating_add(1);
    let mut letters = Vec::new();
    while value > 0 {
        let remainder = (value - 1) % 26;
        letters.push((b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        letters.into_iter().collect::<String>(),
        row.saturating_add(1)
    )
}

fn invalid_template_xml(
    source: &Path,
    part: &str,
    error: impl std::fmt::Display,
) -> SpreadsheetError {
    SpreadsheetError::InvalidWorkbook {
        path: source.to_path_buf(),
        message: format!("invalid {part} XML: {error}"),
    }
}

fn render_workbook(loaded: &LoadedWorkbook, output: &Path) -> Result<Vec<u8>, SpreadsheetError> {
    let mut workbook = Workbook::new();
    for sheet in &loaded.sheets {
        let worksheet = workbook.add_worksheet();
        worksheet
            .set_name(&sheet.name)
            .map_err(|error| write_failed(output, error))?;
        match sheet.visibility {
            SheetVisibility::Visible => {}
            SheetVisibility::Hidden => {
                worksheet.set_hidden(true);
            }
            SheetVisibility::VeryHidden => {
                worksheet.set_very_hidden(true);
            }
        }
        for (address, cell) in &sheet.cells {
            write_stored_cell(worksheet, *address, cell, output)?;
        }
    }
    workbook
        .save_to_buffer()
        .map_err(|error| write_failed(output, error))
}

fn write_stored_cell(
    worksheet: &mut Worksheet,
    address: CellAddress,
    cell: &StoredCell,
    output: &Path,
) -> Result<(), SpreadsheetError> {
    let row = address.row;
    let column = address.column as u16;
    if let Some(expression) = &cell.formula {
        let mut formula = Formula::new(expression);
        if let Some(result) = cell
            .formula_result
            .clone()
            .or_else(|| formula_result_from_value(&cell.value))
        {
            formula = formula.set_result(result);
        }
        worksheet
            .write_formula(row, column, formula)
            .map_err(|error| write_failed(output, error))?;
        return Ok(());
    }

    match &cell.value {
        SpreadsheetCellValue::Empty => {}
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => {
            worksheet
                .write_string(row, column, value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Integer(value) => {
            worksheet
                .write(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Number(value) => {
            worksheet
                .write_number(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Boolean(value) => {
            worksheet
                .write_boolean(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::DateTime(value) => {
            worksheet
                .write_number(row, column, value.serial)
                .map_err(|error| write_failed(output, error))?;
        }
    }
    Ok(())
}

fn formula_result_from_value(value: &SpreadsheetCellValue) -> Option<String> {
    match value {
        SpreadsheetCellValue::Empty => None,
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => Some(value.clone()),
        SpreadsheetCellValue::Integer(value) => Some(value.to_string()),
        SpreadsheetCellValue::Number(value) => Some(value.to_string()),
        SpreadsheetCellValue::Boolean(value) => {
            Some(if *value { "TRUE" } else { "FALSE" }.to_string())
        }
        SpreadsheetCellValue::DateTime(value) => Some(value.serial.to_string()),
    }
}

fn write_failed(output: &Path, error: impl std::fmt::Display) -> SpreadsheetError {
    SpreadsheetError::WriteFailed {
        path: output.to_path_buf(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::{Color, Format};
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = format!(
                "opentopia-spreadsheet-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system time")
                    .as_nanos(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(unique);
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

    fn address(row: u32, column: u32) -> CellAddress {
        CellAddress { row, column }
    }

    fn range(start: (u32, u32), end: (u32, u32)) -> CellRange {
        CellRange {
            start: address(start.0, start.1),
            end: address(end.0, end.1),
        }
    }

    fn update(row: u32, column: u32, value: SpreadsheetCellInput) -> CellUpdate {
        CellUpdate {
            address: address(row, column),
            value,
        }
    }

    fn zip_part(path: &Path, name: &str) -> Vec<u8> {
        let bytes = fs::read(path).expect("read XLSX package");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open XLSX package");
        let mut part = archive.by_name(name).expect("open XLSX part");
        let mut contents = Vec::new();
        part.read_to_end(&mut contents).expect("read XLSX part");
        contents
    }

    #[test]
    fn create_inspect_read_and_update_roundtrip() {
        let directory = TestDirectory::new();
        let original = directory.path("original.xlsx");
        let updated = directory.path("updated.xlsx");

        let created = write_workbook(&WriteWorkbookRequest {
            source: None,
            output: original.clone(),
            sheets: vec![
                SheetWriteRequest {
                    name: "Data".to_string(),
                    visibility: None,
                    cells: vec![
                        update(0, 0, SpreadsheetCellInput::String("label".to_string())),
                        update(1, 0, SpreadsheetCellInput::Integer(42)),
                        update(
                            1,
                            1,
                            SpreadsheetCellInput::Formula(FormulaInput {
                                expression: "A2*2".to_string(),
                                cached_result: Some("84".to_string()),
                            }),
                        ),
                    ],
                },
                SheetWriteRequest {
                    name: "Archive".to_string(),
                    visibility: Some(SheetVisibility::Hidden),
                    cells: vec![],
                },
            ],
        })
        .expect("create workbook");
        assert_eq!(created.sheet_count, 2);
        assert_eq!(created.output_cells, 3);

        let listed = list_sheets(&ListSheetsRequest {
            path: original.clone(),
        })
        .expect("list sheets");
        assert_eq!(listed.sheets.len(), 2);
        assert_eq!(listed.sheets[1].visibility, SheetVisibility::Hidden);

        let inspected = inspect_workbook(&InspectWorkbookRequest {
            path: original.clone(),
        })
        .expect("inspect workbook");
        assert_eq!(inspected.populated_cells, 3);
        assert_eq!(inspected.sheets[0].used_range, Some(range((0, 0), (1, 1))));

        let read = read_range(&ReadRangeRequest {
            path: original.clone(),
            sheet: "Data".to_string(),
            range: range((0, 0), (1, 1)),
        })
        .expect("read range");
        assert_eq!(
            read.rows[0][0].value,
            SpreadsheetCellValue::String("label".to_string())
        );
        assert_eq!(read.rows[1][0].value, SpreadsheetCellValue::Number(42.0));
        assert!(read.rows[1][1]
            .formula
            .as_deref()
            .is_some_and(|formula| formula.contains("A2*2")));

        write_workbook(&WriteWorkbookRequest {
            source: Some(original),
            output: updated.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Data".to_string(),
                visibility: None,
                cells: vec![
                    update(1, 0, SpreadsheetCellInput::Integer(43)),
                    update(0, 2, SpreadsheetCellInput::Boolean(true)),
                ],
            }],
        })
        .expect("update workbook");

        let read = read_range(&ReadRangeRequest {
            path: updated,
            sheet: "Data".to_string(),
            range: range((0, 0), (1, 2)),
        })
        .expect("read updated range");
        assert_eq!(read.rows[1][0].value, SpreadsheetCellValue::Number(43.0));
        assert_eq!(read.rows[0][2].value, SpreadsheetCellValue::Boolean(true));
        assert!(read.rows[1][1].formula.is_some());
    }

    #[test]
    fn template_patch_preserves_styles_and_non_worksheet_parts() {
        let directory = TestDirectory::new();
        let template = directory.path("styled-template.xlsx");
        let output = directory.path("styled-output.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Orders").expect("set sheet name");
        let format = Format::new().set_bold().set_background_color(Color::Yellow);
        worksheet
            .write_with_format(0, 0, "old", &format)
            .expect("write formatted template cell");
        worksheet
            .set_column_width(0, 28)
            .expect("set template width");
        workbook.save(&template).expect("save template");

        let styles_before = zip_part(&template, "xl/styles.xml");
        let workbook_before = zip_part(&template, "xl/workbook.xml");
        let result = write_workbook(&WriteWorkbookRequest {
            source: Some(template.clone()),
            output: output.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Orders".to_string(),
                visibility: None,
                cells: vec![
                    update(0, 0, SpreadsheetCellInput::String("new".to_string())),
                    update(10, 2, SpreadsheetCellInput::Integer(17)),
                ],
            }],
        })
        .expect("patch template");

        assert!(result.preserved_template_parts);
        assert!(!result.rebuilt_from_source);
        assert_eq!(zip_part(&output, "xl/styles.xml"), styles_before);
        assert_eq!(zip_part(&output, "xl/workbook.xml"), workbook_before);
        let worksheet_xml = String::from_utf8(zip_part(&output, "xl/worksheets/sheet1.xml"))
            .expect("worksheet XML is UTF-8");
        assert!(worksheet_xml.contains("s=\"1\""));
        assert!(worksheet_xml.contains(">new<"));
        let read = read_range(&ReadRangeRequest {
            path: output,
            sheet: "Orders".to_string(),
            range: range((0, 0), (0, 0)),
        })
        .expect("read patched template");
        assert_eq!(
            read.rows[0][0].value,
            SpreadsheetCellValue::String("new".to_string())
        );
        let appended = read_range(&ReadRangeRequest {
            path: read.path,
            sheet: "Orders".to_string(),
            range: range((10, 2), (10, 2)),
        })
        .expect("read cell appended beyond the template dimension");
        assert_eq!(
            appended.rows[0][0].value,
            SpreadsheetCellValue::Number(17.0)
        );
    }

    #[test]
    fn read_ranges_returns_multiple_bounded_regions_in_one_result() {
        let directory = TestDirectory::new();
        let workbook = directory.path("ranges.xlsx");
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Data".to_string(),
                visibility: None,
                cells: vec![
                    update(0, 0, SpreadsheetCellInput::String("header".to_string())),
                    update(10, 2, SpreadsheetCellInput::Integer(42)),
                ],
            }],
        })
        .expect("create range workbook");

        let result = read_ranges(&ReadRangesRequest {
            path: workbook,
            ranges: vec![
                SheetRangeRequest {
                    sheet: "Data".to_string(),
                    range: range((0, 0), (0, 0)),
                },
                SheetRangeRequest {
                    sheet: "Data".to_string(),
                    range: range((10, 2), (10, 2)),
                },
            ],
        })
        .expect("read multiple ranges");
        assert_eq!(result.total_cells, 2);
        assert_eq!(result.ranges.len(), 2);
        assert_eq!(
            result.ranges[1].rows[0][0].value,
            SpreadsheetCellValue::Number(42.0)
        );
    }

    #[test]
    fn find_and_filter_return_bounded_structured_rows() {
        let directory = TestDirectory::new();
        let workbook = directory.path("search-filter.xlsx");
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Orders".to_string(),
                visibility: None,
                cells: vec![
                    update(0, 0, SpreadsheetCellInput::String("Order".to_string())),
                    update(0, 1, SpreadsheetCellInput::String("Status".to_string())),
                    update(0, 2, SpreadsheetCellInput::String("Amount".to_string())),
                    update(1, 0, SpreadsheetCellInput::String("A-001".to_string())),
                    update(1, 1, SpreadsheetCellInput::String("Paid".to_string())),
                    update(1, 2, SpreadsheetCellInput::Integer(120)),
                    update(2, 0, SpreadsheetCellInput::String("A-002".to_string())),
                    update(2, 1, SpreadsheetCellInput::String("Pending".to_string())),
                    update(2, 2, SpreadsheetCellInput::Integer(80)),
                    update(3, 0, SpreadsheetCellInput::String("B-003".to_string())),
                    update(3, 1, SpreadsheetCellInput::String("paid".to_string())),
                    update(3, 2, SpreadsheetCellInput::Integer(200)),
                    update(
                        4,
                        2,
                        SpreadsheetCellInput::Formula(FormulaInput {
                            expression: "SUM(C2:C4)".to_string(),
                            cached_result: Some("400".to_string()),
                        }),
                    ),
                ],
            }],
        })
        .expect("create search/filter workbook");

        let found = find_cells(&FindCellsRequest {
            path: workbook.clone(),
            sheet: Some("Orders".to_string()),
            range: None,
            query: "paid".to_string(),
            match_mode: SpreadsheetTextMatchMode::Exact,
            case_sensitive: false,
            include_formulas: false,
            max_results: 10,
        })
        .expect("find status cells");
        assert_eq!(
            found
                .matches
                .iter()
                .map(|item| item.address)
                .collect::<Vec<_>>(),
            vec![address(1, 1), address(3, 1)]
        );
        assert!(!found.truncated);

        let formula = find_cells(&FindCellsRequest {
            path: workbook.clone(),
            sheet: Some("Orders".to_string()),
            range: None,
            query: "SUM(".to_string(),
            match_mode: SpreadsheetTextMatchMode::Contains,
            case_sensitive: true,
            include_formulas: true,
            max_results: 10,
        })
        .expect("find formula");
        assert_eq!(formula.matches.len(), 1);
        assert!(formula.matches[0].matched_formula);

        let filtered = filter_rows(&FilterRowsRequest {
            path: workbook,
            sheet: "Orders".to_string(),
            range: range((1, 0), (3, 2)),
            conditions: vec![
                SpreadsheetFilterCondition {
                    column: 1,
                    operator: SpreadsheetFilterOperator::Equals,
                    value: Some(SpreadsheetFilterValue::String("paid".to_string())),
                    case_sensitive: false,
                },
                SpreadsheetFilterCondition {
                    column: 2,
                    operator: SpreadsheetFilterOperator::GreaterThanOrEqual,
                    value: Some(SpreadsheetFilterValue::Integer(100)),
                    case_sensitive: false,
                },
            ],
            match_mode: SpreadsheetFilterMatchMode::All,
            max_results: 10,
        })
        .expect("filter order rows");
        assert_eq!(filtered.matched_row_indices, vec![1, 3]);
        assert_eq!(filtered.rows.len(), 2);
        assert!(!filtered.truncated);
    }

    #[test]
    fn openpyxl_backend_round_trips_structural_changes_when_available() {
        let Some(python) = discover_openpyxl_python() else {
            return;
        };
        let directory = TestDirectory::new();
        let template = directory.path("openpyxl-template.xlsx");
        let output = directory.path("openpyxl-output.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Orders").expect("set sheet name");
        let format = Format::new().set_bold().set_background_color(Color::Yellow);
        worksheet
            .write_with_format(0, 0, "old", &format)
            .expect("write formatted template cell");
        workbook.save(&template).expect("save template");

        let result = write_workbook_openpyxl(
            &WriteWorkbookRequest {
                source: Some(template),
                output: output.clone(),
                sheets: vec![
                    SheetWriteRequest {
                        name: "Orders".to_string(),
                        visibility: None,
                        cells: vec![update(
                            0,
                            0,
                            SpreadsheetCellInput::String("updated".to_string()),
                        )],
                    },
                    SheetWriteRequest {
                        name: "Review".to_string(),
                        visibility: None,
                        cells: vec![update(
                            0,
                            0,
                            SpreadsheetCellInput::String("ready".to_string()),
                        )],
                    },
                ],
            },
            &python,
        )
        .expect("run openpyxl backend");
        assert_eq!(result.backend, SpreadsheetWriteBackend::Openpyxl);
        assert_eq!(result.sheet_count, 2);
        assert!(result.rebuilt_from_source);
        assert!(!result.preserved_template_parts);
        let listed = list_sheets(&ListSheetsRequest {
            path: output.clone(),
        })
        .expect("list openpyxl output sheets");
        assert_eq!(listed.sheets.len(), 2);
        let read = read_range(&ReadRangeRequest {
            path: output.clone(),
            sheet: "Review".to_string(),
            range: range((0, 0), (0, 0)),
        })
        .expect("read added sheet");
        assert_eq!(
            read.rows[0][0].value,
            SpreadsheetCellValue::String("ready".to_string())
        );
        let worksheet_xml = String::from_utf8(zip_part(&output, "xl/worksheets/sheet1.xml"))
            .expect("worksheet XML is UTF-8");
        assert!(worksheet_xml.contains("s=\"1\""));
    }

    #[test]
    fn rejects_range_and_cell_limits() {
        let directory = TestDirectory::new();
        let workbook = directory.path("limits.xlsx");
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells: vec![],
            }],
        })
        .expect("create workbook");

        let error = read_range(&ReadRangeRequest {
            path: workbook,
            sheet: "Sheet1".to_string(),
            range: range((0, 0), (MAX_READ_ROWS as u32, 0)),
        })
        .expect_err("range must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::RangeTooLarge);

        let error = write_workbook(&WriteWorkbookRequest {
            source: None,
            output: directory.path("out-of-bounds.xlsx"),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells: vec![update(
                    EXCEL_MAX_ROWS,
                    0,
                    SpreadsheetCellInput::Boolean(true),
                )],
            }],
        })
        .expect_err("out-of-bounds cell must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::CellOutOfBounds);

        let cells = (0..=MAX_WRITE_UPDATES)
            .map(|row| update(row as u32, 0, SpreadsheetCellInput::Integer(1)))
            .collect();
        let error = write_workbook(&WriteWorkbookRequest {
            source: None,
            output: directory.path("too-many-updates.xlsx"),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells,
            }],
        })
        .expect_err("too many updates must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::TooManyCells);
    }

    #[test]
    fn rejects_oversized_files_and_return_content() {
        let directory = TestDirectory::new();
        let oversized = directory.path("oversized.xlsx");
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&oversized)
            .expect("create sparse file");
        file.set_len(MAX_INPUT_FILE_BYTES + 1)
            .expect("extend sparse file");
        drop(file);
        let error = list_sheets(&ListSheetsRequest { path: oversized })
            .expect_err("oversized file must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::FileTooLarge);

        let workbook = directory.path("large-return.xlsx");
        let value = "x".repeat(MAX_CELL_CHARACTERS);
        let cells = (0..40)
            .map(|row| update(row, 0, SpreadsheetCellInput::String(value.clone())))
            .collect();
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells,
            }],
        })
        .expect("create large-return workbook");
        let error = read_range(&ReadRangeRequest {
            path: workbook,
            sheet: "Sheet1".to_string(),
            range: range((0, 0), (39, 0)),
        })
        .expect_err("large return must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::ReturnTooLarge);
    }

    #[test]
    fn reports_unsupported_format_missing_sheet_and_duplicate_update() {
        let directory = TestDirectory::new();
        let xls = directory.path("legacy.xls");
        fs::write(&xls, b"not an xls file").expect("write legacy file");
        let error = list_sheets(&ListSheetsRequest { path: xls })
            .expect_err("legacy format must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::UnsupportedFormat);

        let workbook = directory.path("errors.xlsx");
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells: vec![],
            }],
        })
        .expect("create workbook");
        let error = read_range(&ReadRangeRequest {
            path: workbook,
            sheet: "Missing".to_string(),
            range: range((0, 0), (0, 0)),
        })
        .expect_err("missing sheet must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::SheetNotFound);

        let duplicate = update(0, 0, SpreadsheetCellInput::Integer(1));
        let error = write_workbook(&WriteWorkbookRequest {
            source: None,
            output: directory.path("duplicate.xlsx"),
            sheets: vec![SheetWriteRequest {
                name: "Sheet1".to_string(),
                visibility: None,
                cells: vec![duplicate.clone(), duplicate],
            }],
        })
        .expect_err("duplicate update must be rejected");
        assert_eq!(error.code(), SpreadsheetErrorCode::DuplicateCellUpdate);
        assert_eq!(error.info().code, SpreadsheetErrorCode::DuplicateCellUpdate);
    }
}
